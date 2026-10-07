//! The workload actions of spec 0032 in a headless window over two loaded clusters, one active at a
//! time: the fixture starts on `prod-a` (locked at open), switches to `stg-b` (unlocked), and makes
//! it live. `activate_prod` does the same for `prod-a`. Each session answers from its own fake API
//! server, so a test sees which cluster a request reached and nothing leaves the machine.

use cluster::fake_api::{FakeApi, RecordedRequest};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, InputEvent as _, KeyDownEvent, Keystroke, TestAppContext};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{
    Clusters, audit_lines, go_live_answering, session_of, switch_to, writes,
};
use super::batch_write::{ItemProgress, MAX_BATCH_ITEMS};
use super::write_flow::DryRunState;
use super::*;
use crate::batch_rows::{cron_job_row, job_row};
use crate::environment::Environment;
use crate::kind_row::KindRow;
use crate::resource_actions::{ResourceAction, RowAction};
use crate::row_selection::{BulkButton, BulkState};
use crate::value_popover::ValuePopover;
use crate::workload_actions::workload_actions_tests::{cron_job, deployment, job};
use crate::workload_rows::deployment_row;
use crate::write_guard::{DialogConfirm, WriteLock};

const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;
const DEPLOYMENT_JSON: &str = r#"{"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"api","namespace":"team-a"}}"#;
const SCALE_JSON: &str = r#"{"apiVersion":"autoscaling/v1","kind":"Scale","metadata":{"name":"api","namespace":"team-a"},"spec":{"replicas":5}}"#;
const CRON_JOB_JSON: &str = r#"{"apiVersion":"batch/v1","kind":"CronJob","metadata":{"name":"reconcile","namespace":"team-a","uid":"cron-uid"},"spec":{"schedule":"*/5 * * * *","jobTemplate":{"spec":{"template":{"spec":{"containers":[{"name":"c","image":"busybox"}],"restartPolicy":"Never"}}}}}}"#;
const JOB_JSON: &str = r#"{"apiVersion":"batch/v1","kind":"Job","metadata":{"name":"etl","namespace":"team-a"},"spec":{"template":{"spec":{"containers":[{"name":"c","image":"busybox"}],"restartPolicy":"Never"}}}}"#;

/// Accepts the patch of a Deployment, serves the CronJob and the Job to copy, and names the Job a
/// create makes; finds nothing else.
fn workload_answers(request: &RecordedRequest) -> (u16, String) {
    let path = request.path.as_str();
    match request.method.as_str() {
        "PATCH" if path.ends_with("/scale") => (200, SCALE_JSON.to_owned()),
        "PATCH" if path.contains("/deployments/") => (200, DEPLOYMENT_JSON.to_owned()),
        "GET" if path.ends_with("/cronjobs/reconcile") => (200, CRON_JOB_JSON.to_owned()),
        "GET" if path.ends_with("/jobs/etl") => (200, JOB_JSON.to_owned()),
        "POST" if path.ends_with("/jobs") => {
            let name = if request.body.contains("-rerun-") {
                "etl-rerun-k9d2z"
            } else {
                "reconcile-manual-x7k2p"
            };
            let body = format!(
                r#"{{"apiVersion":"batch/v1","kind":"Job","metadata":{{"name":"{name}","namespace":"team-a"}}}}"#
            );
            (201, body)
        }
        _ => (404, NOT_FOUND.to_owned()),
    }
}

fn workload_clusters(name: &str, cx: &mut TestAppContext) -> Clusters {
    workload_clusters_answering(name, workload_answers, cx)
}

/// Both clusters answer with `respond`: tests that read more objects than `workload_answers` serves.
fn workload_clusters_answering(
    name: &str,
    respond: fn(&RecordedRequest) -> (u16, String),
    cx: &mut TestAppContext,
) -> Clusters {
    // Dialogs open without their animation, so the palette field takes keys at once.
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

impl Clusters {
    /// Switches to `prod-a` and makes it live over a new fake server that answers like the
    /// default workload server. The old session is gone, so a test never has both clusters live.
    pub(super) fn activate_prod(&self, cx: &mut TestAppContext) -> FakeApi {
        self.activate_answering(&self.prod, "node-a", workload_answers, cx)
    }

    /// Shows `kind` and gives the open cluster the loaded `rows` of its own: `prod_rows` when
    /// `prod-a` is open, `stg_rows` when `stg-b` is.
    pub(super) fn show_kind(
        &self,
        kind: ResourceKind,
        prod_rows: Vec<KindRow>,
        stg_rows: Vec<KindRow>,
        cx: &mut TestAppContext,
    ) {
        self.fixture
            .shell
            .update(cx, |shell, cx| shell.show_screen(Screen::Kind(kind), cx));
        cx.run_until_parked();
        for (cluster, rows) in [(&self.prod, prod_rows), (&self.stg, stg_rows)] {
            let session = self
                .fixture
                .shell
                .read_with(cx, |shell, _| shell.session_of(cluster).cloned());
            if let Some(session) = session {
                session.update(cx, |session, cx| {
                    session.set_kind_rows_for_test(kind, rows, cx);
                });
            }
        }
        cx.run_until_parked();
        self.fixture.draw_twice(cx);
    }

    /// Puts the cursor on the object `name` of `kind` in `cluster`.
    pub(super) fn cursor_on(
        &self,
        cluster: &ClusterRef,
        kind: ResourceKind,
        name: &str,
        cx: &mut TestAppContext,
    ) {
        let key = ResourceKey::Kind {
            kind,
            namespace: Some("team-a".to_owned()),
            name: name.to_owned(),
        };
        let object = ClusterObject::new(cluster.clone(), key);
        self.fixture.shell.update(cx, |shell, cx| {
            shell.change_selection(Some(object), cx);
        });
        cx.run_until_parked();
    }

    pub(super) fn press(&self, keys: &str, cx: &mut TestAppContext) {
        self.fixture.press(keys, cx);
    }

    pub(super) fn dialog_label(&self, cx: &mut TestAppContext) -> String {
        let dialog = self.dialog(cx);
        dialog
            .read_with(cx, |dialog, _| dialog.label())
            .expect("a write dialog")
            .to_string()
    }
}

fn deployments(paused: bool) -> Vec<KindRow> {
    let mut summary = deployment("api");
    // Fully rolled out: a rollout in progress adds a box to the drawer, which would push the
    // Revisions out of the test window.
    (summary.ready, summary.available) = (summary.desired, summary.desired);
    summary.is_paused = paused;
    vec![deployment_row(&summary)]
}

fn restart_path() -> &'static str {
    "/apis/apps/v1/namespaces/team-a/deployments/api"
}

#[gpui_kit::test]
fn r_restarts_the_cursor_row(cx: &mut TestAppContext) {
    let t = workload_clusters("restart-key", cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployments(false),
        deployments(false),
        cx,
    );
    t.cursor_on(&t.stg, ResourceKind::Deployments, "api", cx);
    t.press("r", cx);
    assert_eq!(t.dialog_label(cx), "Restart rollout of deployment api");
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
    });
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].path, restart_path());
    assert!(sent[0].has_query("dryRun", "All"));
    assert!(sent[0].body.contains("kubectl.kubernetes.io/restartedAt"));
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
    let sent = writes(&t.stg_api);
    assert!(!sent[1].has_query_key("dryRun"));
    // The dry-run and the commit are one request, so the check says what the commit does.
    assert_eq!(sent[0].body, sent[1].body);
}

#[gpui_kit::test]
fn restart_in_production_types_the_workload_name(cx: &mut TestAppContext) {
    let t = workload_clusters("restart-cluster", cx);
    let prod_api = t.activate_prod(cx);
    t.set_lock(&t.prod, WriteLock::Unlocked, cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployments(false),
        deployments(false),
        cx,
    );
    t.cursor_on(&t.prod, ResourceKind::Deployments, "api", cx);
    t.press("r", cx);
    // The cluster is production: its tier asks for the name of the workload.
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "api".to_owned()
            }
        );
    });
    t.wait_for_dry_run(cx);
    assert_eq!(writes(&prod_api).len(), 1);
    assert!(writes(&t.stg_api).is_empty());
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&prod_api).len(), 1, "the name was not typed");
    t.type_name("api", cx);
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&prod_api).len() == 2);
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn r_on_a_paused_deployment_opens_no_dialog(cx: &mut TestAppContext) {
    let t = workload_clusters("restart-paused", cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployments(true),
        deployments(true),
        cx,
    );
    t.cursor_on(&t.stg, ResourceKind::Deployments, "api", cx);
    t.press("r", cx);
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn r_on_a_locked_production_row_opens_no_dialog(cx: &mut TestAppContext) {
    let t = workload_clusters("restart-locked", cx);
    let prod_api = t.activate_prod(cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployments(false),
        deployments(false),
        cx,
    );
    t.cursor_on(&t.prod, ResourceKind::Deployments, "api", cx);
    t.press("r", cx);
    assert!(!t.has_dialog(cx));
    assert!(writes(&prod_api).is_empty());
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn palette_runs_the_same_arm(cx: &mut TestAppContext) {
    let t = workload_clusters("restart-palette", cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployments(false),
        deployments(false),
        cx,
    );
    t.cursor_on(&t.stg, ResourceKind::Deployments, "api", cx);
    // The palette lists the action of the cursor row, enabled, and dispatches its key action.
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
                crate::palette_search::PaletteTarget::RowAction(RowAction::RestartRollout)
            )
        })
        .expect("the palette offers Restart rollout");
    assert!(entry.is_enabled());
    let action = RowAction::RestartRollout.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    assert_eq!(t.dialog_label(cx), "Restart rollout of deployment api");
    t.wait_for_dry_run(cx);
    let state = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.dry_run_state());
    assert!(
        matches!(state, Some(DryRunState::Passed { .. })),
        "{state:?}"
    );
}

#[gpui_kit::test]
fn pause_and_resume_send_booleans(cx: &mut TestAppContext) {
    let t = workload_clusters("pause-key", cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployments(false),
        deployments(false),
        cx,
    );
    t.cursor_on(&t.stg, ResourceKind::Deployments, "api", cx);
    let action = RowAction::PauseRollout.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    assert_eq!(t.dialog_label(cx), "Pause rollout of deployment api");
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent[0].path, restart_path());
    assert_eq!(sent[0].body, r#"{"spec":{"paused":true}}"#);
}

#[gpui_kit::test]
fn trigger_audit_names_the_created_job(cx: &mut TestAppContext) {
    let t = workload_clusters("trigger-audit", cx);
    let dir = t.enable_audit_folder("trigger-audit", cx);
    let rows = vec![cron_job_row(&cron_job("reconcile", "Forbid", 1))];
    t.show_kind(ResourceKind::CronJobs, Vec::new(), rows, cx);
    t.cursor_on(&t.stg, ResourceKind::CronJobs, "reconcile", cx);
    let action = RowAction::TriggerCronJob.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    assert_eq!(t.dialog_label(cx), "Trigger cronjob reconcile now");
    t.dialog(cx).read_with(cx, |dialog, _| {
        let lines: Vec<String> = dialog
            .warning_lines()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            [
                "1 job(s) of this CronJob are running; this run starts anyway",
                "While this run is active, scheduled runs are skipped (concurrency Forbid)"
            ]
        );
    });
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Trigger now");
    let fields = line["fields"].as_array().expect("fields");
    let value_of = |path: &str| {
        fields
            .iter()
            .find(|field| field["path"] == path)
            .map(|field| field["value"].clone())
    };
    assert_eq!(
        value_of("metadata.generateName"),
        Some("reconcile-manual-".into())
    );
    assert_eq!(
        value_of("metadata.name"),
        Some("reconcile-manual-x7k2p".into())
    );
    let posts: Vec<_> = writes(&t.stg_api)
        .into_iter()
        .filter(|request| request.method == "POST")
        .collect();
    assert_eq!(posts.len(), 2, "a dry-run and a commit: {posts:?}");
    assert_eq!(posts[0].path, "/apis/batch/v1/namespaces/team-a/jobs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn rerun_creates_a_standalone_job(cx: &mut TestAppContext) {
    let t = workload_clusters("rerun", cx);
    let dir = t.enable_audit_folder("rerun", cx);
    let rows = vec![job_row(&job("etl"))];
    t.show_kind(ResourceKind::Jobs, Vec::new(), rows, cx);
    t.cursor_on(&t.stg, ResourceKind::Jobs, "etl", cx);
    let action = RowAction::RerunJob.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    assert_eq!(t.dialog_label(cx), "Re-run job etl");
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Re-run");
    assert!(
        line["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .any(|field| field["path"] == "metadata.name" && field["value"] == "etl-rerun-k9d2z")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn menu_item_dispatches_the_key_on_the_right_clicked_row(cx: &mut TestAppContext) {
    let t = workload_clusters("menu-restart", cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployments(false),
        deployments(false),
        cx,
    );
    t.fixture
        .shell
        .update(cx, |shell, cx| shell.close_drawer(cx));
    t.fixture.draw_twice(cx);
    t.fixture
        .with_window(cx, |window, cx| window.right_click(("row", 0usize), cx));
    cx.run_until_parked();
    t.fixture.draw_twice(cx);
    cx.run_until_parked();
    let selected = t.fixture.shell.read_with(cx, |shell, _| {
        shell.selected.as_ref().map(|o| o.cluster.clone())
    });
    assert_eq!(selected.as_ref(), Some(&t.stg));
    // The clickable items before Restart rollout: the workload logs, View YAML, Port-forward, Scale….
    for key in ["down", "down", "down", "down", "down", "enter"] {
        t.press(key, cx);
        cx.run_until_parked();
    }
    assert_eq!(t.dialog_label(cx), "Restart rollout of deployment api");
    t.wait_for_dry_run(cx);
    assert_eq!(writes(&t.stg_api).len(), 1);
}

// ---- Scale: the popover and the palette ----

impl Clusters {
    pub(super) fn popover(&self, cx: &mut TestAppContext) -> Option<Entity<ValuePopover>> {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.value_popover().cloned())
    }

    /// The cursor on the staging Deployment, with the popover and the palette still closed.
    fn on_stg_deployment(&self, cx: &mut TestAppContext) {
        self.show_kind(
            ResourceKind::Deployments,
            deployments(false),
            deployments(false),
            cx,
        );
        self.cursor_on(&self.stg, ResourceKind::Deployments, "api", cx);
    }

    fn open_palette(&self, query: &str, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            self.fixture
                .shell
                .update(cx, |shell, cx| shell.open_palette(query, window, cx));
        });
        cx.run_until_parked();
        // The kit places its highlight while it draws.
        self.fixture.draw_twice(cx);
    }

    fn type_into_focus(&self, text: &str, cx: &mut TestAppContext) {
        self.fixture
            .with_window(cx, |window, cx| window.input(text, cx));
    }

    pub(super) fn type_in_popover(
        &self,
        popover: &Entity<ValuePopover>,
        text: &str,
        cx: &mut TestAppContext,
    ) {
        self.fixture.with_window(cx, |window, cx| {
            popover.update(cx, |popover, cx| popover.type_text(text, window, cx));
        });
    }
}

#[gpui_kit::test]
fn shift_s_opens_the_popover(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-key", cx);
    t.on_stg_deployment(cx);
    t.press("shift-s", cx);
    assert!(t.popover(cx).is_some());
    assert!(!t.has_dialog(cx));
}

#[gpui_kit::test]
fn palette_enter_on_scale_falls_back_to_the_popover(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-palette", cx);
    t.on_stg_deployment(cx);
    t.open_palette("> scale", cx);
    t.press("enter", cx);
    assert!(t.popover(cx).is_some());
}

#[gpui_kit::test]
fn menu_scale_opens_the_popover_for_the_clicked_row(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-menu", cx);
    t.on_stg_deployment(cx);
    t.fixture
        .shell
        .update(cx, |shell, cx| shell.close_drawer(cx));
    t.fixture.draw_twice(cx);
    t.fixture
        .with_window(cx, |window, cx| window.right_click(("row", 0usize), cx));
    cx.run_until_parked();
    t.fixture.draw_twice(cx);
    cx.run_until_parked();
    // The clickable items before Scale…: the workload logs, View YAML, and Port-forward.
    for key in ["down", "down", "down", "down", "enter"] {
        t.press(key, cx);
        cx.run_until_parked();
    }
    let popover = t.popover(cx).expect("the popover is open");
    let subject = t
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.selected.clone());
    assert_eq!(subject.map(|object| object.cluster), Some(t.stg.clone()));
    assert_eq!(
        popover.read_with(cx, |popover, cx| popover.typed_text(cx)),
        "3"
    );
}

#[gpui_kit::test]
fn scale_button_disabled_until_a_new_whole_number(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-button", cx);
    t.on_stg_deployment(cx);
    t.press("shift-s", cx);
    let popover = t.popover(cx).expect("the popover is open");
    // It opens on the replicas the Deployment has now, which is no change.
    assert_eq!(
        popover.read_with(cx, |popover, cx| popover.typed_text(cx)),
        "3"
    );
    assert!(!popover.read_with(cx, |popover, cx| popover.can_submit(cx)));
    let cases = [
        ("5", true),
        ("0", true),
        ("3", false),
        ("", false),
        ("-1", false),
        ("2.5", false),
    ];
    for (text, is_enabled) in cases {
        t.type_in_popover(&popover, text, cx);
        let enabled = popover.read_with(cx, |popover, cx| popover.can_submit(cx));
        assert_eq!(enabled, is_enabled, "{text:?}");
    }
    // A press with nothing to send opens no dialog and sends nothing.
    t.type_in_popover(&popover, "3", cx);
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.press_submit(window, cx));
    });
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_popover_follows_the_count_the_row_has_now(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-count-moved", cx);
    t.on_stg_deployment(cx);
    t.press("shift-s", cx);
    let popover = t.popover(cx).expect("the popover is open");
    assert_eq!(
        popover.read_with(cx, |popover, cx| popover.state_line(cx)),
        "Now 3 desired · 3 ready"
    );
    // The list moved on to 5 while the form was open: 5 is no change now, 3 is.
    let mut moved = deployment("api");
    moved.desired = 5;
    t.show_kind(
        ResourceKind::Deployments,
        deployments(false),
        vec![deployment_row(&moved)],
        cx,
    );
    assert_eq!(
        popover.read_with(cx, |popover, cx| popover.state_line(cx)),
        "Now 5 desired · 2 ready"
    );
    t.type_in_popover(&popover, "5", cx);
    assert!(!popover.read_with(cx, |popover, cx| popover.can_submit(cx)));
    t.type_in_popover(&popover, "3", cx);
    assert!(popover.read_with(cx, |popover, cx| popover.can_submit(cx)));
}

#[gpui_kit::test]
fn the_popover_warns_while_the_number_is_typed(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-warn", cx);
    t.on_stg_deployment(cx);
    t.press("shift-s", cx);
    let popover = t.popover(cx).expect("the popover is open");
    let lines = |text: &str, cx: &mut TestAppContext| {
        t.type_in_popover(&popover, text, cx);
        popover.read_with(cx, |popover, cx| {
            let lines = popover.warning_lines(cx);
            lines.iter().map(ToString::to_string).collect::<Vec<_>>()
        })
    };
    assert_eq!(lines("1", cx), ["Scaling down from 3 to 1"]);
    assert!(lines("6", cx).is_empty());
}

#[gpui_kit::test]
fn enter_is_a_key_trigger(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-enter", cx);
    t.on_stg_deployment(cx);
    t.press("shift-s", cx);
    let popover = t.popover(cx).expect("the popover is open");
    t.type_in_popover(&popover, "5", cx);
    t.fixture.draw_twice(cx);
    // The field has the focus, so Enter sends the number: the confirm dialog opens.
    t.press("enter", cx);
    assert_eq!(t.dialog_label(cx), "Scale deployment api from 3 to 5");
    assert!(t.popover(cx).is_none(), "the popover closes on its way out");
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(
        sent[0].path,
        "/apis/apps/v1/namespaces/team-a/deployments/api/scale"
    );
    assert!(sent[0].has_query("dryRun", "All"));
    assert_eq!(sent[0].body, r#"{"spec":{"replicas":5}}"#);
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
}

#[gpui_kit::test]
fn scale_to_zero_warns_in_the_dialog(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-zero", cx);
    t.on_stg_deployment(cx);
    t.press("shift-s", cx);
    let popover = t.popover(cx).expect("the popover is open");
    t.type_in_popover(&popover, "0", cx);
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.press_submit(window, cx));
    });
    assert_eq!(t.dialog_label(cx), "Scale deployment api from 3 to 0");
    let lines: Vec<String> = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.warning_lines())
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(lines, ["Scaling down from 3 to 0"]);
}

#[gpui_kit::test]
fn esc_closes_the_popover(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-esc", cx);
    t.on_stg_deployment(cx);
    t.press("shift-s", cx);
    t.fixture.draw_twice(cx);
    assert!(t.popover(cx).is_some());
    t.press("escape", cx);
    assert!(t.popover(cx).is_none());
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_popover_closes_when_the_cursor_leaves_its_row(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-cursor", cx);
    t.on_stg_deployment(cx);
    t.press("shift-s", cx);
    assert!(t.popover(cx).is_some());
    t.fixture
        .shell
        .update(cx, |shell, cx| shell.clear_selection(cx));
    cx.run_until_parked();
    assert!(t.popover(cx).is_none());
}

#[gpui_kit::test]
fn drawer_has_no_replicas_input(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-drawer", cx);
    t.on_stg_deployment(cx);
    t.fixture
        .shell
        .update(cx, |shell, cx| shell.set_drawer_open(true, cx));
    t.fixture.draw_twice(cx);
    // Only the popover has the Scale button: the drawer is for reading (W7 note 4).
    assert!(!t.fixture.is_drawn("value-submit", cx));
    assert!(t.popover(cx).is_none());
}

impl Clusters {
    pub(super) fn notification_count(&self, cx: &mut TestAppContext) -> usize {
        self.fixture
            .with_window(cx, |window, cx| window.notifications(cx).len())
    }
}

#[gpui_kit::test]
fn a_scale_for_a_row_that_left_the_list_is_refused_with_a_notice(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-row-gone", cx);
    t.on_stg_deployment(cx);
    let subject = t
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.selected.clone())
        .expect("the cursor is on a row");
    let target =
        crate::workload_actions::ScaleTarget::of(&KindObject::Deployment(deployment("api")), &[])
            .expect("a Deployment can scale");
    // The row is deleted while the form is open.
    t.show_kind(ResourceKind::Deployments, Vec::new(), Vec::new(), cx);
    let before = t.notification_count(cx);
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.submit_scale(&subject, &target, 5, window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(t.notification_count(cx), before + 1);
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn palette_ctrl_enter_argument_mode(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-argument", cx);
    t.on_stg_deployment(cx);
    t.open_palette("> scale", cx);
    t.press("secondary-enter", cx);
    t.fixture.draw_twice(cx);
    t.type_into_focus("5", cx);
    t.press("enter", cx);
    cx.run_until_parked();
    // The palette is gone, no popover opened, and the confirm dialog names the change.
    assert!(t.popover(cx).is_none());
    assert_eq!(t.dialog_label(cx), "Scale deployment api from 3 to 5");
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent[0].body, r#"{"spec":{"replicas":5}}"#);
}

#[gpui_kit::test]
fn palette_rejects_non_numbers(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-argument-bad", cx);
    t.on_stg_deployment(cx);
    t.open_palette("> scale", cx);
    t.press("secondary-enter", cx);
    t.fixture.draw_twice(cx);
    // Letters cannot be typed, so the field stays empty and Enter finds no number.
    t.type_into_focus("abc", cx);
    t.press("enter", cx);
    cx.run_until_parked();
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
    // The palette is still open on the field: a number now goes through.
    t.type_into_focus("2", cx);
    t.press("enter", cx);
    cx.run_until_parked();
    assert_eq!(t.dialog_label(cx), "Scale deployment api from 3 to 2");
}

#[gpui_kit::test]
fn only_a_plain_enter_submits_the_palette_argument(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-argument-modified-enter", cx);
    t.on_stg_deployment(cx);
    t.open_palette("> scale", cx);
    t.press("secondary-enter", cx);
    t.fixture.draw_twice(cx);
    t.type_into_focus("5", cx);
    // The kit Input turns these into its Enter event; neither may start the scale.
    for key in ["shift-enter", "secondary-enter"] {
        t.press(key, cx);
        cx.run_until_parked();
        assert!(!t.has_dialog(cx), "{key}");
        assert!(writes(&t.stg_api).is_empty(), "{key}");
    }
    t.press("enter", cx);
    cx.run_until_parked();
    assert_eq!(t.dialog_label(cx), "Scale deployment api from 3 to 5");
}

#[gpui_kit::test]
fn esc_returns_to_the_list(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-argument-esc", cx);
    t.on_stg_deployment(cx);
    t.open_palette("> scale", cx);
    t.press("secondary-enter", cx);
    t.fixture.draw_twice(cx);
    t.press("escape", cx);
    cx.run_until_parked();
    // Back on the list, which is still open: Enter confirms the Scale entry (the popover).
    assert!(t.popover(cx).is_none());
    t.press("enter", cx);
    cx.run_until_parked();
    assert!(t.popover(cx).is_some());
}

#[gpui_kit::test]
fn ctrl_enter_does_nothing_on_a_disabled_scale_entry(cx: &mut TestAppContext) {
    // The cluster is locked: its Scale entry is disabled with its reason, so no field opens.
    let t = workload_clusters("scale-argument-locked", cx);
    let prod_api = t.activate_prod(cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployments(false),
        deployments(false),
        cx,
    );
    t.cursor_on(&t.prod, ResourceKind::Deployments, "api", cx);
    t.open_palette("> scale", cx);
    t.press("secondary-enter", cx);
    t.fixture.draw_twice(cx);
    t.type_into_focus("5", cx);
    t.press("enter", cx);
    cx.run_until_parked();
    assert!(!t.has_dialog(cx));
    assert!(t.popover(cx).is_none());
    assert!(writes(&prod_api).is_empty());
}

// ---- Roll back: the drawer buttons, the menu, the palette ----

const ROLL_BACK_DEPLOYMENT_JSON: &str = r#"{"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"api","namespace":"team-a","uid":"dep-uid"},"spec":{"selector":{"matchLabels":{"app":"api"}},"template":{"metadata":{"labels":{"app":"api"}},"spec":{"containers":[{"name":"c","image":"api:2.14.0"}]}}}}"#;
const ROLL_BACK_REPLICA_SET_JSON: &str = r#"{"apiVersion":"apps/v1","kind":"ReplicaSet","metadata":{"name":"api-6c1e2a","namespace":"team-a","annotations":{"deployment.kubernetes.io/revision":"6"},"ownerReferences":[{"apiVersion":"apps/v1","kind":"Deployment","name":"api","uid":"dep-uid","controller":true}]},"spec":{"selector":{"matchLabels":{"app":"api"}},"template":{"metadata":{"labels":{"app":"api","pod-template-hash":"6c1e2a"}},"spec":{"containers":[{"name":"c","image":"api:2.13.4"}]}}}}"#;

/// `workload_answers`, plus the Deployment and the ReplicaSet a roll back reads.
fn roll_back_answers(request: &RecordedRequest) -> (u16, String) {
    let path = request.path.as_str();
    match request.method.as_str() {
        "GET" if path.ends_with("/deployments/api") => (200, ROLL_BACK_DEPLOYMENT_JSON.to_owned()),
        "GET" if path.ends_with("/replicasets/api-6c1e2a") => {
            (200, ROLL_BACK_REPLICA_SET_JSON.to_owned())
        }
        _ => workload_answers(request),
    }
}

fn revision_sets() -> Vec<cluster::ReplicaSetSummary> {
    use crate::workload_actions::workload_actions_tests::replica_set;
    vec![
        replica_set("api-7d", Some("7"), Some("api"), "api:2.14.0"),
        replica_set("api-6c1e2a", Some("6"), Some("api"), "api:2.13.4"),
        replica_set("api-5b", Some("5"), Some("api"), "api:2.12.0"),
    ]
}

impl Clusters {
    /// The staging Deployment `api` with its drawer open and its three revisions loaded.
    fn with_revisions(&self, cx: &mut TestAppContext) {
        self.on_stg_deployment(cx);
        self.fixture
            .shell
            .update(cx, |shell, cx| shell.set_drawer_open(true, cx));
        // The drawer's related watch starts after the subject has rested; the test clock is moved.
        cx.executor().advance_clock(
            crate::drawer::DRAWER_SUBJECT_DELAY + std::time::Duration::from_millis(50),
        );
        cx.run_until_parked();
        self.wait_for("the related watch", cx, |cx| {
            self.fixture.shell.read_with(cx, |shell, cx| {
                shell
                    .live_of(&self.stg, cx)
                    .is_some_and(|live| live.related_subject().is_some())
            })
        });
        let session = session_of(&self.fixture, &self.stg, cx);
        session.update(cx, |session, cx| {
            session.set_replica_sets_for_test(revision_sets(), cx);
        });
        cx.run_until_parked();
        self.fixture.draw_twice(cx);
    }
}

#[gpui_kit::test]
fn revision_roll_back_opens_the_confirm_dialog(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("rollback-button", roll_back_answers, cx);
    t.with_revisions(cx);
    // Newest first: rev 7 is current and has no button, rev 6 is the second row.
    t.fixture
        .with_window(cx, |window, cx| window.click(("roll-back", 1usize), cx));
    cx.run_until_parked();
    assert_eq!(
        t.dialog_label(cx),
        "Roll back deployment api to rev 6 (2.13.4)"
    );
    t.wait_for_dry_run(cx);
    let state = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.dry_run_state());
    assert!(
        matches!(state, Some(DryRunState::Passed { .. })),
        "{state:?}"
    );
    let patches: Vec<_> = writes(&t.stg_api)
        .into_iter()
        .filter(|request| request.method == "PATCH")
        .collect();
    assert_eq!(patches.len(), 1, "{patches:?}");
    assert_eq!(
        patches[0].content_type.as_deref(),
        Some("application/json-patch+json")
    );
    assert!(patches[0].has_query("dryRun", "All"));
    assert!(patches[0].body.contains(r#""path":"/metadata/uid""#));
    assert!(patches[0].body.contains(r#""path":"/spec/template""#));
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| {
        writes(&t.stg_api)
            .iter()
            .filter(|request| request.method == "PATCH")
            .count()
            == 2
    });
    // The button did not also open the ReplicaSet behind the row.
    let subject = t
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.selected.clone());
    assert_eq!(
        subject.map(|object| object.key),
        Some(ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("team-a".to_owned()),
            name: "api".to_owned(),
        })
    );
}

#[gpui_kit::test]
fn a_locked_cluster_has_no_roll_back_button(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("rollback-locked", roll_back_answers, cx);
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    t.with_revisions(cx);
    t.fixture
        .with_window(cx, |window, cx| window.click(("roll-back", 1usize), cx));
    cx.run_until_parked();
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn menu_roll_back_scrolls_to_revisions(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("rollback-menu", roll_back_answers, cx);
    t.with_revisions(cx);
    // The drawer shows another tab; Roll back… brings it to the Overview, where the revisions are.
    t.fixture.shell.update(cx, |shell, _| {
        shell.drawer.tab = crate::drawer::DrawerTab::Events;
    });
    // The ⋯ menu and the palette both dispatch this key action.
    let action = RowAction::RollBack.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    t.fixture.draw_twice(cx);
    let (is_open, tab, pending) = t.fixture.shell.read_with(cx, |shell, _| {
        (
            shell.drawer.is_open,
            shell.drawer.tab,
            shell.drawer.reveal_section.get(),
        )
    });
    assert!(is_open);
    assert_eq!(tab, crate::drawer::DrawerTab::Overview);
    // The paint took the request, so the drawer scrolls once.
    assert_eq!(pending, None);
    assert!(!t.has_dialog(cx));
    // `scroll_to_top_of_item` counts the direct children of the scrolled box: the sections must
    // be those, or the request has nothing to scroll to (UX round 3, M7).
    let sections = t
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.drawer.scroll.children_count());
    assert!(sections > 1, "the drawer sections are nested: {sections}");
}

#[gpui_kit::test]
fn roll_back_opens_the_revisions_before_they_are_loaded(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("rollback-unloaded", roll_back_answers, cx);
    t.on_stg_deployment(cx);
    let snapshot = t
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.palette_snapshot(&parse_query(""), cx));
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.label.as_ref() == "Roll back")
        .expect("the palette lists Roll back");
    assert!(matches!(
        entry.state,
        crate::palette_search::EntryState::Enabled
    ));
    // The key opens the drawer on its Revisions, which starts loading them; no dialog yet.
    let action = RowAction::RollBack.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    let (is_open, tab) = t
        .fixture
        .shell
        .read_with(cx, |shell, _| (shell.drawer.is_open, shell.drawer.tab));
    assert!(is_open);
    assert_eq!(tab, crate::drawer::DrawerTab::Overview);
    assert!(!t.has_dialog(cx));
}

#[gpui_kit::test]
fn palette_roll_back_entry_runs_the_dialog(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("rollback-palette", roll_back_answers, cx);
    t.with_revisions(cx);
    let snapshot = t
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.palette_snapshot(&parse_query(""), cx));
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.label.as_ref() == "Roll back to rev 6")
        .expect("the palette offers the previous revision");
    assert!(entry.is_enabled());
    t.open_palette("> roll back", cx);
    t.press("enter", cx);
    cx.run_until_parked();
    assert_eq!(
        t.dialog_label(cx),
        "Roll back deployment api to rev 6 (2.13.4)"
    );
}

// ---- Batch: the selection bar and the list dialog ----

const BATCH_NAMES: [&str; 4] = ["api", "web", "worker", "cron"];

fn deployment_rows(names: &[&str]) -> Vec<KindRow> {
    names
        .iter()
        .map(|name| deployment_row(&deployment(name)))
        .collect()
}

/// `workload_answers`, except that the server refuses `web`: its dry-run when `on_dry_run`, else
/// its commit.
fn refusing_web(request: &RecordedRequest, on_dry_run: bool) -> (u16, String) {
    let is_web = request.path.ends_with("/deployments/web");
    let is_dry_run = request.has_query_key("dryRun");
    if request.method == "PATCH" && is_web && is_dry_run == on_dry_run {
        let body = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"nope","reason":"Forbidden","code":403}"#;
        return (403, body.to_owned());
    }
    workload_answers(request)
}

fn web_dry_run_refused(request: &RecordedRequest) -> (u16, String) {
    refusing_web(request, true)
}

fn web_commit_refused(request: &RecordedRequest) -> (u16, String) {
    refusing_web(request, false)
}

impl Clusters {
    /// The four staging Deployments listed, with the table rows `rows` ticked.
    fn with_ticked_deployments(&self, rows: &[usize], cx: &mut TestAppContext) {
        self.show_kind(
            ResourceKind::Deployments,
            Vec::new(),
            deployment_rows(&BATCH_NAMES),
            cx,
        );
        self.tick(rows, cx);
    }

    pub(super) fn tick(&self, rows: &[usize], cx: &mut TestAppContext) {
        for &row in rows {
            self.fixture.shell.update(cx, |shell, cx| {
                shell.check_rows(RowCheck::Toggle(row), cx);
            });
        }
        cx.run_until_parked();
        self.fixture.draw_twice(cx);
    }

    pub(super) fn bulk_buttons(&self, cx: &mut TestAppContext) -> Vec<BulkButton> {
        self.fixture
            .shell
            .read_with(cx, |shell, cx| shell.bulk_buttons(cx))
    }

    /// Presses the bulk button named `label`.
    pub(super) fn press_bulk(&self, label: &'static str, cx: &mut TestAppContext) {
        self.fixture
            .with_window(cx, |window, cx| window.click(label, cx));
        cx.run_until_parked();
    }

    pub(super) fn items(&self, cx: &mut TestAppContext) -> Vec<ItemProgress> {
        self.dialog(cx)
            .read_with(cx, |dialog, _| dialog.item_states())
    }

    pub(super) fn state_of(buttons: &[BulkButton], label: &str) -> BulkState {
        buttons
            .iter()
            .find(|button| button.label.as_ref() == label)
            .map(|button| button.state.clone())
            .expect("the button is on the bar")
    }
}

#[gpui_kit::test]
fn bulk_buttons_follow_the_ticks_and_the_gate(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-buttons", cx);
    t.with_ticked_deployments(&[0, 1], cx);
    let buttons = t.bulk_buttons(cx);
    let labels: Vec<&str> = buttons.iter().map(|button| button.label.as_ref()).collect();
    assert_eq!(
        labels,
        ["Scale…", "Restart rollout", "Roll back…", "Delete…"]
    );
    assert_eq!(
        Clusters::state_of(&buttons, "Scale…"),
        BulkState::Ready(ResourceAction::Scale(cluster::ObjectKind::Deployment))
    );
    assert_eq!(
        Clusters::state_of(&buttons, "Restart rollout"),
        BulkState::Ready(ResourceAction::RestartRollout(
            cluster::ObjectKind::Deployment
        ))
    );
    // Each Deployment needs its own revision choice, so Roll back has no bulk form.
    assert_eq!(
        Clusters::state_of(&buttons, "Roll back…"),
        BulkState::Off("Roll back one deployment at a time".into())
    );
}

#[gpui_kit::test]
fn bulk_buttons_are_off_for_a_locked_cluster(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-locked", cx);
    t.activate_prod(cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployment_rows(&BATCH_NAMES),
        Vec::new(),
        cx,
    );
    t.tick(&[0, 1], cx);
    let buttons = t.bulk_buttons(cx);
    for label in ["Scale…", "Restart rollout"] {
        assert_eq!(
            Clusters::state_of(&buttons, label),
            BulkState::Off("prod-a is read-only".into()),
            "{label}"
        );
    }
}

#[gpui_kit::test]
fn more_than_fifty_ticks_turn_every_button_off(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-cap", cx);
    let names: Vec<String> = (0..=MAX_BATCH_ITEMS).map(|n| format!("d-{n:02}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    t.show_kind(
        ResourceKind::Deployments,
        Vec::new(),
        deployment_rows(&names),
        cx,
    );
    t.fixture
        .shell
        .update(cx, |shell, cx| shell.check_rows(RowCheck::All(true), cx));
    cx.run_until_parked();
    let buttons = t.bulk_buttons(cx);
    for label in ["Scale…", "Restart rollout"] {
        assert_eq!(
            Clusters::state_of(&buttons, label),
            BulkState::Off("Select at most 50 rows".into()),
            "{label}"
        );
    }
}

fn restart_deployments() -> ResourceAction {
    ResourceAction::RestartRollout(cluster::ObjectKind::Deployment)
}

impl Clusters {
    fn is_batch_running(&self, cluster: &ClusterRef, cx: &mut TestAppContext) -> bool {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.running_batches.contains(cluster))
    }

    /// Starts a second batch from the keys' own entry while the first one commits.
    fn run_bulk_now(&self, action: ResourceAction, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            self.fixture
                .shell
                .update(cx, |shell, cx| shell.run_bulk(action, window, cx));
        });
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn a_running_batch_turns_the_bulk_buttons_off_and_refuses_a_second_batch(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-in-flight", cx);
    t.with_ticked_deployments(&[0, 1, 2, 3], cx);
    t.press_bulk("Restart rollout", cx);
    t.wait_for_dry_run(cx);
    assert!(!t.is_batch_running(&t.stg, cx));
    t.confirm(cx);
    // The flag is set the moment the commit starts, before any answer came back.
    assert!(t.is_batch_running(&t.stg, cx));
    let buttons = t.bulk_buttons(cx);
    for label in ["Scale…", "Restart rollout"] {
        assert_eq!(
            Clusters::state_of(&buttons, label),
            BulkState::Off("A batch is running".into()),
            "{label}"
        );
    }
    // A stale button or a key cannot start another one on the same objects.
    t.run_bulk_now(restart_deployments(), cx);
    t.wait_for("the batch to end", cx, |cx| !t.is_batch_running(&t.stg, cx));
    // Four dry-runs and four commits: the refused second batch sent nothing.
    assert_eq!(writes(&t.stg_api).len(), 8);
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Restart rollout"),
        BulkState::Ready(restart_deployments())
    );
}

#[gpui_kit::test]
fn a_failed_item_still_ends_the_running_batch(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("bulk-in-flight-failed", web_commit_refused, cx);
    t.with_ticked_deployments(&[0, 1, 2, 3], cx);
    t.press_bulk("Restart rollout", cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    assert!(t.is_batch_running(&t.stg, cx));
    t.wait_for("the batch to end", cx, |cx| !t.is_batch_running(&t.stg, cx));
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Restart rollout"),
        BulkState::Ready(restart_deployments())
    );
}

#[gpui_kit::test]
fn a_batch_in_another_cluster_does_not_block_this_one(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-in-flight-other", cx);
    t.with_ticked_deployments(&[0, 1], cx);
    t.fixture
        .shell
        .update(cx, |shell, _| shell.running_batches.insert(t.prod.clone()));
    assert_eq!(
        Clusters::state_of(&t.bulk_buttons(cx), "Restart rollout"),
        BulkState::Ready(restart_deployments())
    );
}

#[gpui_kit::test]
fn batch_dry_runs_are_sequential_and_unaudited(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-restart", cx);
    let dir = t.enable_audit_folder("bulk-restart", cx);
    t.with_ticked_deployments(&[0, 1, 2, 3], cx);
    t.press_bulk("Restart rollout", cx);
    assert_eq!(t.dialog_label(cx), "Restart 4 deployments");
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(dialog.confirm_text().as_deref(), Some("Restart 4"));
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
    });
    t.wait_for_dry_run(cx);
    assert_eq!(t.items(cx), vec![ItemProgress::Passed; 4]);
    // One request per item, in list order, none audited.
    let sent = writes(&t.stg_api);
    let paths: Vec<&str> = sent.iter().map(|request| request.path.as_str()).collect();
    let expected: Vec<String> = BATCH_NAMES
        .iter()
        .map(|name| format!("/apis/apps/v1/namespaces/team-a/deployments/{name}"))
        .collect();
    assert_eq!(
        paths,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(
        sent.iter()
            .all(|request| request.has_query("dryRun", "All"))
    );
    assert!(audit_lines(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn batch_audits_each_commit(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-audit", cx);
    let dir = t.enable_audit_folder("bulk-audit", cx);
    t.with_ticked_deployments(&[0, 1, 2, 3], cx);
    t.press_bulk("Restart rollout", cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("four audit lines", cx, |_| audit_lines(&dir).len() == 4);
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
    assert_eq!(objects, BATCH_NAMES);
    assert!(lines.iter().all(|line| line["action"] == "Restart"));
    assert!(lines.iter().all(|line| line["outcome"] == "applied"));
    assert!(lines.iter().all(|line| line["cluster"] == "stg-b"));
    // Four dry-runs and four commits, each commit a request of its own.
    let commits = writes(&t.stg_api)
        .into_iter()
        .filter(|request| !request.has_query_key("dryRun"))
        .count();
    assert_eq!(commits, 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn batch_apply_needs_every_dry_run_to_pass(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("bulk-dry-fail", web_dry_run_refused, cx);
    t.with_ticked_deployments(&[0, 1, 2, 3], cx);
    t.press_bulk("Restart rollout", cx);
    t.wait_for_dry_run(cx);
    let items = t.items(cx);
    assert_eq!(items[0], ItemProgress::Passed);
    assert!(matches!(items[1], ItemProgress::Rejected(_)), "{items:?}");
    assert_eq!(items[2], ItemProgress::Passed);
    let state = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.dry_run_state());
    let Some(DryRunState::Failed(text)) = state else {
        panic!("the dry-run line should fail: {state:?}");
    };
    assert!(text.starts_with("Dry-run failed for 1 of 4:"), "{text}");
    assert!(t.block(cx).is_some());
    // A press sends nothing: three passed items do not make a partial apply.
    let before = writes(&t.stg_api).len();
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), before);
}

#[gpui_kit::test]
fn batch_continues_after_a_failed_commit(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("bulk-commit-fail", web_commit_refused, cx);
    let dir = t.enable_audit_folder("bulk-commit-fail", cx);
    t.with_ticked_deployments(&[0, 1, 2, 3], cx);
    t.press_bulk("Restart rollout", cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("four audit lines", cx, |_| audit_lines(&dir).len() == 4);
    let outcomes: Vec<String> = audit_lines(&dir)
        .iter()
        .map(|line| line["outcome"].as_str().unwrap_or_default().to_owned())
        .collect();
    // The refused item is the second; the ones after it still went.
    assert_eq!(outcomes, ["applied", "failed", "applied", "applied"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn batch_in_production_types_the_cluster_name(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-prod", cx);
    let prod_api = t.activate_prod(cx);
    t.set_lock(&t.prod, WriteLock::Unlocked, cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployment_rows(&BATCH_NAMES),
        Vec::new(),
        cx,
    );
    t.tick(&[0, 1], cx);
    t.press_bulk("Restart rollout", cx);
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        );
        assert_eq!(dialog.environment(), &Environment::PRODUCTION);
    });
    t.wait_for_dry_run(cx);
    assert_eq!(t.block(cx).as_deref(), Some("Type prod-a to confirm"));
    let before = writes(&prod_api).len();
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&prod_api).len(), before, "the name was not typed");
    t.type_name("prod-a", cx);
    t.confirm(cx);
    t.wait_for("two commits", cx, |_| {
        writes(&prod_api)
            .iter()
            .filter(|request| !request.has_query_key("dryRun"))
            .count()
            == 2
    });
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_lock_after_the_dry_runs_blocks_the_apply(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-locked-late", cx);
    t.with_ticked_deployments(&[0, 1], cx);
    t.press_bulk("Restart rollout", cx);
    t.wait_for_dry_run(cx);
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    assert_eq!(
        t.block(cx).as_deref(),
        Some("stg-b was locked; nothing was changed")
    );
    let before = writes(&t.stg_api).len();
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), before);
}

#[gpui_kit::test]
fn bulk_popover_starts_empty_and_scales_every_ticked_row(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-scale", cx);
    t.with_ticked_deployments(&[0, 1, 2], cx);
    t.press_bulk("Scale…", cx);
    let popover = t.popover(cx).expect("the bulk popover opens");
    assert_eq!(
        popover.read_with(cx, |popover, cx| popover.typed_text(cx)),
        ""
    );
    assert!(!popover.read_with(cx, |popover, cx| popover.can_submit(cx)));
    // One count for all: there is no "unchanged" for rows with different counts.
    t.type_in_popover(&popover, "3", cx);
    assert!(popover.read_with(cx, |popover, cx| popover.can_submit(cx)));
    t.type_in_popover(&popover, "2", cx);
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.press_submit(window, cx));
    });
    assert!(t.popover(cx).is_none());
    assert_eq!(t.dialog_label(cx), "Scale 3 deployments to 2");
    let lines: Vec<String> = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.warning_lines())
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(lines, ["Scaling down 3 of 3"]);
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 3);
    assert!(
        sent.iter()
            .all(|request| request.body == r#"{"spec":{"replicas":2}}"#)
    );
}

#[gpui_kit::test]
fn the_suspend_button_reads_resume_when_every_ticked_cronjob_is_suspended(cx: &mut TestAppContext) {
    let t = workload_clusters("bulk-resume", cx);
    let mut suspended = cron_job("reconcile", "Allow", 0);
    suspended.is_suspended = true;
    let mut other = cron_job("sweep", "Allow", 0);
    other.is_suspended = true;
    let rows = vec![cron_job_row(&suspended), cron_job_row(&other)];
    t.show_kind(ResourceKind::CronJobs, Vec::new(), rows, cx);
    t.tick(&[0, 1], cx);
    let labels: Vec<String> = t
        .bulk_buttons(cx)
        .iter()
        .map(|button| button.label.to_string())
        .collect();
    assert_eq!(labels, ["Trigger now", "Resume", "Delete…"]);
    // One running CronJob in the selection turns the label back.
    let running = cron_job("sweep", "Allow", 0);
    let rows = vec![cron_job_row(&suspended), cron_job_row(&running)];
    t.show_kind(ResourceKind::CronJobs, Vec::new(), rows, cx);
    t.tick(&[0], cx);
    t.tick(&[1], cx);
    let labels: Vec<String> = t
        .bulk_buttons(cx)
        .iter()
        .map(|button| button.label.to_string())
        .collect();
    assert_eq!(labels, ["Trigger now", "Suspend", "Delete…"]);
}

#[gpui_kit::test]
fn menu_shift_s_and_palette_open_the_same_popover(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-roads", cx);
    t.on_stg_deployment(cx);
    // Each road ends in the popover of the staging row, on the count it has now.
    let mut opened = Vec::new();
    // Shift S.
    t.press("shift-s", cx);
    opened.push(t.popover(cx).is_some());
    t.press("escape", cx);
    // The palette: a plain Enter on the Scale entry.
    t.open_palette("> scale", cx);
    t.press("enter", cx);
    opened.push(t.popover(cx).is_some());
    t.press("escape", cx);
    // The menu: the workload logs, View YAML, Port-forward, then Scale….
    t.fixture
        .shell
        .update(cx, |shell, cx| shell.close_drawer(cx));
    t.fixture.draw_twice(cx);
    t.fixture
        .with_window(cx, |window, cx| window.right_click(("row", 0usize), cx));
    cx.run_until_parked();
    t.fixture.draw_twice(cx);
    for key in ["down", "down", "down", "down", "enter"] {
        t.press(key, cx);
        cx.run_until_parked();
    }
    opened.push(t.popover(cx).is_some());
    assert_eq!(opened, [true, true, true]);
    let popover = t.popover(cx).expect("the popover is open");
    assert_eq!(
        popover.read_with(cx, |popover, cx| popover.typed_text(cx)),
        "3"
    );
}

// ---- Spec 0039 step 2: L opens workload logs ----

impl Clusters {
    /// Gives the stg cluster one pod owned by `controller` (a kind and a name) in `team-a`.
    fn seed_controlled_pod(&self, controller: (&str, &str), cx: &mut TestAppContext) {
        let mut pod = super::app_shell_tests::logs_pod();
        pod.namespace = "team-a".to_owned();
        pod.controller = Some(cluster::ControllerRef {
            kind: controller.0.to_owned(),
            name: controller.1.to_owned(),
        });
        let session = session_of(&self.fixture, &self.stg, cx);
        session.update(cx, |session, cx| session.set_pods_for_test(vec![pod], cx));
        cx.run_until_parked();
    }

    fn press_view_logs(&self, cx: &mut TestAppContext) {
        let action = RowAction::ViewLogs.key_action();
        self.fixture
            .with_window(cx, |window, cx| window.dispatch_action(action, cx));
        cx.run_until_parked();
    }

    fn log_tab_labels(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.fixture
            .shell
            .read_with(cx, |shell, cx| shell.dock.read(cx).log_tab_labels(cx))
    }
}

#[gpui_kit::test]
fn l_on_cron_job_opens_last_job_logs(cx: &mut TestAppContext) {
    let t = workload_clusters("l-cron-job", cx);
    let mut reconcile = cron_job("reconcile", "Forbid", 0);
    reconcile.last_schedule_at = jiff::Timestamp::from_second(29_000_000 * 60).ok();
    t.show_kind(
        ResourceKind::CronJobs,
        Vec::new(),
        vec![cron_job_row(&reconcile)],
        cx,
    );
    t.seed_controlled_pod(("Job", "reconcile-29000000"), cx);
    t.cursor_on(&t.stg, ResourceKind::CronJobs, "reconcile", cx);
    t.press_view_logs(cx);
    assert_eq!(t.log_tab_labels(cx), ["job/reconcile-29000000"]);
}

#[gpui_kit::test]
fn l_on_deployment_opens_workload_logs(cx: &mut TestAppContext) {
    let t = workload_clusters("l-deployment", cx);
    t.show_kind(
        ResourceKind::Deployments,
        Vec::new(),
        deployments(false),
        cx,
    );
    t.cursor_on(&t.stg, ResourceKind::Deployments, "api", cx);
    t.press_view_logs(cx);
    assert_eq!(t.log_tab_labels(cx), ["deploy/api"]);
}

// ---- Palette pairs: an action on a search hit ----

/// `api` and `web` in `team-a`, loaded and shown, with the cursor on `web`.
fn pair_fixture(name: &str, cx: &mut TestAppContext) -> Clusters {
    let t = workload_clusters(name, cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployment_rows(&["api", "web"]),
        deployment_rows(&["api", "web"]),
        cx,
    );
    t.cursor_on(&t.stg, ResourceKind::Deployments, "web", cx);
    t
}

fn deployment_object(t: &Clusters, name: &str) -> ClusterObject {
    ClusterObject::new(
        t.stg.clone(),
        ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("team-a".to_owned()),
            name: name.to_owned(),
        },
    )
}

fn run_pair(t: &Clusters, object: ClusterObject, row: RowAction, cx: &mut TestAppContext) {
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.run_row_action_on(object, row, window, cx);
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn palette_pair_reveals_then_opens_the_restart_dialog(cx: &mut TestAppContext) {
    let t = pair_fixture("pair-restart", cx);
    let api = deployment_object(&t, "api");
    run_pair(&t, api.clone(), RowAction::RestartRollout, cx);
    t.fixture.shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected.as_ref(), Some(&api));
        assert!(shell.drawer.is_open);
    });
    // The key's own flow: the 0030 confirm of the cluster's tier, naming the hit.
    t.wait_for("the dialog", cx, |cx| t.has_dialog(cx));
    assert_eq!(t.dialog_label(cx), "Restart rollout of deployment api");
    t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
    });
    t.wait_for_dry_run(cx);
    // Only the dry-run went out; nothing is committed before Confirm.
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].has_query("dryRun", "All"));
    assert_eq!(sent[0].path, restart_path());
}

#[gpui_kit::test]
fn palette_pair_on_the_cursor_row_runs_at_once(cx: &mut TestAppContext) {
    let t = pair_fixture("pair-cursor", cx);
    run_pair(
        &t,
        deployment_object(&t, "web"),
        RowAction::RestartRollout,
        cx,
    );
    t.wait_for("the dialog", cx, |cx| t.has_dialog(cx));
    assert_eq!(t.dialog_label(cx), "Restart rollout of deployment web");
}

#[gpui_kit::test]
fn palette_pair_on_a_vanished_row_runs_nothing(cx: &mut TestAppContext) {
    let t = pair_fixture("pair-vanished", cx);
    let ghost = deployment_object(&t, "ghost");
    run_pair(&t, ghost.clone(), RowAction::RestartRollout, cx);
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
    t.fixture.shell.read_with(cx, |shell, _| {
        assert_ne!(shell.selected.as_ref(), Some(&ghost));
    });
}

#[gpui_kit::test]
fn palette_pair_rereads_the_gate_when_it_runs(cx: &mut TestAppContext) {
    // The cluster is locked after the pair was listed: the key's own gate says no.
    let t = workload_clusters("pair-gate", cx);
    let _prod_api = t.activate_prod(cx);
    t.show_kind(
        ResourceKind::Deployments,
        deployment_rows(&["api", "web"]),
        deployment_rows(&["api", "web"]),
        cx,
    );
    let api = ClusterObject::new(
        t.prod.clone(),
        ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("team-a".to_owned()),
            name: "api".to_owned(),
        },
    );
    // `prod-a` is locked at open, so the key refuses the restart.
    run_pair(&t, api, RowAction::RestartRollout, cx);
    assert!(!t.has_dialog(cx));
}

#[gpui_kit::test]
fn confirming_a_pair_in_the_palette_opens_the_dialog_and_sends_nothing(cx: &mut TestAppContext) {
    let t = pair_fixture("pair-palette", cx);
    t.open_palette("> rest api", cx);
    t.press("enter", cx);
    cx.run_until_parked();
    t.wait_for("the dialog", cx, |cx| t.has_dialog(cx));
    assert_eq!(t.dialog_label(cx), "Restart rollout of deployment api");
    t.wait_for_dry_run(cx);
    assert!(
        writes(&t.stg_api)
            .iter()
            .all(|sent| sent.has_query_key("dryRun"))
    );
}

#[gpui_kit::test]
fn a_held_enter_never_confirms_a_palette_pair(cx: &mut TestAppContext) {
    let t = pair_fixture("pair-held-enter", cx);
    t.open_palette("> rest api", cx);
    let held = KeyDownEvent {
        keystroke: Keystroke::parse("enter").expect("a valid keystroke"),
        is_held: true,
        prefer_character_input: false,
    };
    t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(held.to_platform_input(), cx);
    });
    cx.run_until_parked();
    // Nothing opened and the palette is still there; the cursor did not move.
    let is_palette_open = t
        .fixture
        .with_window(cx, |window, cx| window.has_active_dialog(cx));
    assert!(
        is_palette_open && !t.has_dialog(cx),
        "the palette stays open"
    );
    t.fixture.shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(deployment_object(&t, "web")));
    });
    assert!(writes(&t.stg_api).is_empty());
    // A fresh Enter on the same entry does confirm it.
    t.press("enter", cx);
    cx.run_until_parked();
    t.wait_for("the confirm dialog", cx, |cx| {
        t.has_dialog(cx) && t.dialog_label(cx) == "Restart rollout of deployment api"
    });
}

// ---- Restart consumers (Edit values notice, Used by Restart) ----

fn consumer(kind: &str, name: &str) -> ResourceKey {
    ResourceKey::of_object(kind, Some("team-a"), name).expect("a valid key")
}

#[gpui_kit::test]
fn restart_consumers_opens_one_batch_dialog_per_workload_kind(cx: &mut TestAppContext) {
    // No Deployments or StatefulSets list is loaded: the consumers are named by their pods.
    let t = workload_clusters("restart-consumers", cx);
    let consumers = [
        (ObjectKind::Deployment, consumer("Deployment", "api")),
        (ObjectKind::StatefulSet, consumer("StatefulSet", "db")),
        (ObjectKind::Deployment, consumer("Deployment", "web")),
    ];
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.restart_consumers(&t.stg, &consumers, "web-config", window, cx);
        });
    });
    // The first kind opens last, so it is the one on top.
    assert_eq!(
        t.dialog_label(cx),
        "Restart 2 deployments that read web-config"
    );
    t.wait_for("both dry-runs", cx, |_| writes(&t.stg_api).len() == 3);
    let paths: Vec<String> = writes(&t.stg_api)
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert!(
        paths.iter().any(|path| path.ends_with("/statefulsets/db")),
        "{paths:?}"
    );
}

#[gpui_kit::test]
fn a_used_by_restart_opens_the_consumer_confirm_of_the_edited_object(cx: &mut TestAppContext) {
    // No Deployments list is loaded: the consumer is named by the Used by row, and the confirm is
    // the toast's, which names the ConfigMap the drawer shows.
    let t = workload_clusters("restart-named", cx);
    let subject = ClusterObject::new(t.stg.clone(), consumer("Deployment", "api"));
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            let source = consumer("ConfigMap", "web-config");
            shell.selected = Some(ClusterObject::new(t.stg.clone(), source));
            shell.drawer.is_open = true;
            shell.restart_used_by(&subject, ObjectKind::Deployment, window, cx);
        });
    });
    assert_eq!(
        t.dialog_label(cx),
        "Restart 1 deployment that reads web-config"
    );
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].path, restart_path());
}

// ---- Roll back from the revision diff dialog ----

/// The dialog of the drawer's Deployment `api`: rev 6 against the current rev 7.
fn open_revision_dialog(t: &Clusters, cx: &mut TestAppContext) {
    use crate::revision_diff::{RevisionSide, diff_request};
    let side = |replica_set: &str, revision: u64, tag: &str, is_current: bool| RevisionSide {
        replica_set: replica_set.to_owned(),
        revision: Some(revision),
        tag: Some(tag.to_owned()),
        is_current,
        created_at: None,
    };
    let request = diff_request(
        ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("team-a".to_owned()),
            name: "api".to_owned(),
        },
        side("api-6c1e2a", 6, "2.13.4", false),
        side("api-7d", 7, "2.14.0", true),
    );
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.open_revision_diff(request, window, cx)
        });
    });
    cx.run_until_parked();
    t.fixture.draw_twice(cx);
}

#[gpui_kit::test]
fn the_diff_dialog_rolls_back_to_the_compared_revision(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("rollback-diff-dialog", roll_back_answers, cx);
    t.with_revisions(cx);
    open_revision_dialog(&t, cx);
    t.fixture.with_window(cx, |window, cx| {
        window.click("revision-diff-roll-back", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        t.dialog_label(cx),
        "Roll back deployment api to rev 6 (2.13.4)"
    );
}

#[gpui_kit::test]
fn a_locked_cluster_leaves_the_diff_dialog_roll_back_disabled(cx: &mut TestAppContext) {
    let t = workload_clusters_answering("rollback-diff-locked", roll_back_answers, cx);
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    t.with_revisions(cx);
    open_revision_dialog(&t, cx);
    t.fixture.with_window(cx, |window, cx| {
        window.click("revision-diff-roll-back", cx);
    });
    cx.run_until_parked();
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn r_on_a_ticked_row_restarts_the_ticked_set(cx: &mut TestAppContext) {
    let t = workload_clusters("restart-key-ticked", cx);
    t.with_ticked_deployments(&[0, 1], cx);
    t.cursor_on(&t.stg, ResourceKind::Deployments, "api", cx);
    t.press("r", cx);
    assert_eq!(t.dialog_label(cx), "Restart 2 deployments");
    t.wait_for_dry_run(cx);
    assert_eq!(t.items(cx).len(), 2);
    assert_eq!(writes(&t.stg_api).len(), 2);
}

#[gpui_kit::test]
fn r_acts_on_the_cursor_row_when_it_is_not_ticked(cx: &mut TestAppContext) {
    let t = workload_clusters("restart-key-unticked", cx);
    t.with_ticked_deployments(&[0, 1], cx);
    t.cursor_on(&t.stg, ResourceKind::Deployments, "worker", cx);
    t.press("r", cx);
    assert_eq!(t.dialog_label(cx), "Restart rollout of deployment worker");
}

#[gpui_kit::test]
fn shift_s_on_a_ticked_row_scales_the_ticked_set(cx: &mut TestAppContext) {
    let t = workload_clusters("scale-key-ticked", cx);
    t.with_ticked_deployments(&[0, 1], cx);
    t.cursor_on(&t.stg, ResourceKind::Deployments, "api", cx);
    t.press("shift-s", cx);
    let popover = t.popover(cx).expect("the bulk popover opens");
    t.type_in_popover(&popover, "2", cx);
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.press_submit(window, cx));
    });
    assert_eq!(t.dialog_label(cx), "Scale 2 deployments to 2");
}

// ---- Set image ----

fn deployments_with_images() -> Vec<KindRow> {
    let container = |name: &str, image: &str| cluster::TemplateContainer {
        name: name.to_owned(),
        image: image.to_owned(),
        ports: Vec::new(),
    };
    let mut summary = deployment("api");
    summary.containers = vec![
        container("web", "repo.example.com/library/nginx:1.27-alpine"),
        container("sidecar", "busybox:1.36"),
    ];
    vec![deployment_row(&summary)]
}

impl Clusters {
    fn on_stg_deployment_with_images(&self, cx: &mut TestAppContext) {
        self.show_kind(
            ResourceKind::Deployments,
            deployments_with_images(),
            deployments_with_images(),
            cx,
        );
        self.cursor_on(&self.stg, ResourceKind::Deployments, "api", cx);
    }
}

#[gpui_kit::test]
fn i_opens_the_image_popover_with_the_tag_selected(cx: &mut TestAppContext) {
    let t = workload_clusters("set-image-key", cx);
    t.on_stg_deployment_with_images(cx);
    t.press("i", cx);
    let popover = t.popover(cx).expect("the popover is open");
    assert!(!t.has_dialog(cx));
    popover.read_with(cx, |popover, cx| {
        assert_eq!(popover.title_text(), "Set image of deployment/api");
        assert_eq!(
            popover.typed_text(cx),
            "repo.example.com/library/nginx:1.27-alpine"
        );
        assert_eq!(
            popover.selected_image_text(cx).as_deref(),
            Some("1.27-alpine")
        );
        // It opens on the image the container has now, which is no change.
        assert!(!popover.can_submit(cx));
    });
}

#[gpui_kit::test]
fn the_container_buttons_show_the_image_of_each_container(cx: &mut TestAppContext) {
    let t = workload_clusters("set-image-pick", cx);
    t.on_stg_deployment_with_images(cx);
    t.press("i", cx);
    let popover = t.popover(cx).expect("the popover is open");
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.press_container(1, window, cx));
    });
    popover.read_with(cx, |popover, cx| {
        assert_eq!(popover.typed_text(cx), "busybox:1.36");
        assert_eq!(popover.selected_image_text(cx).as_deref(), Some("1.36"));
    });
}

#[gpui_kit::test]
fn set_image_confirms_a_strategic_patch_with_the_cause(cx: &mut TestAppContext) {
    let t = workload_clusters("set-image-commit", cx);
    t.on_stg_deployment_with_images(cx);
    t.press("i", cx);
    let popover = t.popover(cx).expect("the popover is open");
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| {
            popover.type_image("repo.example.com/library/nginx:1.26-alpine", window, cx);
            popover.type_cause("release test", window, cx);
        });
    });
    assert!(popover.read_with(cx, |popover, cx| popover.can_submit(cx)));
    t.fixture.with_window(cx, |window, cx| {
        popover.update(cx, |popover, cx| popover.press_submit(window, cx));
    });
    assert!(t.popover(cx).is_none(), "the popover closes");
    assert_eq!(t.dialog_label(cx), "Set image of deployment api");
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].path, restart_path());
    assert!(sent[0].has_query("dryRun", "All"));
    assert!(
        sent[0]
            .body
            .contains("repo.example.com/library/nginx:1.26-alpine")
    );
    assert!(sent[0].body.contains("kubernetes.io/change-cause"));
    assert!(sent[0].body.contains("release test"));
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
    assert!(!writes(&t.stg_api)[1].has_query_key("dryRun"));
}

#[gpui_kit::test]
fn i_on_a_row_without_a_pod_template_does_nothing(cx: &mut TestAppContext) {
    let t = workload_clusters("set-image-job", cx);
    t.show_kind(
        ResourceKind::Jobs,
        vec![job_row(&job("etl"))],
        vec![job_row(&job("etl"))],
        cx,
    );
    t.cursor_on(&t.stg, ResourceKind::Jobs, "etl", cx);
    t.press("i", cx);
    assert!(t.popover(cx).is_none());
    assert!(!t.has_dialog(cx));
}
