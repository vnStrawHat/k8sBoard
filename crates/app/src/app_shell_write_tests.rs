//! The write flow, the lock, and the confirm dialog in a headless window over two viewed clusters:
//! `prod-a` (the primary, locked at open) and `stg-b` (unlocked). Each cluster answers from its own
//! fake API server, so a test sees which cluster a request reached and nothing leaves the machine.

use std::path::PathBuf;
use std::time::Duration;

use cluster::fake_api::{FakeApi, RecordedRequest};
use cluster::{
    AccessCheck, AccessDecision, AccessReport, AccessReview, NodeReadiness, NodeScheduling,
    NodeStatus, NodeSummary, NodeSystemInfo, WritePolicy,
};
use gpui_kit::InputEvent as _;
use gpui_kit::{Entity, KeyDownEvent, Keystroke, TestAppContext};

use super::app_shell_switch_tests::{SwitchFixture, open_switch_fixture};
use super::write_flow::DryRunState;
use super::*;
use crate::cluster_session::AccessState;
use crate::confirm_dialog::ConfirmDialog;
use crate::environment::Environment;
use crate::resource_actions::ResourceAction;
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};
use crate::write_guard::{DialogConfirm, WriteLock};

const NODE_JSON: &str = r#"{"apiVersion":"v1","kind":"Node","metadata":{"name":"n"}}"#;
const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;

fn node(name: &str, scheduling: NodeScheduling) -> NodeSummary {
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

/// A report that allows every check.
pub(super) fn allowed() -> AccessState {
    let reviews = AccessCheck::ALL
        .into_iter()
        .map(|check| AccessReview {
            check,
            decision: AccessDecision::Allowed,
        })
        .collect();
    AccessState::Known(AccessReport { reviews })
}

/// The writes a fake server received. A live session also sends GETs (its watches) and the POSTs of
/// the access reviews, which change nothing.
pub(super) fn writes(api: &FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| {
            request.method != "GET" && !request.path.ends_with("/selfsubjectaccessreviews")
        })
        .collect()
}

pub(super) struct Clusters {
    pub(super) fixture: SwitchFixture,
    pub(super) prod_api: FakeApi,
    pub(super) stg_api: FakeApi,
    pub(super) prod: ClusterRef,
    pub(super) stg: ClusterRef,
}

pub(super) fn slot_session(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    cx: &mut TestAppContext,
) -> Entity<ClusterSession> {
    fixture
        .shell
        .read_with(cx, |shell, _| shell.slot_session(cluster).cloned())
        .expect("a viewed slot")
}

pub(super) fn view(fixture: &SwitchFixture, contexts: &[&str], cx: &mut TestAppContext) {
    let wanted: Vec<ClusterRef> = contexts
        .iter()
        .map(|context| fixture.cluster(context, cx))
        .collect();
    fixture
        .shell
        .update(cx, |shell, cx| shell.view_clusters(&wanted, cx));
    cx.run_until_parked();
}

/// Makes `cluster` live over a fake server that accepts patches of nodes, with every permission
/// granted and one node, `node`.
fn go_live_fake(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    node_name: &str,
    cx: &mut TestAppContext,
) -> FakeApi {
    go_live_answering(fixture, cluster, node_name, accept_patches, cx)
}

/// The default fake server: it accepts a patch and finds nothing else.
fn accept_patches(request: &RecordedRequest) -> (u16, String) {
    if request.method == "PATCH" {
        (200, NODE_JSON.to_owned())
    } else {
        (404, NOT_FOUND.to_owned())
    }
}

/// `go_live_fake` over a server that answers with `respond`.
pub(super) fn go_live_answering(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    node_name: &str,
    respond: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static,
    cx: &mut TestAppContext,
) -> FakeApi {
    // The client's worker is a tokio task, so it must be built inside the runtime.
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        FakeApi::connection(WritePolicy::Allowed, respond)
    };
    let session = slot_session(fixture, cluster, cx);
    let node_name = node_name.to_owned();
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
        session.set_access_for_test(allowed(), cx);
        session.set_nodes_for_test(vec![node(&node_name, NodeScheduling::Enabled)], cx);
    });
    cx.run_until_parked();
    api
}

pub(super) fn two_clusters(name: &str, cx: &mut TestAppContext) -> Clusters {
    let fixture = open_switch_fixture(name, cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let prod_api = go_live_fake(&fixture, &prod, "node-a", cx);
    let stg_api = go_live_fake(&fixture, &stg, "node-b", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Nodes, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    Clusters {
        fixture,
        prod_api,
        stg_api,
        prod,
        stg,
    }
}

impl Clusters {
    pub(super) fn lock_of(&self, cluster: &ClusterRef, cx: &mut TestAppContext) -> WriteLock {
        slot_session(&self.fixture, cluster, cx).read_with(cx, |session, _| session.lock())
    }

    pub(super) fn set_lock(&self, cluster: &ClusterRef, lock: WriteLock, cx: &mut TestAppContext) {
        let session = slot_session(&self.fixture, cluster, cx);
        session.update(cx, |session, cx| session.set_lock(lock, cx));
        cx.run_until_parked();
    }

    pub(super) fn generation_of(&self, cluster: &ClusterRef, cx: &mut TestAppContext) -> u64 {
        self.fixture.shell.read_with(cx, |shell, cx| {
            shell
                .guard_for(cluster, cx)
                .expect("a live guard")
                .generation
        })
    }

    pub(super) fn cordon(&self, cluster: &ClusterRef, node_name: &str, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            self.fixture.shell.update(cx, |shell, cx| {
                shell.start_cordon(cluster, node_name, None, window, cx)
            });
        });
    }

    pub(super) fn dialog(&self, cx: &mut TestAppContext) -> Entity<ConfirmDialog> {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.last_dialog.clone())
            .and_then(|dialog| dialog.upgrade())
            .expect("a confirm dialog is open")
    }

    pub(super) fn has_dialog(&self, cx: &mut TestAppContext) -> bool {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.last_dialog.clone())
            .is_some_and(|dialog| dialog.upgrade().is_some())
    }

    pub(super) fn wait_for(
        &self,
        what: &str,
        cx: &mut TestAppContext,
        done: impl Fn(&mut TestAppContext) -> bool,
    ) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    pub(super) fn wait_for_dry_run(&self, cx: &mut TestAppContext) {
        self.wait_for("the dry-run", cx, |cx| {
            let dialog = self.dialog(cx);
            !matches!(
                dialog.read_with(cx, |dialog, _| dialog.dry_run_state()),
                Some(DryRunState::Running)
            )
        });
    }

    pub(super) fn confirm(&self, cx: &mut TestAppContext) {
        let dialog = self.dialog(cx);
        self.fixture.with_window(cx, |window, cx| {
            dialog.update(cx, |dialog, cx| dialog.press_confirm(window, cx));
        });
    }

    pub(super) fn type_name(&self, text: &str, cx: &mut TestAppContext) {
        let dialog = self.dialog(cx);
        self.fixture.with_window(cx, |window, cx| {
            dialog.update(cx, |dialog, cx| dialog.type_text(text, window, cx));
        });
    }

    pub(super) fn block(&self, cx: &mut TestAppContext) -> Option<String> {
        let dialog = self.dialog(cx);
        dialog.read_with(cx, |dialog, cx| {
            dialog.block_reason(cx).map(|reason| reason.to_string())
        })
    }

    pub(super) fn toggle(&self, cluster: &ClusterRef, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            self.fixture
                .shell
                .update(cx, |shell, cx| shell.toggle_write_lock(cluster, window, cx));
        });
    }

    /// Settings that save into `dir`, so the audit log has a folder.
    pub(super) fn enable_audit_folder(&self, name: &str, cx: &mut TestAppContext) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("k8sboard-0030-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the temp dir");
        cx.update(|cx| {
            AppSettings::install(
                LoadedSettings {
                    settings: Settings::default(),
                    writes: WriteMode::Enabled(dir.clone()),
                    notice: None,
                },
                cx,
            );
        });
        dir
    }
}

pub(super) fn audit_lines(dir: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(crate::audit_log::audit_path(dir))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON line"))
        .collect()
}

fn key_down(key: &str, is_held: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(key).expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

fn press_key_event(clusters: &Clusters, event: KeyDownEvent, cx: &mut TestAppContext) {
    clusters.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(event.to_platform_input(), cx);
    });
}

// ---- Cordon on the row's own cluster ----

#[gpui_kit::test]
fn cordon_on_a_staging_row_uses_that_clusters_connection_guard_and_tier(cx: &mut TestAppContext) {
    let t = two_clusters("cordon-stg", cx);
    // The primary is `prod-a`: locked, and typing its name would confirm there.
    assert_eq!(t.lock_of(&t.prod, cx), WriteLock::Locked);
    assert_eq!(t.lock_of(&t.stg, cx), WriteLock::Unlocked);
    let stg_generation = t.generation_of(&t.stg, cx);
    t.cordon(&t.stg, "node-b", cx);
    let dialog = t.dialog(cx);
    // The guard and the tier are stg-b's own: a click on STG, never the primary's typed name.
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        assert_eq!(dialog.environment(), Environment::Staging);
        assert_eq!(dialog.generation(), stg_generation);
    });
    t.wait_for_dry_run(cx);
    let sent = writes(&t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].path, "/api/v1/nodes/node-b");
    assert!(sent[0].has_query("dryRun", "All"));
    assert!(t.block(cx).is_none(), "{:?}", t.block(cx));
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
    let sent = writes(&t.stg_api);
    assert!(!sent[1].has_query_key("dryRun"));
    assert!(sent[1].has_query("fieldManager", "k8sboard"));
    // Nothing reached the primary.
    assert!(writes(&t.prod_api).is_empty(), "{:?}", writes(&t.prod_api));
}

#[gpui_kit::test]
fn cordon_on_a_locked_production_row_opens_no_dialog(cx: &mut TestAppContext) {
    let t = two_clusters("cordon-locked", cx);
    t.cordon(&t.prod, "node-a", cx);
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.prod_api).is_empty());
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn type_name_tier_needs_the_match(cx: &mut TestAppContext) {
    let t = two_clusters("cordon-prod", cx);
    t.set_lock(&t.prod, WriteLock::Unlocked, cx);
    t.cordon(&t.prod, "node-a", cx);
    let dialog = t.dialog(cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        );
        assert_eq!(dialog.environment(), Environment::Production);
    });
    t.wait_for_dry_run(cx);
    assert_eq!(t.block(cx).as_deref(), Some("Type prod-a to confirm"));
    // A press without the name sends nothing.
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.prod_api).len(), 1);
    t.type_name("prod-b", cx);
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.prod_api).len(), 1);
    t.type_name("  prod-a ", cx);
    assert!(t.block(cx).is_none());
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.prod_api).len() == 2);
    assert!(writes(&t.stg_api).is_empty());
}

#[gpui_kit::test]
fn commit_rechecks_the_row_cluster_lock(cx: &mut TestAppContext) {
    let t = two_clusters("lock-after", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    assert_eq!(
        t.block(cx).as_deref(),
        Some("stg-b was locked; nothing was changed")
    );
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), 1, "only the dry-run was sent");
}

#[gpui_kit::test]
fn checked_write_runs_commit_block_before_commit(cx: &mut TestAppContext) {
    let t = two_clusters("leaves-view", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    view(&t.fixture, &["prod-a"], cx);
    assert_eq!(
        t.block(cx).as_deref(),
        Some("stg-b is no longer open; nothing was changed")
    );
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), 1);
}

#[gpui_kit::test]
fn the_debug_policy_blocks_a_write_at_the_dry_run(cx: &mut TestAppContext) {
    // A connection whose policy is Blocked, as every debug build has without the opt-in.
    let t = two_clusters("blocked-policy", cx);
    let (connection, api) = {
        let _guard = t.fixture.runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, |_| (200, NODE_JSON.to_owned()))
    };
    let session = slot_session(&t.fixture, &t.stg, cx);
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
        session.set_access_for_test(allowed(), cx);
        session.set_nodes_for_test(vec![node("node-b", NodeScheduling::Enabled)], cx);
    });
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    let state = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.dry_run_state());
    assert!(
        matches!(&state, Some(DryRunState::Failed(text)) if text.contains("writes are blocked")),
        "{state:?}"
    );
    assert!(t.block(cx).is_some());
    assert!(writes(&api).is_empty());
}

#[gpui_kit::test]
fn a_cordoned_node_is_uncordoned(cx: &mut TestAppContext) {
    let t = two_clusters("uncordon", cx);
    let session = slot_session(&t.fixture, &t.stg, cx);
    session.update(cx, |session, cx| {
        session.set_nodes_for_test(vec![node("node-b", NodeScheduling::Disabled)], cx);
    });
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
    let sent = writes(&t.stg_api);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&sent[1].body).expect("JSON"),
        serde_json::json!({ "spec": { "unschedulable": false } })
    );
}

// ---- Enter ----

#[gpui_kit::test]
fn enter_confirms_the_focused_button(cx: &mut TestAppContext) {
    let t = two_clusters("enter-fresh", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    t.fixture.draw_twice(cx);
    press_key_event(&t, key_down("enter", false), cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
}

#[gpui_kit::test]
fn held_enter_does_not_confirm(cx: &mut TestAppContext) {
    let t = two_clusters("enter-held", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    t.fixture.draw_twice(cx);
    for _ in 0..3 {
        press_key_event(&t, key_down("enter", true), cx);
    }
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), 1, "only the dry-run was sent");
}

// ---- The lock ----

#[gpui_kit::test]
fn a_session_opens_in_its_profiles_lock_state(cx: &mut TestAppContext) {
    let t = two_clusters("open-state", cx);
    assert_eq!(t.lock_of(&t.prod, cx), WriteLock::Locked);
    assert_eq!(t.lock_of(&t.stg, cx), WriteLock::Unlocked);
}

#[gpui_kit::test]
fn lock_toggle_appends_a_line(cx: &mut TestAppContext) {
    let t = two_clusters("lock-line", cx);
    let dir = t.enable_audit_folder("lock-line", cx);
    t.toggle(&t.stg, cx);
    assert_eq!(t.lock_of(&t.stg, cx), WriteLock::Locked);
    assert!(!t.has_dialog(cx));
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let lines = audit_lines(&dir);
    assert_eq!(lines[0]["action"], "Lock");
    assert_eq!(lines[0]["cluster"], "stg-b");
    assert!(lines[0].get("object").is_none());
    // The toggle is for the session only.
    assert!(!dir.join("settings.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn unlocking_prod_asks_for_the_typed_name(cx: &mut TestAppContext) {
    let t = two_clusters("unlock-prod", cx);
    let dir = t.enable_audit_folder("unlock-prod", cx);
    t.toggle(&t.prod, cx);
    // Nothing changes until the dialog is confirmed.
    assert_eq!(t.lock_of(&t.prod, cx), WriteLock::Locked);
    let dialog = t.dialog(cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        );
        assert_eq!(dialog.dry_run_state(), None);
    });
    assert_eq!(t.block(cx).as_deref(), Some("Type prod-a to confirm"));
    t.confirm(cx);
    assert_eq!(t.lock_of(&t.prod, cx), WriteLock::Locked);
    t.type_name("prod-a", cx);
    t.confirm(cx);
    assert_eq!(t.lock_of(&t.prod, cx), WriteLock::Unlocked);
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    assert_eq!(audit_lines(&dir)[0]["action"], "Unlock");
    assert_eq!(audit_lines(&dir)[0]["cluster"], "prod-a");
    assert!(writes(&t.prod_api).is_empty(), "an unlock sends nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn unlocking_non_prod_asks_for_a_click(cx: &mut TestAppContext) {
    let t = two_clusters("unlock-stg", cx);
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    t.toggle(&t.stg, cx);
    let dialog = t.dialog(cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click)
    });
    assert!(t.block(cx).is_none());
    t.confirm(cx);
    assert_eq!(t.lock_of(&t.stg, cx), WriteLock::Unlocked);
    assert_eq!(
        t.lock_of(&t.prod, cx),
        WriteLock::Locked,
        "the other cluster keeps its lock"
    );
}

#[gpui_kit::test]
fn the_lock_chord_acts_on_the_cursor_cluster(cx: &mut TestAppContext) {
    let t = two_clusters("chord-cursor", cx);
    // Without a cursor the chord means the primary.
    assert_eq!(
        t.fixture
            .shell
            .read_with(cx, |shell, _| shell.lock_target()),
        Some(t.prod.clone())
    );
    let table = t
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.node_table.clone());
    for row in 0..2 {
        cx.update(|cx| table.update(cx, |table, cx| table.set_selected_row(row, cx)));
        cx.run_until_parked();
        let target = t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.lock_target());
        let cursor = t.fixture.shell.read_with(cx, |shell, _| {
            shell.selected.as_ref().map(|object| object.cluster.clone())
        });
        assert_eq!(target, cursor);
    }
    // The last row is stg-b's node: the chord locks stg-b and leaves the primary alone.
    t.fixture.press("secondary-shift-r", cx);
    let target = t
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.lock_target());
    assert_eq!(target, Some(t.stg.clone()));
    assert_eq!(t.lock_of(&t.stg, cx), WriteLock::Locked);
    assert_eq!(t.lock_of(&t.prod, cx), WriteLock::Locked);
}

#[gpui_kit::test]
fn the_badge_is_drawn_with_a_session(cx: &mut TestAppContext) {
    let t = two_clusters("badge", cx);
    assert!(t.fixture.is_drawn("write-lock", cx));
}

// ---- Failures after the commit ----

fn refuses_commits_with(
    code: u16,
    reason: &'static str,
) -> impl Fn(&RecordedRequest) -> (u16, String) {
    move |request| {
        if request.method != "PATCH" {
            return (404, NOT_FOUND.to_owned());
        }
        if request.has_query_key("dryRun") {
            return (200, NODE_JSON.to_owned());
        }
        let status = serde_json::json!({
            "kind": "Status", "apiVersion": "v1", "status": "Failure",
            "message": "the server said no", "reason": reason, "code": code,
        });
        (code, status.to_string())
    }
}

#[gpui_kit::test]
fn a_conflict_keeps_the_dialog_with_a_retry_that_checks_again(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("conflict", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let prod_api = go_live_fake(&fixture, &prod, "node-a", cx);
    let stg_api = go_live_answering(
        &fixture,
        &stg,
        "node-b",
        refuses_commits_with(409, "Conflict"),
        cx,
    );
    let t = Clusters {
        fixture,
        prod_api,
        stg_api,
        prod,
        stg,
    };
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    let dialog = t.dialog(cx);
    t.wait_for("the failure", cx, |cx| {
        let state = dialog.read_with(cx, |dialog, _| dialog.dry_run_state());
        matches!(state, Some(DryRunState::Failed(_)))
    });
    let state = dialog.read_with(cx, |dialog, _| dialog.dry_run_state());
    assert_eq!(
        state,
        Some(DryRunState::Failed(
            "The object changed since the check: the server said no".into()
        ))
    );
    assert_eq!(writes(&t.stg_api).len(), 2);
    // The check is stale: the commit stays blocked until Retry has checked again.
    assert!(t.block(cx).is_some());
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(
        writes(&t.stg_api).len(),
        2,
        "no second commit without a fresh dry-run"
    );
    // Retry runs the dry-run again, which lifts the block.
    t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.press_retry(window, cx))
    });
    t.wait_for("the second dry-run", cx, |_| writes(&t.stg_api).len() == 3);
    assert!(writes(&t.stg_api)[2].has_query("dryRun", "All"));
    t.wait_for_dry_run(cx);
    assert_eq!(t.block(cx), None);
    assert!(writes(&t.prod_api).is_empty());
}

#[gpui_kit::test]
fn a_refused_commit_with_no_retry_closes_the_dialog(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("forbidden", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let prod_api = go_live_fake(&fixture, &prod, "node-a", cx);
    let stg_api = go_live_answering(
        &fixture,
        &stg,
        "node-b",
        refuses_commits_with(404, "NotFound"),
        cx,
    );
    let t = Clusters {
        fixture,
        prod_api,
        stg_api,
        prod,
        stg,
    };
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| writes(&t.stg_api).len() == 2);
    t.wait_for("the dialog to close", cx, |cx| !t.has_dialog(cx));
}

#[gpui_kit::test]
fn a_cordon_commit_appends_an_audit_line_of_the_target_cluster(cx: &mut TestAppContext) {
    let t = two_clusters("audit-cordon", cx);
    let dir = t.enable_audit_folder("audit-cordon", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    // A dry-run is not recorded.
    assert!(audit_lines(&dir).is_empty());
    t.confirm(cx);
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["cluster"], "stg-b");
    assert_eq!(line["context"], "stg-b");
    assert_eq!(line["action"], "Cordon");
    assert_eq!(line["outcome"], "applied");
    assert_eq!(line["object"]["kind"], "Node");
    assert_eq!(line["object"]["name"], "node-b");
    assert_eq!(line["fields"][0]["path"], "spec.unschedulable");
    assert_eq!(line["fields"][0]["value"], "true");
    assert!(!line.to_string().contains("merge-patch"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- The read-only user of the UAT cluster: denied, nothing sent ----

#[gpui_kit::test]
fn cordon_without_patch_nodes_is_not_permitted_and_sends_nothing(cx: &mut TestAppContext) {
    let t = two_clusters("not-permitted", cx);
    t.set_lock(&t.stg, WriteLock::Unlocked, cx);
    let reviews = AccessCheck::ALL
        .into_iter()
        .map(|check| AccessReview {
            check,
            decision: if check == AccessCheck::PatchNodes {
                AccessDecision::Denied { reason: None }
            } else {
                AccessDecision::Allowed
            },
        })
        .collect();
    let session = slot_session(&t.fixture, &t.stg, cx);
    session.update(cx, |session, cx| {
        session.set_access_for_test(AccessState::Known(AccessReport { reviews }), cx);
    });
    // The gate says why, in the order the spec fixes: permission before the lock.
    let reason = t.fixture.shell.read_with(cx, |shell, cx| {
        let guard = shell.guard_for(&t.stg, cx).expect("a live guard");
        match crate::resource_actions::action_availability(ResourceAction::Cordon, &guard) {
            crate::resource_actions::ActionAvailability::Disabled { reason } => reason.to_string(),
            crate::resource_actions::ActionAvailability::Enabled => "enabled".to_owned(),
        }
    });
    assert_eq!(reason, "Not permitted: patch nodes");
    // A stale menu or a key cannot bypass it: no dialog opens and no request goes out.
    t.cordon(&t.stg, "node-b", cx);
    assert!(!t.has_dialog(cx));
    assert!(writes(&t.stg_api).is_empty());
    assert!(writes(&t.prod_api).is_empty());
}

// ---- The rest of the plan's window tests ----

#[gpui_kit::test]
fn lock_toggle_does_not_write_settings(cx: &mut TestAppContext) {
    let t = two_clusters("no-settings", cx);
    let dir = t.enable_audit_folder("no-settings", cx);
    t.toggle(&t.stg, cx);
    assert_eq!(t.lock_of(&t.stg, cx), WriteLock::Locked);
    t.toggle(&t.stg, cx);
    t.confirm(cx);
    assert_eq!(t.lock_of(&t.stg, cx), WriteLock::Unlocked);
    assert!(!dir.join("settings.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn ctrl_shift_r_locks_at_once(cx: &mut TestAppContext) {
    let t = two_clusters("chord-locks", cx);
    let table = t
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.node_table.clone());
    cx.update(|cx| table.update(cx, |table, cx| table.set_selected_row(1, cx)));
    cx.run_until_parked();
    assert_eq!(
        t.fixture
            .shell
            .read_with(cx, |shell, _| shell.lock_target()),
        Some(t.stg.clone())
    );
    t.fixture.press("secondary-shift-r", cx);
    assert_eq!(t.lock_of(&t.stg, cx), WriteLock::Locked);
    assert!(!t.has_dialog(cx), "locking asks nothing");
}

#[gpui_kit::test]
fn reconnect_bumps_the_generation(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("reconnect-generation", cx);
    fixture.wait_until_failed(cx);
    let session = fixture.session(cx);
    let before = session.read_with(cx, |session, _| (session.generation(), session.lock()));
    session.update(cx, |session, cx| session.retry(cx));
    let after = session.read_with(cx, |session, _| (session.generation(), session.lock()));
    assert!(after.0 > before.0, "{before:?} then {after:?}");
    // A reconnect keeps the lock the session had.
    assert_eq!(after.1, before.1);
}

#[gpui_kit::test]
fn confirm_dialog_enables_apply_after_dry_run_passes(cx: &mut TestAppContext) {
    let t = two_clusters("enables-apply", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    assert_eq!(t.block(cx), None);
    let state = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.dry_run_state());
    assert!(
        matches!(state, Some(DryRunState::Passed { .. })),
        "{state:?}"
    );
}

#[gpui_kit::test]
fn rejected_dry_run_keeps_apply_disabled(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("rejected", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let prod_api = go_live_fake(&fixture, &prod, "node-a", cx);
    let rejecting = |request: &RecordedRequest| {
        if request.method != "PATCH" {
            return (404, NOT_FOUND.to_owned());
        }
        let status = serde_json::json!({
            "kind": "Status", "apiVersion": "v1", "status": "Failure", "reason": "BadRequest",
            "message": "admission webhook \"x.example.com\" does not support dry run", "code": 400,
        });
        (400, status.to_string())
    };
    let stg_api = go_live_answering(&fixture, &stg, "node-b", rejecting, cx);
    let t = Clusters {
        fixture,
        prod_api,
        stg_api,
        prod,
        stg,
    };
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    let state = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.dry_run_state());
    assert!(matches!(state, Some(DryRunState::Rejected(_))), "{state:?}");
    let block = t.block(cx).expect("the commit is blocked");
    assert!(
        block.starts_with("An admission webhook does not support dry-run"),
        "{block}"
    );
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), 1, "only the rejected dry-run");
}

#[gpui_kit::test]
fn closing_the_dialog_drops_the_dry_run(cx: &mut TestAppContext) {
    let t = two_clusters("closing", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.fixture.with_window(cx, |window, cx| {
        use gpui_kit::component::WindowExt as _;
        window.close_dialog(cx);
    });
    cx.run_until_parked();
    assert!(!t.has_dialog(cx), "nothing keeps the dialog alive");
    // The audit log has no line for a change that was never confirmed.
    assert!(writes(&t.prod_api).is_empty());
}

#[gpui_kit::test]
fn checked_write_dry_run_writes_no_audit(cx: &mut TestAppContext) {
    let t = two_clusters("dry-run-no-audit", cx);
    let dir = t.enable_audit_folder("dry-run-no-audit", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    cx.run_until_parked();
    assert!(audit_lines(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- The review's smaller findings ----

#[gpui_kit::test]
fn the_retry_button_shows_after_a_failed_check(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("retry-shown", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let prod_api = go_live_fake(&fixture, &prod, "node-a", cx);
    // A refusal for now (429) on the dry-run: the check failed and can be run again.
    let refusing = |request: &RecordedRequest| {
        if request.method != "PATCH" {
            return (404, NOT_FOUND.to_owned());
        }
        let status = serde_json::json!({
            "kind": "Status", "apiVersion": "v1", "status": "Failure", "reason": "TooManyRequests",
            "message": "slow down", "code": 429,
        });
        (429, status.to_string())
    };
    let stg_api = go_live_answering(&fixture, &stg, "node-b", refusing, cx);
    let t = Clusters {
        fixture,
        prod_api,
        stg_api,
        prod,
        stg,
    };
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    t.fixture.draw_twice(cx);
    assert!(t.fixture.is_drawn("write-retry", cx));
    assert!(t.block(cx).is_some());
}

#[gpui_kit::test]
fn a_dialog_that_passed_its_check_has_no_retry(cx: &mut TestAppContext) {
    let t = two_clusters("no-retry", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    t.fixture.draw_twice(cx);
    assert!(!t.fixture.is_drawn("write-retry", cx));
}

#[gpui_kit::test]
fn escape_marks_the_dialog_closed(cx: &mut TestAppContext) {
    let t = two_clusters("escape-closes", cx);
    t.cordon(&t.stg, "node-b", cx);
    let dialog = t.dialog(cx);
    assert!(dialog.read_with(cx, |dialog, _| dialog.is_open()));
    t.fixture.draw_twice(cx);
    t.fixture.press("escape", cx);
    assert!(!dialog.read_with(cx, |dialog, _| dialog.is_open()));
}

#[gpui_kit::test]
fn a_session_that_is_not_live_offers_no_lock(cx: &mut TestAppContext) {
    // The primary connects to a closed port and never goes live.
    let fixture = open_switch_fixture("not-live", cx);
    let cluster = fixture.cluster("prod-a", cx);
    let session = slot_session(&fixture, &cluster, cx);
    let before = session.read_with(cx, |session, _| session.lock());
    fixture.with_window(cx, |window, cx| {
        fixture.shell.update(cx, |shell, cx| {
            shell.toggle_write_lock(&cluster, window, cx)
        });
    });
    assert_eq!(session.read_with(cx, |session, _| session.lock()), before);
    assert!(
        fixture
            .shell
            .read_with(cx, |shell, _| shell.last_dialog.clone())
            .is_none()
    );
}

#[gpui_kit::test]
fn a_menu_that_is_out_of_date_adds_a_warning(cx: &mut TestAppContext) {
    let t = two_clusters("stale-menu", cx);
    // The menu was built for a schedulable node, which has been cordoned since.
    let session = slot_session(&t.fixture, &t.stg, cx);
    session.update(cx, |session, cx| {
        session.set_nodes_for_test(vec![node("node-b", NodeScheduling::Disabled)], cx);
    });
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.start_cordon(&t.stg, "node-b", Some(NodeScheduling::Enabled), window, cx);
        });
    });
    let lines = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.warning_lines());
    assert_eq!(lines.len(), 1);
    assert_eq!(
        &*lines[0],
        "The menu offered Cordon, but the node has changed since, so this is Uncordon."
    );
}

#[gpui_kit::test]
fn an_up_to_date_menu_adds_no_warning(cx: &mut TestAppContext) {
    let t = two_clusters("fresh-menu", cx);
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.start_cordon(&t.stg, "node-b", Some(NodeScheduling::Enabled), window, cx);
        });
    });
    let lines = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.warning_lines());
    assert!(lines.is_empty());
}

#[gpui_kit::test]
fn a_tier_made_stricter_after_opening_applies(cx: &mut TestAppContext) {
    let t = two_clusters("stricter", cx);
    t.cordon(&t.stg, "node-b", cx);
    t.wait_for_dry_run(cx);
    // Opened on STG's click tier; Settings now asks for the typed name.
    assert_eq!(t.block(cx), None);
    cx.update(|cx| {
        crate::clusters_page::set_confirm(
            &t.stg,
            Some(crate::write_guard::ConfirmMode::TypeName),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(t.block(cx).as_deref(), Some("Type stg-b to confirm"));
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.stg_api).len(), 1, "only the dry-run was sent");
    t.type_name("stg-b", cx);
    assert_eq!(t.block(cx), None);
}

#[gpui_kit::test]
fn the_audit_log_of_a_lock_session_holds_only_lock_lines(cx: &mut TestAppContext) {
    // Test-plan step 4: Ctrl Shift R on a production cluster, unlock with the typed name, lock again.
    let t = two_clusters("lock-session", cx);
    let dir = t.enable_audit_folder("lock-session", cx);
    let table = t
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.node_table.clone());
    cx.update(|cx| table.update(cx, |table, cx| table.set_selected_row(0, cx)));
    cx.run_until_parked();
    assert_eq!(
        t.fixture
            .shell
            .read_with(cx, |shell, _| shell.lock_target()),
        Some(t.prod.clone())
    );
    t.fixture.press("secondary-shift-r", cx);
    t.type_name("prod-a", cx);
    t.confirm(cx);
    t.fixture.press("secondary-shift-r", cx);
    t.wait_for("two lines", cx, |_| audit_lines(&dir).len() == 2);
    let actions: Vec<String> = audit_lines(&dir)
        .iter()
        .map(|line| line["action"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(actions, ["Unlock", "Lock"]);
    assert!(writes(&t.prod_api).is_empty() && writes(&t.stg_api).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
