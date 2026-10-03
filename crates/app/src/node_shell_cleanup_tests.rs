//! A node shell pod never outlives its session: every end of a tab deletes it once, on the
//! connection the tab held, with its own audit line. The fixture (`debug_open_tests`) keeps a node
//! shell tab open by leaving the pod in ContainerCreating, and answers a delete with 200.

use std::time::Duration;

use cluster::fake_api::FakeApi;
use cluster::{ShellExit, ShellUpdate, WritePolicy};
use gpui_kit::TestAppContext;
use gpui_kit::component::dialog::Cancel;
use gpui_kit::component::dialog::Confirm;

use super::*;
use crate::app_shell::debug_open::debug_open_tests::{Answers, Debugs, audit_lines};
use crate::app_shell::node_shell_open::node_shell_open_tests::{
    audited_node_clusters, deletes, node_clusters,
};
use crate::app_shell::write_flow::CleanupAudit;
use crate::cluster_registry::ClusterRef;
use crate::write_guard::WriteLock;

impl Debugs {
    /// Opens a node shell on `wk-03` of `cluster` and waits for its tab.
    fn open_live_node_shell(&self, cluster: &ClusterRef, cx: &mut TestAppContext) {
        let before = self.tab_count(cx);
        self.start_node(
            cluster,
            "wk-03",
            "kube-system",
            cluster::DEFAULT_DEBUG_IMAGE,
            cx,
        );
        self.confirm(cx);
        self.wait_for("the node shell tab", cx, |cx| {
            self.tab_count(cx) == before + 1
        });
    }

    fn wait_for_deletes(&self, api: &FakeApi, count: usize, cx: &mut TestAppContext) {
        self.wait_for("the delete", cx, |_| deletes(api).len() >= count);
    }

    /// Lets a late second delete show up before a test says there was only one.
    fn settle(&self, cx: &mut TestAppContext) {
        std::thread::sleep(Duration::from_millis(150));
        cx.run_until_parked();
    }

    fn press(&self, answer: impl gpui_kit::Action, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            window.dispatch_action(Box::new(answer), cx);
        });
    }
}

fn delete_lines(dir: &std::path::Path) -> Vec<serde_json::Value> {
    audit_lines(dir)
        .into_iter()
        .filter(|line| line["action"] == "Delete node shell pod")
        .collect()
}

// ---- every end of a session ----

#[gpui_kit::test]
fn a_shell_that_exits_deletes_its_pod(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-exit", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    assert!(
        deletes(&debugs.stg_api).is_empty(),
        "a live shell keeps its pod"
    );
    let tab = debugs.tabs(cx).remove(0);
    tab.update(cx, |tab, cx| {
        tab.apply(
            ShellUpdate::Exited(ShellExit {
                code: Some(0),
                message: None,
            }),
            cx,
        );
    });
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    let delete = deletes(&debugs.stg_api).remove(0);
    assert_eq!(delete.method, "DELETE");
    assert!(
        delete
            .path
            .starts_with("/api/v1/namespaces/kube-system/pods/k8sboard-node-shell-wk-03-")
    );
    assert!(
        !delete.has_query_key("dryRun"),
        "a cleanup is a commit only"
    );
    assert!(deletes(&debugs.prod_api).is_empty());
}

#[gpui_kit::test]
fn closing_the_tab_deletes_its_pod(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-close", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.dock.update(cx, |dock, cx| dock.close_active_tab(cx));
    });
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    assert_eq!(debugs.tab_count(cx), 0);
}

#[gpui_kit::test]
fn a_switch_asks_then_deletes_the_pod_on_the_held_connection(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-switch", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    let target = debugs.fixture.cluster("dev-c", cx);
    debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    let lines = debugs
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.last_leaving.clone());
    assert_eq!(
        lines,
        Some(vec![
            "1 node shell will close; its pod is deleted".to_owned()
        ])
    );
    // Nothing is deleted while the question is open, and Esc keeps everything.
    assert!(deletes(&debugs.stg_api).is_empty());
    debugs.press(Cancel, cx);
    debugs.settle(cx);
    assert!(deletes(&debugs.stg_api).is_empty());
    assert_eq!(debugs.tab_count(cx), 1);
    // Asking again and confirming releases the sessions; the delete still reaches stg-b.
    debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    debugs.press(Confirm { secondary: false }, cx);
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    assert_eq!(debugs.tab_count(cx), 0);
}

#[gpui_kit::test]
fn releasing_a_slot_deletes_only_its_pods(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-release", Answers::Waiting, cx);
    debugs.set_lock(&debugs.prod, WriteLock::Unlocked, cx);
    debugs.open_live_node_shell(&debugs.prod, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.remove_from_view(&debugs.stg, cx));
    cx.run_until_parked();
    debugs.press(Confirm { secondary: false }, cx);
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    debugs.settle(cx);
    assert_eq!(deletes(&debugs.stg_api).len(), 1);
    assert!(
        deletes(&debugs.prod_api).is_empty(),
        "the other cluster's pod stays"
    );
    assert_eq!(debugs.tab_count(cx), 1);
}

#[gpui_kit::test]
fn a_failed_attach_deletes_the_pod_it_created(cx: &mut TestAppContext) {
    // The fake finds no pod to wait for, so the attach fails at once.
    let debugs = node_clusters("nc-fail", Answers::Accepts, cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
}

#[gpui_kit::test]
fn cleanup_runs_once(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-once", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    let tab = debugs.tabs(cx).remove(0);
    tab.update(cx, |tab, cx| {
        tab.apply(
            ShellUpdate::Exited(ShellExit {
                code: Some(0),
                message: None,
            }),
            cx,
        );
    });
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    // The exit is followed by the close of the tab, a switch, and a quit: still one delete.
    drop(tab);
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.dock.update(cx, |dock, cx| dock.close_all(cx));
    });
    debugs.settle(cx);
    assert_eq!(deletes(&debugs.stg_api).len(), 1);
}

#[gpui_kit::test]
fn cleanup_needs_no_gate_and_audits_one_line(cx: &mut TestAppContext) {
    let (debugs, dir) = audited_node_clusters("nc-gate", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    // The cluster is locked and the right to delete is gone by the time the session ends.
    debugs.set_lock(&debugs.stg, WriteLock::Locked, cx);
    debugs.set_access(
        &debugs.stg,
        crate::app_shell::debug_open::debug_open_tests::report_denying(&[
            cluster::AccessCheck::DeletePods,
        ]),
        cx,
    );
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.dock.update(cx, |dock, cx| dock.close_active_tab(cx));
    });
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    debugs.wait_for("the delete line", cx, |_| delete_lines(&dir).len() == 1);
    let line = delete_lines(&dir).remove(0);
    assert_eq!(line["outcome"], "applied");
    assert_eq!(line["cluster"], "stg-b");
    assert_eq!(line["object"]["namespace"], "kube-system");
    debugs.settle(cx);
    assert_eq!(delete_lines(&dir).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_pod_created_while_the_window_is_closing_is_deleted_at_once(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-late", Answers::Waiting, cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let dialog = debugs.dialog(cx);
    debugs.wait_for_dry_run(&dialog, cx);
    // The window was asked to close while the create was on its way.
    debugs.fixture.shell.update(cx, |shell, _| {
        shell.node_shell_runs.is_closing = true;
    });
    debugs.confirm(cx);
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    assert_eq!(debugs.tab_count(cx), 0, "no tab opens for a closing window");
}

// ---- the delete itself ----

fn cleanup_over(connection: cluster::ClusterConnection) -> NodeShellCleanup {
    let target = cluster::ObjectRef::new(
        cluster::ObjectKind::Pod,
        Some("kube-system".to_owned()),
        "k8sboard-node-shell-wk-03-x7k2q".to_owned(),
    )
    .expect("a pod");
    let request = cluster::WriteRequest::new(
        target,
        cluster::WriteOperation::DeleteNodeShellPod {
            uid: "uid-1".to_owned(),
        },
    )
    .expect("a delete");
    let access = crate::cluster_session::AccessState::Unknown;
    let guard = crate::write_guard::test_guard(
        &access,
        WriteLock::Locked,
        "stg-b",
        crate::environment::Environment::Staging,
    );
    NodeShellCleanup::new(connection, request, CleanupAudit::of(&guard)).expect("a cleanup")
}

fn run(policy: WritePolicy, code: u16, dir: Option<std::path::PathBuf>) -> CleanupOutcome {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let _guard = runtime.enter();
    let (connection, _api) = FakeApi::connection(policy, move |_| {
        (
            code,
            format!(
                r#"{{"kind":"Status","apiVersion":"v1","status":"Failure","message":"m","reason":"r","code":{code}}}"#
            ),
        )
    });
    let cluster_runtime = ClusterRuntime::new(runtime.handle().clone());
    runtime.block_on(run_cleanup(cleanup_over(connection), &cluster_runtime, dir))
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("k8sboard-0037-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp dir");
    dir
}

#[test]
fn cleanup_404_counts_as_done() {
    let dir = temp_dir("gone");
    assert_eq!(
        run(WritePolicy::Allowed, 404, Some(dir.clone())),
        CleanupOutcome::Done
    );
    let lines = audit_lines(&dir);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["outcome"], "applied");
    assert!(
        lines[0].get("error").is_none(),
        "no error for a pod that is gone"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pod_another_object_took_the_name_of_is_not_deleted_and_is_audited_as_failed() {
    let dir = temp_dir("taken");
    let outcome = run(WritePolicy::Allowed, 409, Some(dir.clone()));
    assert_eq!(
        outcome,
        CleanupOutcome::Failed("another pod now has that name".to_owned())
    );
    assert_eq!(audit_lines(&dir)[0]["outcome"], "failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn any_other_failure_names_why_and_is_audited() {
    let dir = temp_dir("refused");
    match run(WritePolicy::Allowed, 500, Some(dir.clone())) {
        CleanupOutcome::Failed(text) => assert!(!text.is_empty()),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(audit_lines(&dir)[0]["outcome"], "failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_blocked_build_sends_nothing_and_writes_no_line() {
    let dir = temp_dir("blocked");
    assert_eq!(
        run(WritePolicy::Blocked, 200, Some(dir.clone())),
        CleanupOutcome::Blocked
    );
    assert!(audit_lines(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- the window and the app quit ----

#[gpui_kit::test]
fn a_window_with_no_node_shell_closes_at_once(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-free", Answers::Waiting, cx);
    let may_close = debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.main_window_may_close(cx));
    assert!(may_close);
}

#[gpui_kit::test]
fn window_close_waits_for_pending_deletes(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-window", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    let may_close = debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.main_window_may_close(cx));
    assert!(!may_close, "the close waits while a pod is still there");
    // The deletes start at once, and asking again does not start them again.
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    let again = debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.main_window_may_close(cx));
    debugs.settle(cx);
    assert_eq!(deletes(&debugs.stg_api).len(), 1);
    let _ = again;
    // When the last delete reported, the window closes itself.
    debugs.wait_for("the window to close", cx, |cx| {
        cx.update_window(debugs.fixture.window.into(), |_, _, _| ())
            .is_err()
    });
}

#[gpui_kit::test]
fn quit_hook_is_best_effort(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-quit", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    let waiting = debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.cleanup_for_quit(cx));
    // The future holds the running deletes: it is polled like GPUI polls it at a quit.
    let _waiting = cx.update(|cx| cx.spawn(async move |_| waiting.await));
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    assert_eq!(
        deletes(&debugs.stg_api).len(),
        1,
        "the quit hook deleted the pod"
    );
}

#[gpui_kit::test]
fn a_delete_that_cannot_report_before_the_quit_leaves_an_abandoned_line(cx: &mut TestAppContext) {
    let (debugs, dir) = audited_node_clusters("nc-abandon", Answers::Waiting, cx);
    // A connection that accepts the request and never answers: the delete cannot report.
    let (hanging, _api) = {
        let _guard = debugs.fixture.runtime.enter();
        FakeApi::failing(WritePolicy::Allowed, cluster::fake_api::Failure::Hang)
    };
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.begin_cleanup(cleanup_over(hanging), cx);
    });
    cx.run_until_parked();
    let waiting = debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.cleanup_for_quit(cx));
    drop(waiting);
    let lines = delete_lines(&dir);
    assert_eq!(lines.len(), 1, "written at once, before the process ends");
    assert_eq!(lines[0]["outcome"], "abandoned");
    assert_eq!(
        lines[0]["error"],
        "the delete did not report before the app quit"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_delete_the_server_refuses_is_audited_as_failed_and_is_no_longer_pending(
    cx: &mut TestAppContext,
) {
    let (debugs, dir) = audited_node_clusters("nc-refused", Answers::Deletes(500), cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the delete line", cx, |_| delete_lines(&dir).len() == 1);
    let line = delete_lines(&dir).remove(0);
    assert_eq!(line["outcome"], "failed");
    assert!(
        line["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty())
    );
    debugs.wait_for("the delete to settle", cx, |cx| {
        debugs
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.node_shell_runs.pending.is_empty())
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_pod_that_is_gone_at_delete_time_is_done_without_an_error(cx: &mut TestAppContext) {
    let (debugs, dir) = audited_node_clusters("nc-gone", Answers::Deletes(404), cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the delete line", cx, |_| delete_lines(&dir).len() == 1);
    let line = delete_lines(&dir).remove(0);
    assert_eq!(line["outcome"], "applied");
    assert!(line.get("error").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
