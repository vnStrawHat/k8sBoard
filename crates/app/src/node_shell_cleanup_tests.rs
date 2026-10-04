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
fn a_switch_deletes_the_node_shell_pods_of_the_old_cluster(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-release", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&debugs.prod, cx));
    cx.run_until_parked();
    debugs.press(Confirm { secondary: false }, cx);
    debugs.wait_for_deletes(&debugs.stg_api, 2, cx);
    debugs.settle(cx);
    assert_eq!(deletes(&debugs.stg_api).len(), 2);
    assert_eq!(debugs.tab_count(cx), 0);
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

// ---- review fixes ----

fn window_is_open(debugs: &Debugs, cx: &mut TestAppContext) -> bool {
    cx.update_window(debugs.fixture.window.into(), |_, _, _| ())
        .is_ok()
}

#[gpui_kit::test]
fn the_title_bar_close_asks_the_shell_first(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-titlebar", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    debugs.fixture.with_window(cx, |window, cx| {
        debugs
            .fixture
            .shell
            .update(cx, |shell, cx| shell.close_main_window(window, cx));
    });
    // The pod is deleted first; the window closes when the delete reports.
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    debugs.wait_for("the window to close", cx, |cx| !window_is_open(&debugs, cx));
}

#[gpui_kit::test]
fn the_title_bar_close_without_pods_closes_at_once(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-titlebar-free", Answers::Waiting, cx);
    debugs.fixture.with_window(cx, |window, cx| {
        debugs
            .fixture
            .shell
            .update(cx, |shell, cx| shell.close_main_window(window, cx));
    });
    assert!(!window_is_open(&debugs, cx));
}

#[gpui_kit::test]
fn quit_writes_an_abandoned_line_for_every_delete_it_starts(cx: &mut TestAppContext) {
    let (debugs, dir) = audited_node_clusters("nc-quit-order", Answers::Waiting, cx);
    debugs.open_live_node_shell(&debugs.stg, cx);
    // Nothing has been deleted yet: the quit starts the delete, then writes its line.
    let waiting = debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.cleanup_for_quit(cx));
    let lines = delete_lines(&dir);
    assert!(
        lines.iter().any(|line| line["outcome"] == "abandoned"),
        "the delete the quit started has its abandoned line at once: {lines:?}"
    );
    drop(waiting);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_create_in_flight_keeps_the_window_open(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-creating", Answers::Waiting, cx);
    debugs
        .fixture
        .shell
        .update(cx, |shell, _| shell.node_shell_runs.create_started());
    let may_close = debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.main_window_may_close(cx));
    assert!(!may_close, "a pod may be created any moment");
    assert!(window_is_open(&debugs, cx));
    // Once the create has an owner (here: nothing to own), the close goes through.
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.node_shell_runs.create_finished();
        shell.close_window_when_idle(cx);
    });
    debugs.wait_for("the window to close", cx, |cx| !window_is_open(&debugs, cx));
}

fn node_plan(
    debugs: &Debugs,
    cluster: &ClusterRef,
    cx: &mut TestAppContext,
) -> crate::app_shell::debug_open::TabPlan {
    use crate::shell_tab::{ShellKind, ShellTarget};
    let audit = debugs.fixture.shell.read_with(cx, |shell, cx| {
        shell
            .guard_for(&debugs.stg, cx)
            .map(|guard| CleanupAudit::of(&guard))
    });
    crate::app_shell::debug_open::TabPlan {
        cluster: cluster.clone(),
        target: ShellTarget {
            cluster: cluster.clone(),
            namespace: "kube-system".to_owned(),
            pod: "k8sboard-node-shell-wk-03-x7k2q".to_owned(),
            short_pod: "wk-03".to_owned(),
            container: "shell".to_owned(),
        },
        kind: ShellKind::NodeShell {
            node: "wk-03".to_owned(),
            image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
        },
        cluster_label: "dev-c".to_owned(),
        image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
        namespace: Some("kube-system".to_owned()),
        cleanup_audit: audit,
        generation: debugs
            .fixture
            .shell
            .read_with(cx, |shell, cx| {
                shell.guard_for(cluster, cx).map(|guard| guard.generation)
            })
            .unwrap_or_default(),
    }
}

fn created(uid: Option<&str>) -> cluster::WriteOutcome {
    cluster::WriteOutcome {
        mode: cluster::WriteMode::Commit,
        elapsed: Duration::ZERO,
        effect: cluster::WriteEffect::Created,
        created_name: Some("k8sboard-node-shell-wk-03-x7k2q".to_owned()),
        dropped_fields: Vec::new(),
        uid: uid.map(str::to_owned),
    }
}

#[gpui_kit::test]
fn a_pod_created_for_a_cluster_that_left_the_view_is_deleted_not_forgotten(
    cx: &mut TestAppContext,
) {
    let debugs = node_clusters("nc-orphan", Answers::Waiting, cx);
    // dev-c is not viewed, so no slot owns a tab for it.
    let gone = debugs.fixture.cluster("dev-c", cx);
    let plan = node_plan(&debugs, &gone, cx);
    let connection = debugs.session_connection(&debugs.stg, cx);
    let permit = debugs.attach_permit();
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.open_debug_tab(
                &plan,
                permit,
                connection,
                created(Some("uid-1")),
                window,
                cx,
            );
        });
    });
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    assert_eq!(debugs.tab_count(cx), 0, "no tab for a released cluster");
}

#[gpui_kit::test]
fn a_pod_created_across_a_switch_and_back_is_deleted_not_given_a_tab(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-back", Answers::Waiting, cx);
    // The start was made on the first session of stg-b.
    let plan = node_plan(&debugs, &debugs.stg.clone(), cx);
    debugs.activate(&debugs.prod, cx);
    let stg_again = debugs.activate(&debugs.stg, cx);
    // The create answers now: the cluster is open again, but it is a new session.
    let connection = debugs.session_connection(&debugs.stg, cx);
    let permit = debugs.attach_permit();
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.open_debug_tab(
                &plan,
                permit,
                connection,
                created(Some("uid-1")),
                window,
                cx,
            );
        });
    });
    debugs.wait_for_deletes(&stg_again, 1, cx);
    assert_eq!(debugs.tab_count(cx), 0, "no tab in the new session");
}

#[gpui_kit::test]
fn a_discarded_start_deletes_its_pod_without_a_window(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-discard", Answers::Waiting, cx);
    let plan = node_plan(&debugs, &debugs.stg.clone(), cx);
    let connection = debugs.session_connection(&debugs.stg, cx);
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.discard_debug_start(&plan, &connection, &created(Some("uid-1")), cx);
    });
    debugs.wait_for_deletes(&debugs.stg_api, 1, cx);
    // A server that reported no uid leaves nothing the delete could be exact about.
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.discard_debug_start(&plan, &connection, &created(None), cx);
    });
    debugs.settle(cx);
    assert_eq!(deletes(&debugs.stg_api).len(), 1);
}

#[gpui_kit::test]
fn finished_deletes_are_dropped_and_any_delete_hooks_the_quit(cx: &mut TestAppContext) {
    let debugs = node_clusters("nc-prune", Answers::Waiting, cx);
    let plan = node_plan(&debugs, &debugs.stg.clone(), cx);
    let connection = debugs.session_connection(&debugs.stg, cx);
    assert!(
        debugs
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.node_shell_runs.quit.is_none())
    );
    for round in 1..=3 {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.discard_debug_start(&plan, &connection, &created(Some("uid-1")), cx);
        });
        debugs.wait_for("the delete", cx, |_| {
            deletes(&debugs.stg_api).len() >= round
        });
        debugs.wait_for("the delete to settle", cx, |cx| {
            debugs
                .fixture
                .shell
                .read_with(cx, |shell, _| shell.node_shell_runs.pending.is_empty())
        });
    }
    // A delete that no tab started (a sweep row, a late create) hooked the quit all the same.
    assert!(
        debugs
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.node_shell_runs.quit.is_some())
    );
    // The next start drops the tasks of the finished ones.
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.discard_debug_start(&plan, &connection, &created(Some("uid-1")), cx);
    });
    let kept = debugs
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.node_shell_runs.in_flight.len());
    assert_eq!(kept, 1);
}
