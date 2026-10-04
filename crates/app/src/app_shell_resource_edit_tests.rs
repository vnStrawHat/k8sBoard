//! The resource edits of spec 0032b (HPA min / max, PVC Expand, Set as default storage class) in a
//! headless window over two loaded clusters, one active at a time: the fixture starts on `prod-a`
//! (locked at open), switches to `stg-b` (unlocked), and makes it live; `activate_prod` does the
//! same for `prod-a`. Each session answers from its own fake API server, so a test sees which
//! cluster a request reached and nothing leaves the machine: no test sends a real write.

use cluster::fake_api::RecordedRequest;
use cluster::{AccessCheck, AccessDecision, AccessReport, AccessReview};
use gpui_kit::InputEvent as _;
use gpui_kit::{Entity, KeyDownEvent, Keystroke, TestAppContext};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{
    Clusters, audit_lines, go_live_answering, slot_session, switch_to, writes,
};
use super::batch_write::ItemProgress;
use super::write_flow::DryRunState;
use super::*;
use crate::cluster_session::AccessState;
use crate::kind_row::KindRow;
use crate::policy_rows::horizontal_pod_autoscaler_row;
use crate::resource_actions::{ResourceAction, RowAction};
use crate::resource_edits::resource_edits_tests::{claim, class, hpa};
use crate::row_selection::BulkState;
use crate::storage_rows::{persistent_volume_claim_row, storage_class_row};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::value_popover::ValuePopover;
use crate::write_guard::{DialogConfirm, WriteLock};

const ANSWER: &str = r#"{"apiVersion":"v1","kind":"Any","metadata":{"name":"x"}}"#;
const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;

/// Accepts every patch and finds nothing else.
fn accept_patches(request: &RecordedRequest) -> (u16, String) {
    if request.method == "PATCH" {
        (200, ANSWER.to_owned())
    } else {
        (404, NOT_FOUND.to_owned())
    }
}

fn edit_clusters(name: &str, cx: &mut TestAppContext) -> Clusters {
    edit_clusters_answering(name, accept_patches, cx)
}

/// `prod-a` becomes the open cluster, live over a new fake server that accepts patches. The old
/// session is gone, so a test never has both clusters live at once.
fn activate_prod(t: &Clusters, cx: &mut TestAppContext) -> cluster::fake_api::FakeApi {
    t.activate_answering(&t.prod, "node-a", accept_patches, cx)
}

/// The open cluster, `stg-b`, answers with `respond`.
fn edit_clusters_answering(
    name: &str,
    respond: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + Clone + 'static,
    cx: &mut TestAppContext,
) -> Clusters {
    // Dialogs open without their animation, so the keys reach the fields at once.
    cx.update(|cx| cx.set_reduce_motion(true));
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let stg_api = go_live_answering(&fixture, &stg, "node-b", respond, cx);
    Clusters {
        fixture,
        stg_api,
        prod,
        stg,
    }
}

fn dispatch(t: &Clusters, action: RowAction, cx: &mut TestAppContext) {
    let action = action.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
}

fn key_down(key: &str, is_held: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(key).expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

fn texts(lines: Vec<gpui_kit::SharedString>) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

// ---- HPA min / max ----

const HPA_KIND: ResourceKind = ResourceKind::HorizontalPodAutoscalers;
const HPA_PATH: &str =
    "/apis/autoscaling/v2/namespaces/team-a/horizontalpodautoscalers/frontend-hpa";

fn hpa_rows(specs: &[(&str, u32, u32, u32)]) -> Vec<KindRow> {
    specs
        .iter()
        .map(|(name, min, max, current)| {
            horizontal_pod_autoscaler_row(&hpa(name, *min, *max, *current))
        })
        .collect()
}

/// The cursor on `frontend-hpa` (3 to 20, 9 replicas now) of staging.
fn on_stg_hpa(t: &Clusters, cx: &mut TestAppContext) {
    let rows = || hpa_rows(&[("frontend-hpa", 3, 20, 9)]);
    t.show_kind(HPA_KIND, rows(), rows(), cx);
    t.cursor_on(&t.stg, HPA_KIND, "frontend-hpa", cx);
}

fn open_range_popover(t: &Clusters, cx: &mut TestAppContext) -> Entity<ValuePopover> {
    dispatch(t, RowAction::EditHpaRange, cx);
    t.popover(cx).expect("the popover is open")
}

fn type_range(
    t: &Clusters,
    popover: &Entity<ValuePopover>,
    (min, max): (&str, &str),
    cx: &mut TestAppContext,
) {
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.type_range(min, max, window, cx));
    });
}

fn submit(t: &Clusters, popover: &Entity<ValuePopover>, cx: &mut TestAppContext) {
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.press_submit(window, cx));
    });
}

#[gpui_kit::test]
fn edit_min_max_opens_the_confirm_dialog(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range", cx);
    on_stg_hpa(&t, cx);
    let popover = open_range_popover(&t, cx);
    assert!(!t.has_dialog(cx));
    // It opens on the range the HPA has, with the replicas it runs now.
    popover.read_with(cx, |popover, cx| {
        assert_eq!(
            popover.typed_range(cx),
            Some(("3".to_owned(), "20".to_owned()))
        );
        assert_eq!(popover.state_line(cx), "Now 9 replicas");
        assert!(popover.warning_lines(cx).is_empty());
    });
    type_range(&t, &popover, ("3", "5"), cx);
    popover.read_with(cx, |popover, cx| {
        assert_eq!(
            texts(popover.warning_lines(cx)),
            ["The HPA will scale deployment/frontend down from 9 to 5"]
        );
    });
    submit(&t, &popover, cx);
    assert!(t.popover(cx).is_none(), "the popover closes on its way out");
    assert_eq!(
        t.dialog_label(cx),
        "Set replicas of hpa frontend-hpa to 3\u{2013}5"
    );
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        assert_eq!(
            texts(dialog.warning_lines()),
            ["The HPA will scale deployment/frontend down from 9 to 5"]
        );
    });
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].path, HPA_PATH);
    assert!(sent[0].has_query("dryRun", "All"));
    assert_eq!(
        sent[0].body,
        r#"{"spec":{"minReplicas":3,"maxReplicas":5}}"#
    );
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
    let sent = writes(&t.stg_api);
    assert!(!sent[1].has_query_key("dryRun"));
    assert_eq!(sent[0].body, sent[1].body);
}

#[gpui_kit::test]
fn range_form_validates_min_and_max(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-validate", cx);
    on_stg_hpa(&t, cx);
    let popover = open_range_popover(&t, cx);
    let cases = [
        (("0", "5"), Some("Min must be at least 1"), false),
        (("6", "5"), Some("Min must not exceed max"), false),
        (("", "5"), None, false),
        (("3", ""), None, false),
        (("3", "2147483648"), None, false),
        (("2", "5"), None, true),
        (("5", "5"), None, true),
        (("3", "20"), None, false),
    ];
    for ((min, max), error, is_enabled) in cases {
        type_range(&t, &popover, (min, max), cx);
        popover.read_with(cx, |popover, cx| {
            assert_eq!(popover.error_line(cx).as_deref(), error, "{min:?} {max:?}");
            assert_eq!(popover.can_submit(cx), is_enabled, "{min:?} {max:?}");
        });
    }
}

#[gpui_kit::test]
fn range_form_disabled_when_unchanged(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-unchanged", cx);
    on_stg_hpa(&t, cx);
    let popover = open_range_popover(&t, cx);
    assert!(!popover.read_with(cx, |popover, cx| popover.can_submit(cx)));
    // A press with nothing to send opens no dialog and sends nothing.
    submit(&t, &popover, cx);
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn range_form_owns_its_inputs(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-inputs", cx);
    on_stg_hpa(&t, cx);
    let popover = open_range_popover(&t, cx);
    type_range(&t, &popover, ("7", "9"), cx);
    popover.read_with(cx, |popover, cx| {
        assert_eq!(
            popover.typed_range(cx),
            Some(("7".to_owned(), "9".to_owned()))
        );
        assert_eq!(popover.title_text(), "Edit min / max of hpa/frontend-hpa");
    });
    // The Scale form of the same type has no range fields.
    t.press("escape", cx);
    t.show_kind(
        ResourceKind::Deployments,
        Vec::new(),
        vec![crate::workload_rows::deployment_row(
            &crate::workload_actions::workload_actions_tests::deployment("api"),
        )],
        cx,
    );
    t.cursor_on(&t.stg, ResourceKind::Deployments, "api", cx);
    t.press("shift-s", cx);
    let scale = t.popover(cx).expect("the Scale popover is open");
    assert_eq!(
        scale.read_with(cx, |popover, cx| popover.typed_range(cx)),
        None
    );
}

#[gpui_kit::test]
fn the_range_popover_follows_the_hpa_it_has_now(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-moved", cx);
    on_stg_hpa(&t, cx);
    let popover = open_range_popover(&t, cx);
    // The list moved on to 3-30 and 12 replicas while the form was open: 3-20 is a change now.
    let moved = hpa_rows(&[("frontend-hpa", 3, 30, 12)]);
    t.show_kind(HPA_KIND, hpa_rows(&[("frontend-hpa", 3, 20, 9)]), moved, cx);
    popover.read_with(cx, |popover, cx| {
        assert_eq!(popover.state_line(cx), "Now 12 replicas");
        assert!(popover.can_submit(cx));
    });
}

#[gpui_kit::test]
fn edit_min_max_on_a_locked_production_row_opens_no_popover(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-locked", cx);
    let prod_api = activate_prod(&t, cx);
    let rows = || hpa_rows(&[("frontend-hpa", 3, 20, 9)]);
    t.show_kind(HPA_KIND, rows(), rows(), cx);
    t.cursor_on(&t.prod, HPA_KIND, "frontend-hpa", cx);
    dispatch(&t, RowAction::EditHpaRange, cx);
    assert!(t.popover(cx).is_none());
    assert!(!t.has_dialog(cx));
    assert!(writes(&prod_api).is_empty());
    assert!(writes(&t.stg_api).is_empty());
}

/// `cluster`'s access report with every right but `denied` granted.
fn deny(t: &Clusters, cluster: &ClusterRef, denied: AccessCheck, cx: &mut TestAppContext) {
    let reviews = AccessCheck::ALL
        .into_iter()
        .map(|check| AccessReview {
            check,
            decision: if check == denied {
                AccessDecision::Denied { reason: None }
            } else {
                AccessDecision::Allowed
            },
        })
        .collect();
    let session = slot_session(&t.fixture, cluster, cx);
    session.update(cx, |session, cx| {
        session.set_access_for_test(AccessState::Known(AccessReport { reviews }), cx);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn edit_min_max_without_patch_hpa_permission_opens_no_popover(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-denied", cx);
    on_stg_hpa(&t, cx);
    deny(&t, &t.stg, AccessCheck::PatchHorizontalPodAutoscalers, cx);
    dispatch(&t, RowAction::EditHpaRange, cx);
    assert!(t.popover(cx).is_none());
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
    let buttons = {
        t.show_kind(HPA_KIND, Vec::new(), hpa_rows(&[("a", 1, 2, 1)]), cx);
        t.tick(&[0], cx);
        t.bulk_buttons(cx)
    };
    assert_eq!(
        Clusters::state_of(&buttons, "Edit limits"),
        BulkState::Off("Not permitted: patch horizontalpodautoscalers".into())
    );
}

#[gpui_kit::test]
fn production_edit_min_max_types_the_cluster_name(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-prod", cx);
    let prod_api = activate_prod(&t, cx);
    t.set_lock(&t.prod, WriteLock::Unlocked, cx);
    let rows = || hpa_rows(&[("frontend-hpa", 3, 20, 9)]);
    t.show_kind(HPA_KIND, rows(), rows(), cx);
    t.cursor_on(&t.prod, HPA_KIND, "frontend-hpa", cx);
    let popover = open_range_popover(&t, cx);
    type_range(&t, &popover, ("2", "30"), cx);
    submit(&t, &popover, cx);
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        );
    });
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&prod_api).len(), 1, "the name was not typed");
    t.type_name("prod-a", cx);
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&prod_api).len() == 2);
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_held_enter_does_not_confirm_the_range(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-held", cx);
    on_stg_hpa(&t, cx);
    let popover = open_range_popover(&t, cx);
    type_range(&t, &popover, ("2", "30"), cx);
    submit(&t, &popover, cx);
    t.wait_for_dry_run(cx);
    t.fixture.draw_twice(cx);
    for _ in 0..3 {
        t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(key_down("enter", true).to_platform_input(), cx);
        });
    }
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), 1, "only the dry-run was sent");
}

#[gpui_kit::test]
fn a_range_for_a_row_that_left_the_list_is_refused_with_a_notice(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-gone", cx);
    on_stg_hpa(&t, cx);
    let popover = open_range_popover(&t, cx);
    type_range(&t, &popover, ("2", "30"), cx);
    // The HPA is deleted while the form is open.
    t.show_kind(HPA_KIND, Vec::new(), Vec::new(), cx);
    let before = t.notification_count(cx);
    submit(&t, &popover, cx);
    cx.run_until_parked();
    assert_eq!(t.notification_count(cx), before + 1);
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_audit_line_names_the_range(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-audit", cx);
    let dir = t.enable_audit_folder("hpa-range-audit", cx);
    on_stg_hpa(&t, cx);
    let popover = open_range_popover(&t, cx);
    type_range(&t, &popover, ("2", "30"), cx);
    submit(&t, &popover, cx);
    t.wait_for_dry_run(cx);
    assert!(audit_lines(&dir).is_empty(), "a dry-run is not audited");
    t.confirm(cx);
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Set limits");
    assert_eq!(line["object"]["kind"], "HorizontalPodAutoscaler");
    assert_eq!(line["object"]["name"], "frontend-hpa");
    assert_eq!(line["outcome"], "applied");
    let fields = line["fields"].as_array().expect("fields");
    let value_of = |path: &str| {
        fields
            .iter()
            .find(|field| field["path"] == path)
            .map(|field| field["value"].clone())
    };
    assert_eq!(value_of("spec.minReplicas"), Some("2".into()));
    assert_eq!(value_of("spec.maxReplicas"), Some("30".into()));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- Edit limits on the ticked HPAs ----

const NAMES: [&str; 3] = ["api-hpa", "web-hpa", "worker-hpa"];

fn ticked_hpas(t: &Clusters, cx: &mut TestAppContext) {
    let rows = hpa_rows(&[
        ("api-hpa", 1, 5, 2),
        ("web-hpa", 2, 10, 6),
        ("worker-hpa", 3, 12, 12),
    ]);
    t.show_kind(HPA_KIND, Vec::new(), rows, cx);
    t.tick(&[0, 1, 2], cx);
}

#[gpui_kit::test]
fn the_edit_limits_button_follows_the_ticks_and_the_gate(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-bulk-buttons", cx);
    ticked_hpas(&t, cx);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Edit limits"),
        BulkState::Ready(ResourceAction::EditHpaRange)
    );
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Edit limits"),
        BulkState::Off("stg-b is read-only".into())
    );
}

#[gpui_kit::test]
fn edit_limits_applies_one_range_and_audits_each_object(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-bulk", cx);
    let dir = t.enable_audit_folder("hpa-bulk", cx);
    ticked_hpas(&t, cx);
    t.press_bulk("Edit limits", cx);
    let popover = t.popover(cx).expect("the popover is open");
    popover.read_with(cx, |popover, cx| {
        assert_eq!(
            popover.typed_range(cx),
            Some((String::new(), String::new()))
        );
        assert_eq!(popover.title_text(), "Edit limits of 3 hpas");
        assert!(!popover.can_submit(cx));
    });
    type_range(&t, &popover, ("4", "10"), cx);
    submit(&t, &popover, cx);
    assert_eq!(t.dialog_label(cx), "Set limits of 3 hpas to 4\u{2013}10");
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(dialog.confirm_text().as_deref(), Some("Set limits 3"));
        // api-hpa runs 2 replicas, below 4; worker-hpa runs 12, above 10; web-hpa sits inside.
        assert_eq!(
            texts(dialog.warning_lines()),
            ["2 of 3 will scale their workload at once (their replicas are outside 4\u{2013}10)"]
        );
    });
    t.wait_for_dry_run(cx);
    assert_eq!(t.items(cx), vec![ItemProgress::Passed; 3]);
    let sent = writes(&t.stg_api);
    let paths: Vec<&str> = sent.iter().map(|request| request.path.as_str()).collect();
    let expected: Vec<String> = NAMES
        .iter()
        .map(|name| {
            format!("/apis/autoscaling/v2/namespaces/team-a/horizontalpodautoscalers/{name}")
        })
        .collect();
    assert_eq!(
        paths,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(audit_lines(&dir).is_empty());
    t.confirm(cx);
    t.wait_for("three audit lines", cx, |_| audit_lines(&dir).len() == 3);
    let lines = audit_lines(&dir);
    let objects: Vec<String> = lines
        .iter()
        .map(|line| {
            line["object"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(objects, NAMES);
    for line in &lines {
        assert_eq!(line["action"], "Set limits");
        assert_eq!(line["cluster"], "stg-b");
    }
    let commits: Vec<_> = writes(&t.stg_api)
        .into_iter()
        .filter(|request| !request.has_query_key("dryRun"))
        .collect();
    assert_eq!(commits.len(), 3);
    assert!(
        commits
            .iter()
            .all(|request| request.body == r#"{"spec":{"minReplicas":4,"maxReplicas":10}}"#)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn edit_limits_skips_the_rows_already_in_range(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-bulk-skip", cx);
    ticked_hpas(&t, cx);
    t.press_bulk("Edit limits", cx);
    let popover = t.popover(cx).expect("the popover is open");
    // web-hpa is 2-10 already.
    type_range(&t, &popover, ("2", "10"), cx);
    submit(&t, &popover, cx);
    assert_eq!(t.dialog_label(cx), "Set limits of 2 hpas to 2\u{2013}10");
    t.wait_for_dry_run(cx);
    assert_eq!(writes(&t.stg_api).len(), 2);
}

#[gpui_kit::test]
fn the_palette_lists_edit_min_max_and_runs_the_same_arm(cx: &mut TestAppContext) {
    let t = edit_clusters("hpa-range-palette", cx);
    on_stg_hpa(&t, cx);
    let snapshot = t
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.palette_snapshot(&parse_query(""), cx));
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| {
            matches!(
                entry.target,
                crate::palette_search::PaletteTarget::RowAction(RowAction::EditHpaRange)
            )
        })
        .expect("the palette offers Edit min / max");
    assert!(entry.is_enabled());
    dispatch(&t, RowAction::EditHpaRange, cx);
    assert!(t.popover(cx).is_some());
}

// ---- PVC Expand ----

const PVC_KIND: ResourceKind = ResourceKind::PersistentVolumeClaims;
const CLAIM_PATH: &str = "/api/v1/namespaces/team-a/persistentvolumeclaims/data-kafka-0";
const RESIZE_REFUSED: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"persistentvolumeclaims \"data-kafka-0\" is forbidden: only dynamically provisioned pvc can be resized and the storageclass that provisions the pvc must support resize","reason":"Forbidden","code":403}"#;

fn claim_rows(specs: &[(&str, &str, &str)]) -> Vec<KindRow> {
    specs
        .iter()
        .map(|(name, requested, capacity)| {
            persistent_volume_claim_row(&claim(name, requested, capacity))
        })
        .collect()
}

/// The cursor on `data-kafka-0` (100Gi, bound, class gp3) of staging.
fn on_stg_claim(t: &Clusters, cx: &mut TestAppContext) {
    let rows = || claim_rows(&[("data-kafka-0", "100Gi", "100Gi")]);
    t.show_kind(PVC_KIND, rows(), rows(), cx);
    t.cursor_on(&t.stg, PVC_KIND, "data-kafka-0", cx);
}

fn open_expand_popover(t: &Clusters, cx: &mut TestAppContext) -> Entity<ValuePopover> {
    dispatch(t, RowAction::ExpandClaim, cx);
    t.popover(cx).expect("the popover is open")
}

fn type_storage(t: &Clusters, popover: &Entity<ValuePopover>, text: &str, cx: &mut TestAppContext) {
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.type_storage(text, window, cx));
    });
}

#[gpui_kit::test]
fn expand_opens_the_popover_then_the_confirm_dialog(cx: &mut TestAppContext) {
    let t = edit_clusters("expand", cx);
    on_stg_claim(&t, cx);
    let popover = open_expand_popover(&t, cx);
    assert!(!t.has_dialog(cx));
    // It opens on the size the claim has, which is no growth.
    popover.read_with(cx, |popover, cx| {
        assert_eq!(popover.typed_text(cx), "100Gi");
        assert_eq!(popover.state_line(cx), "Now 100Gi \u{b7} class gp3");
        assert!(!popover.can_submit(cx));
        assert_eq!(
            popover.error_line(cx).as_deref(),
            Some("Must be larger than 100Gi")
        );
    });
    type_storage(&t, &popover, " 150Gi ", cx);
    popover.read_with(cx, |popover, cx| {
        assert!(popover.can_submit(cx));
        assert_eq!(popover.error_line(cx), None);
        assert_eq!(
            texts(popover.warning_lines(cx)),
            ["A volume cannot shrink; this cannot be undone"]
        );
    });
    submit(&t, &popover, cx);
    assert!(t.popover(cx).is_none(), "the popover closes on its way out");
    assert_eq!(
        t.dialog_label(cx),
        "Expand claim data-kafka-0 from 100Gi to 150Gi"
    );
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        assert_eq!(dialog.confirm_text().as_deref(), Some("Expand"));
        assert_eq!(
            texts(dialog.warning_lines()),
            ["A volume cannot shrink; this cannot be undone"]
        );
    });
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].path, CLAIM_PATH);
    assert!(sent[0].has_query("dryRun", "All"));
    // The trimmed text, as typed.
    assert_eq!(
        sent[0].body,
        r#"{"spec":{"resources":{"requests":{"storage":"150Gi"}}}}"#
    );
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
    let sent = writes(&t.stg_api);
    assert!(!sent[1].has_query_key("dryRun"));
    assert_eq!(sent[0].body, sent[1].body);
}

#[gpui_kit::test]
fn expand_form_validates_the_size(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-validate", cx);
    on_stg_claim(&t, cx);
    let popover = open_expand_popover(&t, cx);
    let cases = [
        ("100Gi", Some("Must be larger than 100Gi"), false),
        ("99Gi", Some("Must be larger than 100Gi"), false),
        ("abc", Some("Enter a size such as 150Gi"), false),
        ("0", Some("Enter a size such as 150Gi"), false),
        ("-5Gi", Some("Enter a size such as 150Gi"), false),
        ("", None, false),
        ("101Gi", None, true),
        ("1Ti", None, true),
    ];
    for (text, error, is_enabled) in cases {
        type_storage(&t, &popover, text, cx);
        popover.read_with(cx, |popover, cx| {
            assert_eq!(popover.error_line(cx).as_deref(), error, "{text:?}");
            assert_eq!(popover.can_submit(cx), is_enabled, "{text:?}");
        });
    }
}

#[gpui_kit::test]
fn expand_compares_with_the_size_the_claim_has_now(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-moved", cx);
    on_stg_claim(&t, cx);
    let popover = open_expand_popover(&t, cx);
    type_storage(&t, &popover, "120Gi", cx);
    assert!(popover.read_with(cx, |popover, cx| popover.can_submit(cx)));
    // Someone asked for 150Gi while the form was open: 120Gi would shrink that request.
    t.show_kind(
        PVC_KIND,
        Vec::new(),
        claim_rows(&[("data-kafka-0", "150Gi", "100Gi")]),
        cx,
    );
    popover.read_with(cx, |popover, cx| {
        assert!(!popover.can_submit(cx));
        assert_eq!(
            popover.error_line(cx).as_deref(),
            Some("Must be larger than 150Gi")
        );
    });
}

#[gpui_kit::test]
fn expand_on_a_claim_that_is_not_bound_opens_no_popover(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-pending", cx);
    let mut pending = claim("data-kafka-0", "100Gi", "100Gi");
    pending.phase = "Pending".to_owned();
    t.show_kind(
        PVC_KIND,
        Vec::new(),
        vec![persistent_volume_claim_row(&pending)],
        cx,
    );
    t.cursor_on(&t.stg, PVC_KIND, "data-kafka-0", cx);
    let before = t.notification_count(cx);
    dispatch(&t, RowAction::ExpandClaim, cx);
    cx.run_until_parked();
    assert!(t.popover(cx).is_none());
    assert!(!t.has_dialog(cx));
    assert_eq!(t.notification_count(cx), before + 1);
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn expand_on_a_locked_production_row_opens_no_popover(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-locked", cx);
    let prod_api = activate_prod(&t, cx);
    let rows = || claim_rows(&[("data-kafka-0", "100Gi", "100Gi")]);
    t.show_kind(PVC_KIND, rows(), rows(), cx);
    t.cursor_on(&t.prod, PVC_KIND, "data-kafka-0", cx);
    dispatch(&t, RowAction::ExpandClaim, cx);
    assert!(t.popover(cx).is_none());
    assert!(!t.has_dialog(cx));
    assert!(writes(&prod_api).is_empty());
}

#[gpui_kit::test]
fn production_expand_types_the_cluster_name(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-prod", cx);
    let prod_api = activate_prod(&t, cx);
    t.set_lock(&t.prod, WriteLock::Unlocked, cx);
    let rows = || claim_rows(&[("data-kafka-0", "100Gi", "100Gi")]);
    t.show_kind(PVC_KIND, rows(), rows(), cx);
    t.cursor_on(&t.prod, PVC_KIND, "data-kafka-0", cx);
    let popover = open_expand_popover(&t, cx);
    type_storage(&t, &popover, "150Gi", cx);
    submit(&t, &popover, cx);
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        );
    });
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&prod_api).len(), 1, "the name was not typed");
    t.type_name("prod-a", cx);
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&prod_api).len() == 2);
    assert!(writes(&t.stg_api).is_empty());
}

fn resize_refused(request: &RecordedRequest) -> (u16, String) {
    if request.method == "PATCH" {
        (403, RESIZE_REFUSED.to_owned())
    } else {
        (404, NOT_FOUND.to_owned())
    }
}

#[gpui_kit::test]
fn the_dry_run_reports_the_admission_refusal_and_blocks_the_commit(cx: &mut TestAppContext) {
    let t = edit_clusters_answering("expand-refused", resize_refused, cx);
    on_stg_claim(&t, cx);
    let popover = open_expand_popover(&t, cx);
    type_storage(&t, &popover, "150Gi", cx);
    submit(&t, &popover, cx);
    t.wait_for_dry_run(cx);
    let state = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.dry_run_state());
    let Some(DryRunState::Failed(text)) = state else {
        panic!("the dry-run should fail: {state:?}");
    };
    assert!(text.contains("the change is invalid"), "{text}");
    assert!(text.contains("only dynamically provisioned pvc"), "{text}");
    assert!(!text.contains("not permitted"), "{text}");
    assert!(t.block(cx).is_some());
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), 1, "only the dry-run was sent");
}

#[gpui_kit::test]
fn the_expand_audit_line_names_the_size(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-audit", cx);
    let dir = t.enable_audit_folder("expand-audit", cx);
    on_stg_claim(&t, cx);
    let popover = open_expand_popover(&t, cx);
    type_storage(&t, &popover, "150Gi", cx);
    submit(&t, &popover, cx);
    t.wait_for_dry_run(cx);
    assert!(audit_lines(&dir).is_empty(), "a dry-run is not audited");
    t.confirm(cx);
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Expand");
    assert_eq!(line["object"]["kind"], "PersistentVolumeClaim");
    assert_eq!(line["object"]["name"], "data-kafka-0");
    let fields = line["fields"].as_array().expect("fields");
    assert!(
        fields.iter().any(|field| {
            field["path"] == "spec.resources.requests.storage" && field["value"] == "150Gi"
        }),
        "{fields:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_held_enter_does_not_confirm_the_expand(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-held", cx);
    on_stg_claim(&t, cx);
    let popover = open_expand_popover(&t, cx);
    type_storage(&t, &popover, "150Gi", cx);
    submit(&t, &popover, cx);
    t.wait_for_dry_run(cx);
    t.fixture.draw_twice(cx);
    for _ in 0..3 {
        t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(key_down("enter", true).to_platform_input(), cx);
        });
    }
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), 1, "only the dry-run was sent");
}

const CLAIM_NAMES: [&str; 3] = ["data-a", "data-b", "data-c"];

fn ticked_claims(t: &Clusters, cx: &mut TestAppContext) {
    let rows = claim_rows(&[
        ("data-a", "100Gi", "100Gi"),
        ("data-b", "300Gi", "300Gi"),
        ("data-c", "50Gi", "50Gi"),
    ]);
    t.show_kind(PVC_KIND, Vec::new(), rows, cx);
    t.tick(&[0, 1, 2], cx);
}

#[gpui_kit::test]
fn the_expand_button_follows_the_ticks_and_the_gate(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-bulk-buttons", cx);
    ticked_claims(&t, cx);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Expand"),
        BulkState::Ready(ResourceAction::ExpandClaim)
    );
    deny(&t, &t.stg, AccessCheck::PatchPersistentVolumeClaims, cx);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Expand"),
        BulkState::Off("Not permitted: patch persistentvolumeclaims".into())
    );
}

#[gpui_kit::test]
fn bulk_expand_applies_one_size_skips_the_large_and_audits_each_object(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-bulk", cx);
    let dir = t.enable_audit_folder("expand-bulk", cx);
    ticked_claims(&t, cx);
    t.press_bulk("Expand", cx);
    let popover = t.popover(cx).expect("the popover is open");
    popover.read_with(cx, |popover, cx| {
        assert_eq!(popover.typed_text(cx), "");
        assert_eq!(popover.title_text(), "Expand 3 claims");
        assert!(!popover.can_submit(cx));
    });
    type_storage(&t, &popover, "200Gi", cx);
    submit(&t, &popover, cx);
    // data-b is 300Gi already.
    assert_eq!(t.dialog_label(cx), "Expand 2 claims to 200Gi");
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(dialog.confirm_text().as_deref(), Some("Expand 2"));
        assert_eq!(
            texts(dialog.warning_lines()),
            ["A volume cannot shrink; this cannot be undone"]
        );
    });
    t.wait_for_dry_run(cx);
    assert_eq!(t.items(cx), vec![ItemProgress::Passed; 2]);
    assert!(audit_lines(&dir).is_empty());
    t.confirm(cx);
    t.wait_for("two audit lines", cx, |_| audit_lines(&dir).len() == 2);
    let lines = audit_lines(&dir);
    let objects: Vec<String> = lines
        .iter()
        .map(|line| {
            line["object"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(objects, [CLAIM_NAMES[0], CLAIM_NAMES[2]]);
    let commits: Vec<_> = writes(&t.stg_api)
        .into_iter()
        .filter(|request| !request.has_query_key("dryRun"))
        .collect();
    assert_eq!(commits.len(), 2);
    assert!(commits.iter().all(|request| {
        request.body == r#"{"spec":{"resources":{"requests":{"storage":"200Gi"}}}}"#
    }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_running_batch_refuses_a_bulk_expand(cx: &mut TestAppContext) {
    let t = edit_clusters("expand-bulk-running", cx);
    ticked_claims(&t, cx);
    t.fixture
        .shell
        .update(cx, |shell, _| shell.running_batches.insert(t.stg.clone()));
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Expand"),
        BulkState::Off("A batch is running".into())
    );
}

// ---- Set as default storage class ----

const CLASS_KIND: ResourceKind = ResourceKind::StorageClasses;
const INVALID: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"storageclass is invalid","reason":"Invalid","details":{"causes":[{"reason":"FieldValueInvalid","message":"x","field":"metadata.annotations"}]},"code":422}"#;
const NEWER: &str = "2025-06-01T00:00:00Z";
const OLDER: &str = "2024-01-01T00:00:00Z";

/// `(name, is_default, created)`; the order is the order of the rows.
fn class_rows(specs: &[(&str, bool, &str)]) -> Vec<KindRow> {
    specs
        .iter()
        .map(|(name, is_default, created)| {
            let mut summary = class(name, *is_default, true);
            summary.created_at = created.parse().ok();
            storage_class_row(&summary)
        })
        .collect()
}

/// Staging holds `gp3` (not default, newer) and `io2` (the default).
fn on_stg_classes(t: &Clusters, cx: &mut TestAppContext) {
    let rows = class_rows(&[("gp3", false, NEWER), ("io2", true, OLDER)]);
    t.show_kind(CLASS_KIND, Vec::new(), rows, cx);
    cursor_on_class(t, &t.stg, "gp3", cx);
}

/// The cursor on a class (cluster-scoped, so its key has no namespace).
fn cursor_on_class(t: &Clusters, cluster: &ClusterRef, name: &str, cx: &mut TestAppContext) {
    let key = ResourceKey::Kind {
        kind: CLASS_KIND,
        namespace: None,
        name: name.to_owned(),
    };
    let object = ClusterObject::new(cluster.clone(), key);
    t.fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object), cx);
    });
    cx.run_until_parked();
}

fn is_commit_of(request: &RecordedRequest, class: &str) -> bool {
    request.method == "PATCH"
        && request.path == format!("/apis/storage.k8s.io/v1/storageclasses/{class}")
        && !request.has_query_key("dryRun")
}

fn set_commit_fails(request: &RecordedRequest) -> (u16, String) {
    if is_commit_of(request, "gp3") {
        (422, INVALID.to_owned())
    } else {
        accept_patches(request)
    }
}

fn unset_commit_fails(request: &RecordedRequest) -> (u16, String) {
    if is_commit_of(request, "io2") {
        (422, INVALID.to_owned())
    } else {
        accept_patches(request)
    }
}

const SET_BODY: &str =
    r#"{"metadata":{"annotations":{"storageclass.kubernetes.io/is-default-class":"true"}}}"#;
const UNSET_BODY: &str = r#"{"metadata":{"annotations":{"storageclass.beta.kubernetes.io/is-default-class":null,"storageclass.kubernetes.io/is-default-class":"false"}}}"#;

#[gpui_kit::test]
fn set_default_sets_the_new_class_then_unsets_the_old(cx: &mut TestAppContext) {
    let t = edit_clusters("default", cx);
    let dir = t.enable_audit_folder("default", cx);
    on_stg_classes(&t, cx);
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    assert_eq!(t.dialog_label(cx), "Make gp3 the default storage class");
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        assert_eq!(dialog.confirm_text().as_deref(), Some("Set default 2"));
        assert_eq!(
            texts(dialog.warning_lines()),
            [
                "New claims without a class will use gp3; existing claims keep their class",
                "io2 stops being the default"
            ]
        );
    });
    t.wait_for_dry_run(cx);
    assert_eq!(t.items(cx), vec![ItemProgress::Passed; 2]);
    // Every dry-run first, in the order of the commits.
    let sent = writes(&t.stg_api);
    let order: Vec<(&str, bool)> = sent
        .iter()
        .map(|request| (request.path.as_str(), request.has_query("dryRun", "All")))
        .collect();
    assert_eq!(
        order,
        [
            ("/apis/storage.k8s.io/v1/storageclasses/gp3", true),
            ("/apis/storage.k8s.io/v1/storageclasses/io2", true)
        ]
    );
    assert!(audit_lines(&dir).is_empty());
    t.confirm(cx);
    t.wait_for("two audit lines", cx, |_| audit_lines(&dir).len() == 2);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 4);
    assert!(is_commit_of(&sent[2], "gp3") && is_commit_of(&sent[3], "io2"));
    assert_eq!(sent[2].body, SET_BODY);
    // The beta key is removed with the GA key set to false.
    let unset: serde_json::Value = serde_json::from_str(&sent[3].body).expect("JSON");
    let expected: serde_json::Value = serde_json::from_str(UNSET_BODY).expect("JSON");
    assert_eq!(unset, expected);
    // One audit line per class, so the log shows both halves.
    let lines = audit_lines(&dir);
    let objects: Vec<&str> = lines
        .iter()
        .map(|line| line["object"]["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(objects, ["gp3", "io2"]);
    assert_eq!(lines[0]["fields"][0]["value"], "true");
    assert_eq!(lines[1]["fields"][0]["value"], "false");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn set_default_stops_when_the_set_fails(cx: &mut TestAppContext) {
    let t = edit_clusters_answering("default-set-fails", set_commit_fails, cx);
    let dir = t.enable_audit_folder("default-set-fails", cx);
    on_stg_classes(&t, cx);
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("the batch to end", cx, |cx| {
        t.fixture
            .shell
            .read_with(cx, |shell, _| !shell.running_batches.contains(&t.stg))
    });
    // Two dry-runs and the failed set: the unset was never sent, so the old default stays.
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 3, "{sent:?}");
    assert!(is_commit_of(&sent[2], "gp3"));
    let lines = audit_lines(&dir);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["outcome"], "failed");
    // The notice says how far it got and offers Retry.
    t.fixture.draw_twice(cx);
    assert!(t.fixture.is_drawn("batch-retry", cx));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Staging after the set went through and the unset did not: both classes are marked default.
fn show_two_defaults(t: &Clusters, cx: &mut TestAppContext) {
    let rows = class_rows(&[("gp3", true, NEWER), ("io2", true, OLDER)]);
    t.show_kind(CLASS_KIND, Vec::new(), rows, cx);
}

#[gpui_kit::test]
fn a_partial_default_change_offers_retry_that_replans_only_the_unsets(cx: &mut TestAppContext) {
    let t = edit_clusters_answering("default-partial", unset_commit_fails, cx);
    on_stg_classes(&t, cx);
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("the batch to end", cx, |cx| {
        t.fixture
            .shell
            .read_with(cx, |shell, _| !shell.running_batches.contains(&t.stg))
    });
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 4, "{sent:?}");
    assert!(is_commit_of(&sent[2], "gp3") && is_commit_of(&sent[3], "io2"));
    t.fixture.draw_twice(cx);
    assert!(t.fixture.is_drawn("batch-retry", cx));
    // The watch shows the first commit.
    show_two_defaults(&t, cx);
    // The normal path refuses: gp3 is the default now.
    let before = t.notification_count(cx);
    cursor_on_class(&t, &t.stg, "gp3", cx);
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    cx.run_until_parked();
    assert_eq!(t.notification_count(cx), before + 1);
    // The Retry bypasses that and plans only the unset, with the state left behind named.
    let subject = ClusterObject::new(
        t.stg.clone(),
        ResourceKey::Kind {
            kind: CLASS_KIND,
            namespace: None,
            name: "gp3".to_owned(),
        },
    );
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.retry_batch(ResourceAction::SetDefaultStorageClass, &subject, window, cx);
        });
    });
    assert_eq!(t.dialog_label(cx), "Make gp3 the default storage class");
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(dialog.confirm_text().as_deref(), Some("Set default 1"));
        assert_eq!(
            texts(dialog.warning_lines()),
            [
                "io2 stops being the default",
                "Both gp3 and io2 are marked default; the cluster uses the newer one (gp3) until io2 is unset"
            ]
        );
    });
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 5, "{sent:?}");
    assert_eq!(sent[4].path, "/apis/storage.k8s.io/v1/storageclasses/io2");
    assert!(sent[4].has_query("dryRun", "All"));
}

#[gpui_kit::test]
fn the_retry_is_still_gated(cx: &mut TestAppContext) {
    let t = edit_clusters("default-retry-gate", cx);
    show_two_defaults(&t, cx);
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    let subject = ClusterObject::new(
        t.stg.clone(),
        ResourceKey::Kind {
            kind: CLASS_KIND,
            namespace: None,
            name: "gp3".to_owned(),
        },
    );
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.retry_batch(ResourceAction::SetDefaultStorageClass, &subject, window, cx);
        });
    });
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_lock_that_comes_on_after_the_first_item_stops_the_rest(cx: &mut TestAppContext) {
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = std::sync::Arc::new(std::sync::Mutex::new(Some(gate)));
    let held = std::sync::Arc::clone(&gate);
    let respond = move |request: &RecordedRequest| {
        if is_commit_of(request, "gp3") {
            // The fake server holds the first commit until the test has locked the cluster.
            let receiver = held.lock().ok().and_then(|mut gate| gate.take());
            if let Some(receiver) = receiver {
                let _ = receiver.recv();
            }
        }
        accept_patches(request)
    };
    let t = edit_clusters_answering("default-blocked", respond, cx);
    let dir = t.enable_audit_folder("default-blocked", cx);
    on_stg_classes(&t, cx);
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("the first commit", cx, |_| writes(&t.stg_api).len() == 3);
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    release.send(()).expect("the server waits for the release");
    t.wait_for("the first audit line", cx, |_| audit_lines(&dir).len() == 1);
    t.wait_for("the batch to end", cx, |cx| {
        t.fixture
            .shell
            .read_with(cx, |shell, _| !shell.running_batches.contains(&t.stg))
    });
    // The unset was blocked before it was sent, and is not recorded.
    assert_eq!(writes(&t.stg_api).len(), 3);
    assert_eq!(audit_lines(&dir).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn set_default_on_the_default_opens_no_dialog(cx: &mut TestAppContext) {
    let t = edit_clusters("default-already", cx);
    on_stg_classes(&t, cx);
    cursor_on_class(&t, &t.stg, "io2", cx);
    let before = t.notification_count(cx);
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    cx.run_until_parked();
    assert!(!t.has_dialog(cx));
    assert_eq!(t.notification_count(cx), before + 1);
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn set_default_without_a_previous_default_is_one_item(cx: &mut TestAppContext) {
    let t = edit_clusters("default-first", cx);
    let rows = class_rows(&[("gp3", false, NEWER), ("io2", false, OLDER)]);
    t.show_kind(CLASS_KIND, Vec::new(), rows, cx);
    cursor_on_class(&t, &t.stg, "gp3", cx);
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(dialog.confirm_text().as_deref(), Some("Set default 1"));
        assert_eq!(
            texts(dialog.warning_lines()),
            ["New claims without a class will use gp3; existing claims keep their class"]
        );
    });
    t.wait_for_dry_run(cx);
    assert_eq!(writes(&t.stg_api).len(), 1);
}

#[gpui_kit::test]
fn production_set_default_types_the_cluster_name(cx: &mut TestAppContext) {
    let t = edit_clusters("default-prod", cx);
    let prod_api = activate_prod(&t, cx);
    t.set_lock(&t.prod, WriteLock::Unlocked, cx);
    let rows = class_rows(&[("gp3", false, NEWER), ("io2", true, OLDER)]);
    t.show_kind(CLASS_KIND, rows, Vec::new(), cx);
    cursor_on_class(&t, &t.prod, "gp3", cx);
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        );
    });
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&prod_api).len(), 2, "the name was not typed");
    t.type_name("prod-a", cx);
    t.confirm(cx);
    t.wait_for("the commits", cx, |_| writes(&prod_api).len() == 4);
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn set_default_needs_exactly_one_ticked_class(cx: &mut TestAppContext) {
    let t = edit_clusters("default-bulk", cx);
    let rows = class_rows(&[
        ("gp3", false, NEWER),
        ("io2", true, OLDER),
        ("st1", false, OLDER),
    ]);
    t.show_kind(CLASS_KIND, Vec::new(), rows, cx);
    t.tick(&[0, 2], cx);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Set default"),
        BulkState::Off("Tick one storage class".into())
    );
    // Ticking the default alone: there is nothing to set.
    t.tick(&[0, 2], cx);
    t.tick(&[1], cx);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Set default"),
        BulkState::Off("Already the default".into())
    );
    t.tick(&[1, 0], cx);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Set default"),
        BulkState::Ready(ResourceAction::SetDefaultStorageClass)
    );
    t.press_bulk("Set default", cx);
    assert_eq!(t.dialog_label(cx), "Make gp3 the default storage class");
    t.wait_for_dry_run(cx);
    assert_eq!(writes(&t.stg_api).len(), 2);
}

#[gpui_kit::test]
fn set_default_is_off_without_permission_and_while_locked(cx: &mut TestAppContext) {
    let t = edit_clusters("default-gate", cx);
    on_stg_classes(&t, cx);
    t.tick(&[0], cx);
    deny(&t, &t.stg, AccessCheck::PatchStorageClasses, cx);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Set default"),
        BulkState::Off("Not permitted: patch storageclasses".into())
    );
    dispatch(&t, RowAction::SetDefaultStorageClass, cx);
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_class_list_is_loaded_only_on_the_storage_classes_screen(cx: &mut TestAppContext) {
    let t = edit_clusters("default-loaded", cx);
    let names = |t: &Clusters, cx: &mut TestAppContext| -> Vec<String> {
        slot_session(&t.fixture, &t.stg, cx).read_with(cx, |session, _| {
            session
                .live()
                .map(|live| {
                    live.loaded_storage_classes()
                        .iter()
                        .map(|class| class.name.clone())
                        .collect()
                })
                .unwrap_or_default()
        })
    };
    on_stg_classes(&t, cx);
    assert_eq!(names(&t, cx), ["gp3", "io2"]);
    let rows = || claim_rows(&[("data-kafka-0", "100Gi", "100Gi")]);
    t.show_kind(PVC_KIND, rows(), rows(), cx);
    assert!(names(&t, cx).is_empty());
}

// ---- Review fixes ----

#[gpui_kit::test]
fn the_end_note_warns_when_a_default_was_made_meanwhile(cx: &mut TestAppContext) {
    let t = edit_clusters("default-meanwhile", cx);
    on_stg_classes(&t, cx);
    let (gp3, io2) = (class("gp3", false, true), class("io2", true, true));
    let cluster = t.stg.clone();
    let scope = crate::workload_actions::WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    let batch =
        crate::resource_edits::default_class_intent(&scope, &gp3, &[&gp3, &io2]).expect("a plan");
    let results = vec![ItemProgress::Done; batch.plan.items.len()];
    let note = |t: &Clusters, cx: &mut TestAppContext| {
        t.fixture.shell.read_with(cx, |shell, cx| {
            shell.many_defaults_note(&batch, &results, cx)
        })
    };
    // The list still shows the old state (the watch has not caught up): one default after the run.
    assert_eq!(note(&t, cx), None);
    // st1 became the default while the dialog was open.
    let rows = class_rows(&[
        ("gp3", false, NEWER),
        ("io2", true, OLDER),
        ("st1", true, OLDER),
    ]);
    t.show_kind(CLASS_KIND, Vec::new(), rows, cx);
    assert_eq!(
        note(&t, cx).as_deref(),
        Some(
            "More than one storage class is marked default now (gp3, st1); the cluster uses the newest"
        )
    );
}

#[gpui_kit::test]
fn a_retry_off_the_storage_classes_screen_says_why_and_opens_nothing(cx: &mut TestAppContext) {
    let t = edit_clusters("default-retry-off-screen", cx);
    let rows = || claim_rows(&[("data-kafka-0", "100Gi", "100Gi")]);
    t.show_kind(PVC_KIND, rows(), rows(), cx);
    let subject = ClusterObject::new(
        t.stg.clone(),
        ResourceKey::Kind {
            kind: CLASS_KIND,
            namespace: None,
            name: "gp3".to_owned(),
        },
    );
    let before = t.notification_count(cx);
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.retry_batch(ResourceAction::SetDefaultStorageClass, &subject, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(!t.has_dialog(cx));
    assert_eq!(t.notification_count(cx), before + 1);
    assert!(writes(&t.stg_api).is_empty());
}
