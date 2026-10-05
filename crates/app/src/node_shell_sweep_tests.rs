//! The leftover sweep over two loaded clusters (see `debug_open_tests` for the fixture): a list on
//! the first Live of the active one, a notice that never deletes, and a delete that goes through the
//! cleanup path with the uid the list gave.

use cluster::{AccessCheck, LeftoverPhase};
use gpui_kit::TestAppContext;
use gpui_kit::component::WindowExt as _;

use super::*;
use crate::app_shell::debug_open::debug_open_tests::{
    Answers, Debugs, audit_lines, go_live_answering, report_denying, respond, two_clusters,
};
use crate::app_shell::node_shell_open::node_shell_open_tests::deletes;
use crate::cluster_session::AccessState;
use crate::environment::Environment;
use crate::write_guard::{WriteLock, test_guard};

fn leftover(name: &str, phase: LeftoverPhase) -> NodeShellLeftover {
    NodeShellLeftover {
        namespace: "kube-system".to_owned(),
        name: name.to_owned(),
        uid: format!("uid-{name}"),
        node: Some("wk-03".to_owned()),
        phase,
        created_at: None,
    }
}

fn lists(api: &cluster::fake_api::FakeApi) -> Vec<cluster::fake_api::RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| request.method == "GET" && request.has_query_key("labelSelector"))
        .collect()
}

impl Debugs {
    fn notices(&self, cx: &mut TestAppContext) -> Vec<(ClusterRef, usize)> {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.sweep_notices.clone())
    }
}

#[gpui_kit::test]
fn sweep_runs_on_first_live(cx: &mut TestAppContext) {
    let debugs = two_clusters("sw-first", Answers::Leftovers, cx);
    debugs.wait_for("the notice", cx, |cx| debugs.notices(cx).len() == 1);
    // One list, on the connection of the cluster that went live; prod-a never did.
    assert_eq!(lists(&debugs.stg_api).len(), 1);
    assert_eq!(debugs.notices(cx), [(debugs.stg.clone(), 2)]);
}

#[gpui_kit::test]
fn sweep_lists_other_instances_in_any_phase(cx: &mut TestAppContext) {
    let debugs = two_clusters("sw-selector", Answers::Leftovers, cx);
    debugs.wait_for("the list", cx, |_| !lists(&debugs.stg_api).is_empty());
    let run_id = debugs.run_id(cx);
    let request = lists(&debugs.stg_api).remove(0);
    let selector = request
        .query
        .split('&')
        .find_map(|pair| pair.strip_prefix("labelSelector="))
        .expect("a label selector")
        .replace("%2C", ",")
        .replace("%21", "!")
        .replace("%3D", "=")
        .replace("%2F", "/");
    assert_eq!(
        selector,
        format!(
            "app.kubernetes.io/managed-by=k8sboard,k8sboard.io/purpose=node-shell,k8sboard.io/instance!={run_id}"
        )
    );
    // The Running and the Succeeded pod of the other run are both counted.
    debugs.wait_for("the notice", cx, |cx| !debugs.notices(cx).is_empty());
    assert!(debugs.notices(cx).iter().all(|(_, count)| *count == 2));
}

#[gpui_kit::test]
fn sweep_notice_never_deletes_by_itself(cx: &mut TestAppContext) {
    let debugs = two_clusters("sw-quiet", Answers::Leftovers, cx);
    debugs.wait_for("the notice", cx, |cx| debugs.notices(cx).len() == 1);
    debugs.settle_for_sweep(cx);
    assert!(deletes(&debugs.stg_api).is_empty());
    assert!(
        !debugs.has_dialog(cx),
        "no dialog until the user clicks Review"
    );
}

impl Debugs {
    fn settle_for_sweep(&self, cx: &mut TestAppContext) {
        std::thread::sleep(std::time::Duration::from_millis(150));
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn a_cluster_with_nothing_left_says_nothing(cx: &mut TestAppContext) {
    let debugs = two_clusters("sw-none", Answers::Accepts, cx);
    debugs.wait_for("the list", cx, |_| !lists(&debugs.stg_api).is_empty());
    debugs.settle_for_sweep(cx);
    assert!(debugs.notices(cx).is_empty());
}

#[gpui_kit::test]
fn sweep_deletes_selected_leftovers_through_cleanup(cx: &mut TestAppContext) {
    let debugs = two_clusters("sw-delete", Answers::Leftovers, cx);
    let dir = debugs.audit_folder("sw-delete", cx);
    let rows = [
        leftover("k8sboard-node-shell-wk-03-aaaaa", LeftoverPhase::Running),
        leftover("k8sboard-node-shell-wk-03-bbbbb", LeftoverPhase::Succeeded),
    ];
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.delete_leftovers(&debugs.stg, &rows, window, cx);
        });
    });
    debugs.wait_for("both deletes", cx, |_| deletes(&debugs.stg_api).len() == 2);
    let mut uids: Vec<String> = deletes(&debugs.stg_api)
        .iter()
        .map(|request| {
            let body: serde_json::Value = serde_json::from_str(&request.body).expect("JSON");
            body["preconditions"]["uid"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    uids.sort();
    assert_eq!(
        uids,
        [
            "uid-k8sboard-node-shell-wk-03-aaaaa",
            "uid-k8sboard-node-shell-wk-03-bbbbb"
        ]
    );
    // One audit line per delete.
    debugs.wait_for("both lines", cx, |_| {
        audit_lines(&dir)
            .iter()
            .filter(|line| line["action"] == "Delete node shell pod")
            .count()
            == 2
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_locked_cluster_deletes_nothing_and_says_why(cx: &mut TestAppContext) {
    let debugs = two_clusters("sw-locked", Answers::Leftovers, cx);
    let prod_api = debugs.activate(&debugs.prod, cx);
    // prod-a is locked at open.
    let rows = [leftover(
        "k8sboard-node-shell-wk-03-aaaaa",
        LeftoverPhase::Succeeded,
    )];
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.delete_leftovers(&debugs.prod, &rows, window, cx);
        });
    });
    debugs.settle_for_sweep(cx);
    assert!(deletes(&prod_api).is_empty());
}

#[gpui_kit::test]
fn a_denied_right_to_delete_deletes_nothing(cx: &mut TestAppContext) {
    let debugs = two_clusters("sw-denied", Answers::Leftovers, cx);
    debugs.set_access(&debugs.stg, report_denying(&[AccessCheck::DeletePods]), cx);
    let rows = [leftover(
        "k8sboard-node-shell-wk-03-aaaaa",
        LeftoverPhase::Succeeded,
    )];
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.delete_leftovers(&debugs.stg, &rows, window, cx);
        });
    });
    debugs.settle_for_sweep(cx);
    assert!(deletes(&debugs.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_review_dialog_opens_over_the_listed_pods(cx: &mut TestAppContext) {
    let debugs = two_clusters("sw-review", Answers::Accepts, cx);
    let rows = vec![
        leftover("k8sboard-node-shell-wk-03-aaaaa", LeftoverPhase::Running),
        leftover("k8sboard-node-shell-wk-03-bbbbb", LeftoverPhase::Failed),
    ];
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.open_leftover_review(debugs.stg.clone(), rows, window, cx);
        });
    });
    assert!(debugs.has_dialog(cx));
    debugs
        .fixture
        .with_window(cx, |window, cx| window.close_dialog(cx));
}

#[test]
fn finished_pods_are_checked_and_running_ones_are_not() {
    for (phase, expected) in [
        (LeftoverPhase::Succeeded, true),
        (LeftoverPhase::Failed, true),
        (LeftoverPhase::Running, false),
        (LeftoverPhase::Pending, false),
        (LeftoverPhase::Unknown, false),
    ] {
        assert_eq!(
            is_checked_by_default(&leftover("x", phase)),
            expected,
            "{phase:?}"
        );
    }
}

#[test]
fn the_notice_counts_pods() {
    assert_eq!(notice_text(1), "1 leftover node shell pod");
    assert_eq!(notice_text(3), "3 leftover node shell pods");
}

#[test]
fn a_sweep_asks_a_cluster_only_when_the_review_does_not_say_no() {
    let denied = AccessState::Known(cluster::AccessReport {
        reviews: AccessCheck::ALL
            .into_iter()
            .map(|check| cluster::AccessReview {
                check,
                decision: cluster::AccessDecision::Denied { reason: None },
            })
            .collect(),
    });
    assert!(!may_list_pods(&denied));
    assert!(may_list_pods(&AccessState::Unknown));
    let allowed = AccessState::Known(report_denying(&[]));
    assert!(may_list_pods(&allowed));
}

#[test]
fn the_delete_button_reads_lock_and_right_in_that_order() {
    let allowed = AccessState::Known(report_denying(&[]));
    let open = test_guard(&allowed, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    assert_eq!(sweep_block(&open), None);
    let locked = test_guard(&allowed, WriteLock::Locked, "stg-b", Environment::STAGING);
    assert_eq!(sweep_block(&locked).as_deref(), Some("stg-b is read-only"));
    let denied = AccessState::Known(report_denying(&[AccessCheck::DeletePods]));
    let guard = test_guard(&denied, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    assert_eq!(
        sweep_block(&guard).as_deref(),
        Some("Not permitted: delete pods")
    );
    let unknown = AccessState::Unknown;
    let guard = test_guard(&unknown, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    assert_eq!(
        sweep_block(&guard).as_deref(),
        Some("Permissions could not be checked")
    );
}

#[test]
fn a_listed_pod_becomes_a_delete_of_that_uid_only() {
    let request = delete_request(&leftover(
        "k8sboard-node-shell-wk-03-aaaaa",
        LeftoverPhase::Running,
    ))
    .expect("a delete");
    assert!(matches!(
        request.operation(),
        WriteOperation::DeleteNodeShellPod { uid } if uid == "uid-k8sboard-node-shell-wk-03-aaaaa"
    ));
    // A pod that is not named like ours cannot become a delete at all.
    assert!(delete_request(&leftover("coredns-5d78c9869d-abcde", LeftoverPhase::Failed)).is_none());
}

#[test]
fn the_review_warns_about_running_pods() {
    assert_eq!(
        RUNNING_WARNING,
        "Running pods may belong to another k8sBoard window or user."
    );
}

#[test]
fn the_sweep_lists_the_node_shell_namespace_beside_the_view_scope() {
    let named = NamespaceScope::of_namespaces(["shop".to_owned()]);
    assert_eq!(
        sweep_scope(&named, Some("kube-system")),
        NamespaceScope::of_namespaces(["kube-system".to_owned(), "shop".to_owned()])
    );
    // Already covered: nothing is added twice, and `All` stays `All`.
    assert_eq!(
        sweep_scope(&named, Some("shop")),
        NamespaceScope::of_namespaces(["shop".to_owned()])
    );
    assert_eq!(
        sweep_scope(&NamespaceScope::All, Some("kube-system")),
        NamespaceScope::All
    );
    assert_eq!(sweep_scope(&named, None), named);
}

#[gpui_kit::test]
fn a_view_scoped_elsewhere_still_sweeps_the_node_shell_namespace(cx: &mut TestAppContext) {
    use cluster::NamespaceScope;
    let debugs = two_clusters("sw-scope", Answers::Accepts, cx);
    // Rescope stg-b to one namespace, then look at what a sweep of it asks.
    let session = debugs.session(&debugs.stg, cx);
    session.update(cx, |session, cx| {
        session.set_scope(NamespaceScope::of_namespaces(["shop".to_owned()]), cx);
    });
    cx.run_until_parked();
    let before = lists(&debugs.stg_api).len();
    debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.sweep_leftovers(&debugs.stg, cx));
    debugs.wait_for("the lists", cx, |_| {
        lists(&debugs.stg_api).len() >= before + 2
    });
    let paths: Vec<String> = lists(&debugs.stg_api)
        .into_iter()
        .skip(before)
        .map(|request| request.path)
        .collect();
    assert!(
        paths.contains(&"/api/v1/namespaces/kube-system/pods".to_owned()),
        "{paths:?}"
    );
    assert!(
        paths.contains(&"/api/v1/namespaces/shop/pods".to_owned()),
        "{paths:?}"
    );
}

#[gpui_kit::test]
fn a_leftover_notice_landing_after_a_switch_is_dropped(cx: &mut TestAppContext) {
    use std::sync::{Mutex, mpsc};
    let debugs = two_clusters("sw-after-switch", Answers::Accepts, cx);
    // The list of stg-b waits at the server until the shell has switched to prod-a.
    let (release, gate) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let held_api = go_live_answering(
        &debugs.fixture,
        &debugs.stg,
        move |request| {
            if request.method == "GET"
                && request.has_query_key("labelSelector")
                && let Ok(gate) = gate.lock()
            {
                let _ = gate.recv_timeout(std::time::Duration::from_secs(10));
            }
            respond(Answers::Leftovers, request)
        },
        cx,
    );
    debugs
        .fixture
        .shell
        .update(cx, |shell, cx| shell.sweep_leftovers(&debugs.stg, cx));
    debugs.wait_for("the list", cx, |_| !lists(&held_api).is_empty());
    debugs.activate(&debugs.prod, cx);
    let _ = release.send(());
    debugs.settle_for_sweep(cx);
    assert!(
        debugs.notices(cx).is_empty(),
        "a notice for a cluster that left is dropped"
    );
}
