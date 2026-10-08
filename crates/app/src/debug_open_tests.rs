//! Starting a debug container in a headless window over two loaded clusters, one active at a time:
//! `prod-a` (Production, locked at open) and `stg-b` (unlocked, a click tier). The fixture starts
//! on `prod-a`, switches to `stg-b`, and makes it live over a fake API server; `activate` does the
//! same for the other one. A test sees which cluster a request reached and nothing leaves the
//! machine. The fake connections carry their own write
//! policy, and a fake never upgrades a connection to a stream: its pod reads answer 404, so the
//! attach of a started tab fails at once.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cluster::fake_api::{FakeApi, RecordedRequest};
use cluster::{
    AccessCheck, AccessDecision, AccessReport, AccessReview, ContainerKind, ContainerState,
    ContainerSummary, NamespaceScope, PodStatus, PodSummary, ReadyCount, StatusReason, WritePolicy,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::{Entity, TestAppContext};

use super::super::write_flow::DryRunState;
use super::*;
use crate::app_shell::Screen;
use crate::app_shell::app_shell_switch_tests::{SwitchFixture, open_switch_fixture};
use crate::cluster_session::{AccessState, ClusterSession};
use crate::confirm_dialog::ConfirmDialog;
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};
use crate::write_guard::{DialogConfirm, WriteLock};

const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;
const REFUSED: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"Invalid","code":422}"#;
const POD: &str =
    r#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"multi-0","namespace":"shop"}}"#;
pub(in crate::app_shell) const MIRROR_IMAGE: &str = "registry.local/tools/busybox:1.36";

/// How a fake cluster answers a debug start.
#[derive(Clone, Copy)]
pub(in crate::app_shell) enum Answers {
    /// Every patch and create is accepted; a delete succeeds.
    Accepts,
    /// A dry-run passes and the commit is refused.
    RefusesCommit,
    /// As `Accepts`, but a delete is answered with this status.
    Deletes(u16),
    /// As `Accepts`, and a node shell pod stays in ContainerCreating, so its tab stays open.
    Waiting,
    /// As `Accepts`, and a pod list finds two node shell pods of another run.
    Leftovers,
}

/// The uid every created pod gets.
pub(in crate::app_shell) const CREATED_UID: &str = "5f1c3a52-0f0b-4c1e-9b0e-2a3f6d6c9e11";

fn status_json(code: u16) -> String {
    format!(
        r#"{{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"Failure","code":{code}}}"#
    )
}

fn other_run_pod(name: &str, phase: &str) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {
            "name": name, "namespace": "kube-system", "uid": format!("uid-{name}"),
            "creationTimestamp": "2026-10-03T08:00:00Z",
            "labels": {
                "app.kubernetes.io/managed-by": "k8sboard",
                "k8sboard.io/purpose": "node-shell",
                "k8sboard.io/instance": "other-run",
            },
        },
        "spec": {"nodeName": "wk-03", "containers": []},
        "status": {"phase": phase},
    })
}

/// What the fake API server of a cluster answers: the debug container patch, the node shell pod
/// create and delete, the leftover list, and nothing else.
pub(in crate::app_shell) fn respond(answers: Answers, request: &RecordedRequest) -> (u16, String) {
    let is_commit = !request.has_query_key("dryRun");
    match request.method.as_str() {
        "PATCH" if is_commit && matches!(answers, Answers::RefusesCommit) => {
            (422, REFUSED.to_owned())
        }
        "PATCH" => (200, POD.to_owned()),
        "POST" if request.path.ends_with("/pods") => {
            if is_commit && matches!(answers, Answers::RefusesCommit) {
                return (422, REFUSED.to_owned());
            }
            let body: serde_json::Value = serde_json::from_str(&request.body).unwrap_or_default();
            let mut created = body;
            created["metadata"]["uid"] = serde_json::json!(CREATED_UID);
            (201, created.to_string())
        }
        "DELETE" => match answers {
            Answers::Deletes(code) if code >= 400 => (code, status_json(code)),
            _ => (
                200,
                r#"{"kind":"Status","apiVersion":"v1","status":"Success"}"#.to_owned(),
            ),
        },
        "GET"
            if matches!(answers, Answers::Waiting)
                && request.path.contains("/pods/k8sboard-node-shell-") =>
        {
            (
                200,
                serde_json::json!({
                    "apiVersion": "v1", "kind": "Pod",
                    "metadata": {"name": "k8sboard-node-shell"},
                    "status": {"phase": "Pending", "containerStatuses": [{
                        "name": "shell", "image": "busybox", "imageID": "", "ready": false,
                        "restartCount": 0, "state": {"waiting": {"reason": "ContainerCreating"}},
                    }]},
                })
                .to_string(),
            )
        }
        "GET" if request.path.ends_with("/pods") && request.has_query_key("labelSelector") => {
            let items = match answers {
                Answers::Leftovers => vec![
                    other_run_pod("k8sboard-node-shell-wk-03-aaaaa", "Running"),
                    other_run_pod("k8sboard-node-shell-wk-03-bbbbb", "Succeeded"),
                ],
                _ => Vec::new(),
            };
            (
                200,
                serde_json::json!({"apiVersion": "v1", "kind": "PodList", "metadata": {}, "items": items})
                    .to_string(),
            )
        }
        _ => (404, NOT_FOUND.to_owned()),
    }
}

/// A node of `operating_system`, ready and schedulable.
pub(in crate::app_shell) fn node(name: &str, operating_system: &str) -> cluster::NodeSummary {
    cluster::NodeSummary {
        annotations: cluster::AnnotationTerms::default(),
        name: name.to_owned(),
        status: cluster::NodeStatus {
            readiness: cluster::NodeReadiness::Ready,
            scheduling: cluster::NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: cluster::NodeSystemInfo {
            operating_system: operating_system.to_owned(),
            ..cluster::NodeSystemInfo::default()
        },
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

pub(in crate::app_shell) fn container(
    name: &str,
    kind: ContainerKind,
    is_running: bool,
) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind,
        state: if is_running {
            ContainerState::Running { started_at: None }
        } else {
            ContainerState::Waiting {
                reason: None,
                message: None,
            }
        },
        is_ready: is_running,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: cluster::ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

pub(in crate::app_shell) fn pod(name: &str, containers: Vec<ContainerSummary>) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        containers,
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
    }
}

/// A report that allows every check except those listed.
pub(in crate::app_shell) fn report_denying(denied: &[AccessCheck]) -> AccessReport {
    AccessReport {
        reviews: AccessCheck::ALL
            .into_iter()
            .map(|check| AccessReview {
                check,
                decision: if denied.contains(&check) {
                    AccessDecision::Denied { reason: None }
                } else {
                    AccessDecision::Allowed
                },
            })
            .collect(),
    }
}

pub(in crate::app_shell) struct Debugs {
    pub(in crate::app_shell) fixture: SwitchFixture,
    /// How every fake cluster of this test answers.
    answers: Answers,
    /// The fake server of `stg-b`, the cluster the fixture ends on.
    pub(in crate::app_shell) stg_api: FakeApi,
    pub(in crate::app_shell) prod: ClusterRef,
    pub(in crate::app_shell) stg: ClusterRef,
}

pub(in crate::app_shell) fn go_live(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    answers: Answers,
    cx: &mut TestAppContext,
) -> FakeApi {
    go_live_answering(
        fixture,
        cluster,
        move |request| respond(answers, request),
        cx,
    )
}

/// `go_live` over a fake server that answers with `respond`, for a test that holds an answer back.
pub(in crate::app_shell) fn go_live_answering(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    respond: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static,
    cx: &mut TestAppContext,
) -> FakeApi {
    // The client's worker is a tokio task, so it must be built inside the runtime.
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        FakeApi::connection(WritePolicy::Allowed, respond)
    };
    let session = fixture
        .shell
        .read_with(cx, |shell, _| shell.session_of(cluster).cloned())
        .expect("a viewed slot");
    let pods = vec![
        pod("api-0", vec![container("app", ContainerKind::Main, true)]),
        pod(
            "multi-0",
            vec![
                container("init", ContainerKind::Init, false),
                container("proxy", ContainerKind::Sidecar, true),
                container("web", ContainerKind::Main, true),
            ],
        ),
        pod("idle-0", vec![container("app", ContainerKind::Main, false)]),
    ];
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
        session.set_access_for_test(AccessState::Known(report_denying(&[])), cx);
        session.set_pods_for_test(pods, cx);
        session.set_nodes_for_test(vec![node("wk-03", "linux"), node("win-01", "windows")], cx);
    });
    cx.run_until_parked();
    api
}

pub(in crate::app_shell) fn two_clusters(
    name: &str,
    answers: Answers,
    cx: &mut TestAppContext,
) -> Debugs {
    let fixture = open_switch_fixture(name, cx);
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let stg_api = go_live(&fixture, &stg, answers, cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    Debugs {
        fixture,
        answers,
        stg_api,
        prod,
        stg,
    }
}

/// The patches a cluster received: the subresource requests only.
pub(in crate::app_shell) fn patches(api: &FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| request.path.ends_with("/ephemeralcontainers"))
        .collect()
}

pub(in crate::app_shell) fn commits(api: &FakeApi) -> Vec<RecordedRequest> {
    patches(api)
        .into_iter()
        .filter(|request| !request.has_query_key("dryRun"))
        .collect()
}

impl Debugs {
    /// Switches to `cluster` and makes it live over a new fake server. The old session is gone, so
    /// a test never has both clusters live at once.
    pub(in crate::app_shell) fn activate(
        &self,
        cluster: &ClusterRef,
        cx: &mut TestAppContext,
    ) -> FakeApi {
        self.fixture
            .shell
            .update(cx, |shell, cx| shell.switch_cluster(cluster, cx));
        cx.run_until_parked();
        let api = go_live(&self.fixture, cluster, self.answers, cx);
        self.fixture
            .shell
            .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
        cx.run_until_parked();
        self.fixture.draw_twice(cx);
        api
    }

    pub(in crate::app_shell) fn pod_ref(&self, cluster: &ClusterRef, pod: &str) -> DebugPod {
        DebugPod {
            cluster: cluster.clone(),
            namespace: "shop".to_owned(),
            pod: pod.to_owned(),
        }
    }

    /// The dialog of Debug container… on `pod`, with `image` and the container the options chose.
    pub(in crate::app_shell) fn start(
        &self,
        cluster: &ClusterRef,
        pod: &str,
        target_container: &str,
        image: &str,
        cx: &mut TestAppContext,
    ) {
        let pod = self.pod_ref(cluster, pod);
        let chosen = DebugChosen {
            target_container: target_container.to_owned(),
            image: image.to_owned(),
        };
        self.fixture.with_window(cx, |window, cx| {
            self.fixture.shell.update(cx, |shell, cx| {
                shell.start_debug_container(&pod, chosen, window, cx);
            });
        });
    }

    pub(in crate::app_shell) fn dialog(&self, cx: &mut TestAppContext) -> Entity<ConfirmDialog> {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.last_dialog.clone())
            .and_then(|dialog| dialog.upgrade())
            .expect("a confirm dialog is open")
    }

    pub(in crate::app_shell) fn has_dialog(&self, cx: &mut TestAppContext) -> bool {
        self.fixture
            .with_window(cx, |window, cx| window.has_active_dialog(cx))
    }

    /// Waits for the dry-run of the open dialog to pass, then presses the confirm button, typing
    /// the cluster name first when the tier asks for it.
    pub(in crate::app_shell) fn confirm(&self, cx: &mut TestAppContext) {
        let dialog = self.dialog(cx);
        self.wait_for_dry_run(&dialog, cx);
        let tier = dialog.read_with(cx, |dialog, _| dialog.tier().clone());
        self.fixture.with_window(cx, |window, cx| {
            dialog.update(cx, |dialog, cx| {
                if let DialogConfirm::TypeName { expected } = &tier {
                    dialog.type_text(expected, window, cx);
                }
                dialog.press_confirm(window, cx);
            });
        });
    }

    pub(in crate::app_shell) fn wait_for_dry_run(
        &self,
        dialog: &Entity<ConfirmDialog>,
        cx: &mut TestAppContext,
    ) {
        for _ in 0..1_500 {
            cx.run_until_parked();
            let state = dialog.read_with(cx, |dialog, _| dialog.dry_run_state());
            if matches!(state, Some(DryRunState::Passed { .. })) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for the dry-run");
    }

    pub(in crate::app_shell) fn wait_for(
        &self,
        what: &str,
        cx: &mut TestAppContext,
        done: impl Fn(&mut TestAppContext) -> bool,
    ) {
        for _ in 0..1_500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    pub(in crate::app_shell) fn tab_count(&self, cx: &mut TestAppContext) -> usize {
        self.fixture
            .shell
            .read_with(cx, |shell, cx| shell.dock.read(cx).tab_count())
    }

    pub(in crate::app_shell) fn tabs(&self, cx: &mut TestAppContext) -> Vec<Entity<ShellTab>> {
        self.fixture
            .shell
            .read_with(cx, |shell, cx| shell.dock.read(cx).shell_tab_entities())
    }

    pub(in crate::app_shell) fn set_lock(
        &self,
        cluster: &ClusterRef,
        lock: WriteLock,
        cx: &mut TestAppContext,
    ) {
        let session = self.session(cluster, cx);
        session.update(cx, |session, cx| session.set_lock(lock, cx));
        cx.run_until_parked();
    }

    pub(in crate::app_shell) fn set_access(
        &self,
        cluster: &ClusterRef,
        access: AccessReport,
        cx: &mut TestAppContext,
    ) {
        let session = self.session(cluster, cx);
        session.update(cx, |session, cx| {
            session.set_access_for_test(AccessState::Known(access), cx);
        });
        cx.run_until_parked();
    }

    pub(in crate::app_shell) fn session(
        &self,
        cluster: &ClusterRef,
        cx: &mut TestAppContext,
    ) -> Entity<ClusterSession> {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.session_of(cluster).cloned())
            .expect("a viewed slot")
    }

    /// Settings that save into a fresh folder, so the audit log has one.
    pub(in crate::app_shell) fn audit_folder(
        &self,
        name: &str,
        cx: &mut TestAppContext,
    ) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("k8sboard-0037-{name}-{}", std::process::id()));
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

pub(in crate::app_shell) fn audit_lines(dir: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(crate::audit_log::audit_path(dir))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON line"))
        .collect()
}

// ---- the guarded start ----

#[gpui_kit::test]
fn a_debug_container_always_asks_and_checks_before_anything_is_changed(cx: &mut TestAppContext) {
    let debugs = two_clusters("asks", Answers::Accepts, cx);
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    // A click tier still shows a dialog, and nothing has been committed or opened behind it.
    assert!(debugs.has_dialog(cx));
    assert_eq!(debugs.tab_count(cx), 0);
    let dialog = debugs.dialog(cx);
    debugs.wait_for_dry_run(&dialog, cx);
    let tier = dialog.read_with(cx, |dialog, _| dialog.tier().clone());
    assert_eq!(tier, DialogConfirm::Click);
    let patches = patches(&debugs.stg_api);
    assert_eq!(patches.len(), 1, "one dry-run and no commit");
    assert!(patches[0].has_query("dryRun", "All"));
    assert!(commits(&debugs.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_dialog_lists_what_the_patch_changes_and_the_ephemeral_warning(cx: &mut TestAppContext) {
    let debugs = two_clusters("lists", Answers::Accepts, cx);
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let dialog = debugs.dialog(cx);
    let (label, warnings, button) = dialog.read_with(cx, |dialog, _| {
        (
            dialog.label(),
            dialog.warning_lines(),
            dialog.confirm_text(),
        )
    });
    assert_eq!(label.as_deref(), Some("Add debug container to multi-0"));
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].starts_with("Ephemeral containers cannot be removed."));
    assert_eq!(button.as_deref(), Some("Add debug container"));
}

#[gpui_kit::test]
fn create_then_attach_opens_after_commit(cx: &mut TestAppContext) {
    let debugs = two_clusters("opens", Answers::Accepts, cx);
    let dir = debugs.audit_folder("opens", cx);
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the tab", cx, |cx| debugs.tab_count(cx) > 0);
    assert_eq!(debugs.tab_count(cx), 1);
    // The patch reached the pod's own cluster, strategic-merge, with the field manager.
    let commits = commits(&debugs.stg_api);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].method, "PATCH");
    assert_eq!(
        commits[0].path,
        "/api/v1/namespaces/shop/pods/multi-0/ephemeralcontainers"
    );
    assert_eq!(
        commits[0].content_type.as_deref(),
        Some("application/strategic-merge-patch+json")
    );
    assert!(commits[0].has_query("fieldManager", "k8sboard"));
    let tab = debugs.tabs(cx).remove(0);
    let (cluster, label, kind) = tab.read_with(cx, |tab, _| {
        (tab.cluster().clone(), tab.label(), tab.kind().clone())
    });
    assert_eq!(cluster, debugs.stg);
    assert_eq!(label, "debug · multi-0/web");
    match kind {
        ShellKind::Debug {
            target_container,
            image,
        } => {
            assert_eq!(target_container, "web");
            assert_eq!(image, cluster::DEFAULT_DEBUG_IMAGE);
        }
        other => panic!("expected a debug tab, got {other:?}"),
    }
    // One line for the patch, one for the attach start (the fake never upgrades, so it fails).
    debugs.wait_for("both lines", cx, |_| audit_lines(&dir).len() == 2);
    let lines = audit_lines(&dir);
    assert_eq!(lines[0]["action"], "Add debug container");
    assert_eq!(lines[0]["outcome"], "applied");
    assert_eq!(lines[0]["cluster"], "stg-b");
    assert_eq!(lines[0]["object"]["kind"], "Pod");
    assert_eq!(lines[0]["object"]["name"], "multi-0");
    assert_eq!(lines[1]["action"], "Open debug shell");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn create_then_attach_takes_the_permit_before_commit(cx: &mut TestAppContext) {
    let debugs = two_clusters("permit", Answers::Accepts, cx);
    let dir = debugs.audit_folder("permit", cx);
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let dialog = debugs.dialog(cx);
    debugs.wait_for_dry_run(&dialog, cx);
    // The right to attach goes away while the dialog is open.
    debugs.set_access(
        &debugs.stg,
        report_denying(&[AccessCheck::GetPodAttach]),
        cx,
    );
    debugs.confirm(cx);
    cx.run_until_parked();
    assert!(
        commits(&debugs.stg_api).is_empty(),
        "no permit, nothing is patched"
    );
    assert_eq!(debugs.tab_count(cx), 0);
    assert!(
        audit_lines(&dir).is_empty(),
        "nothing sent, nothing audited"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_commit_error_opens_nothing_and_is_audited_as_failed(cx: &mut TestAppContext) {
    let debugs = two_clusters("refused", Answers::RefusesCommit, cx);
    let dir = debugs.audit_folder("refused", cx);
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the failed line", cx, |_| audit_lines(&dir).len() == 1);
    assert_eq!(debugs.tab_count(cx), 0, "a refused patch opens no tab");
    assert_eq!(commits(&debugs.stg_api).len(), 1);
    assert_eq!(audit_lines(&dir)[0]["outcome"], "failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_locked_cluster_offers_no_debug_container(cx: &mut TestAppContext) {
    let debugs = two_clusters("locked", Answers::Accepts, cx);
    let prod_api = debugs.activate(&debugs.prod, cx);
    // prod-a is locked at open: no dialog, no request.
    debugs.start(
        &debugs.prod,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    assert!(!debugs.has_dialog(cx));
    assert!(patches(&prod_api).is_empty());
    assert_eq!(debugs.tab_count(cx), 0);
}

#[gpui_kit::test]
fn the_lock_is_checked_again_at_confirm_time(cx: &mut TestAppContext) {
    let debugs = two_clusters("locked-late", Answers::Accepts, cx);
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let dialog = debugs.dialog(cx);
    debugs.wait_for_dry_run(&dialog, cx);
    debugs.set_lock(&debugs.stg, WriteLock::Locked, cx);
    debugs.confirm(cx);
    cx.run_until_parked();
    assert!(commits(&debugs.stg_api).is_empty());
    assert_eq!(debugs.tab_count(cx), 0);
}

#[gpui_kit::test]
fn the_options_dialog_checks_the_gate_and_the_pod_again(cx: &mut TestAppContext) {
    let debugs = two_clusters("options-gate", Answers::Accepts, cx);
    let open = |pod: &str, cx: &mut TestAppContext| {
        let pod = debugs.pod_ref(&debugs.stg, pod);
        debugs.fixture.with_window(cx, |window, cx| {
            debugs.fixture.shell.update(cx, |shell, cx| {
                shell.open_debug_options(pod, None, window, cx);
            });
        });
    };
    // A pod with no running container, and a pod that is not listed: a notice, no dialog.
    open("idle-0", cx);
    assert!(!debugs.has_dialog(cx));
    open("gone-0", cx);
    assert!(!debugs.has_dialog(cx));
    // A pod with a running container opens the options dialog.
    open("multi-0", cx);
    assert!(debugs.has_dialog(cx));
    // A denied right closes the door even for a pod that qualifies.
    debugs
        .fixture
        .with_window(cx, |window, cx| window.close_dialog(cx));
    debugs.set_access(
        &debugs.stg,
        report_denying(&[AccessCheck::PatchPodEphemeralContainers]),
        cx,
    );
    open("multi-0", cx);
    assert!(!debugs.has_dialog(cx));
}

#[gpui_kit::test]
fn options_persist_image_after_start_and_only_when_changed(cx: &mut TestAppContext) {
    let debugs = two_clusters("persist", Answers::Accepts, cx);
    let dir = debugs.audit_folder("persist", cx);
    let stored = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            AppSettings::get(cx)
                .registry
                .clusters
                .iter()
                .find(|entry| entry.cluster == debugs.stg)
                .and_then(|entry| entry.debug_image.clone())
        })
    };
    // The default image is not written down.
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the first tab", cx, |cx| debugs.tab_count(cx) == 1);
    assert_eq!(stored(cx), None);
    // A mirror image is, once the start went through.
    debugs.start(&debugs.stg, "multi-0", "web", MIRROR_IMAGE, cx);
    assert_eq!(stored(cx), None, "not before the start succeeded");
    debugs.confirm(cx);
    debugs.wait_for("the second tab", cx, |cx| debugs.tab_count(cx) == 2);
    assert_eq!(stored(cx).as_deref(), Some(MIRROR_IMAGE));
    // The next dialog opens with it.
    let image = cx.read(|cx| {
        debugs
            .fixture
            .shell
            .read(cx)
            .guard_for(&debugs.stg, cx)
            .map(|guard| guard.profile.debug_image.clone())
    });
    assert_eq!(image.as_deref(), Some(MIRROR_IMAGE));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_refused_start_stores_nothing(cx: &mut TestAppContext) {
    let debugs = two_clusters("persist-refused", Answers::RefusesCommit, cx);
    let dir = debugs.audit_folder("persist-refused", cx);
    debugs.start(&debugs.stg, "multi-0", "web", MIRROR_IMAGE, cx);
    debugs.confirm(cx);
    debugs.wait_for("the failed line", cx, |_| audit_lines(&dir).len() == 1);
    let entries = cx.read(|cx| AppSettings::get(cx).registry.clusters.len());
    assert_eq!(entries, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_debug_tab_closed_before_its_attach_reports_is_audited_as_abandoned(cx: &mut TestAppContext) {
    let debugs = two_clusters("abandon", Answers::Accepts, cx);
    let dir = debugs.audit_folder("abandon", cx);
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the failed start", cx, |_| audit_lines(&dir).len() == 2);
    let tab = debugs.tabs(cx).remove(0);
    // A new attach start that has not reported when the tab goes.
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.begin_shell_start(&tab, cluster::ShellCommand::Auto, cx);
    });
    drop(tab);
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.dock.update(cx, |dock, cx| dock.close_all(cx));
    });
    debugs.wait_for("the abandoned line", cx, |_| audit_lines(&dir).len() == 3);
    let lines = audit_lines(&dir);
    assert_eq!(lines[2]["outcome"], "abandoned");
    assert_eq!(lines[2]["action"], "Open debug shell");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- the menu item ----

/// The shell items of `pod` on `cluster`, as the row menu builds them.
fn menu_items(
    debugs: &Debugs,
    cluster: &ClusterRef,
    pod_name: &str,
    cx: &mut TestAppContext,
) -> crate::resource_actions::ShellItems {
    let (menu, row) = debugs.fixture.shell.read_with(cx, |shell, cx| {
        let guard = shell.guard_for(cluster, cx).expect("a guard");
        let live = shell.live_of(cluster, cx).expect("a live slot");
        let pod = live
            .pods
            .items()
            .iter()
            .find(|pod| pod.name == pod_name)
            .expect("the pod is listed");
        let row = shell.row_context_of(cluster, cx).expect("a slot");
        (crate::resource_actions::ShellMenu::of(pod, &guard), row)
    });
    let weak = debugs.fixture.shell.downgrade();
    debugs
        .fixture
        .with_window(cx, |window, cx| menu.items(&row, &weak, window, cx))
}

#[gpui_kit::test]
fn debug_container_item_is_last_in_the_shell_submenu(cx: &mut TestAppContext) {
    let debugs = two_clusters("menu-pick", Answers::Accepts, cx);
    // Open shell has a submenu, which holds Debug container… after a separator.
    let items = menu_items(&debugs, &debugs.stg, "multi-0", cx);
    assert!(items.debug_container.is_none());
}

#[gpui_kit::test]
fn a_pod_with_one_container_gets_the_debug_item_beside_open_shell(cx: &mut TestAppContext) {
    let debugs = two_clusters("menu-one", Answers::Accepts, cx);
    let stg = &debugs.stg;
    assert!(
        menu_items(&debugs, stg, "api-0", cx)
            .debug_container
            .is_some()
    );
    // Also when Open shell itself is off: the item stays visible, with its own reason.
    assert!(
        menu_items(&debugs, stg, "idle-0", cx)
            .debug_container
            .is_some()
    );
}

#[gpui_kit::test]
fn the_debug_item_reads_the_gate_of_its_own_cluster(cx: &mut TestAppContext) {
    let debugs = two_clusters("menu-gate", Answers::Accepts, cx);
    let state = |cluster: &ClusterRef, cx: &mut TestAppContext| {
        debugs.fixture.shell.read_with(cx, |shell, cx| {
            let guard = shell.guard_for(cluster, cx).expect("a guard");
            let live = shell.live_of(cluster, cx).expect("a live slot");
            let pod = live
                .pods
                .items()
                .iter()
                .find(|pod| pod.name == "multi-0")
                .expect("the pod is listed");
            crate::resource_actions::debug_menu_state(pod, &guard)
        })
    };
    // prod-a is locked, stg-b is not: one pod name, two answers, each from the active session.
    assert_eq!(
        state(&debugs.stg, cx),
        crate::resource_actions::DebugMenuState::Ready
    );
    debugs.activate(&debugs.prod, cx);
    assert_eq!(
        state(&debugs.prod, cx),
        crate::resource_actions::DebugMenuState::Disabled("prod-a is read-only".into())
    );
}

#[gpui_kit::test]
fn reconnect_of_a_debug_tab_reopens_the_options_and_reuses_nothing(cx: &mut TestAppContext) {
    let debugs = two_clusters("reconnect", Answers::Accepts, cx);
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the tab", cx, |cx| debugs.tab_count(cx) == 1);
    let tab = debugs.tabs(cx).remove(0);
    assert!(
        debugs
            .fixture
            .shell
            .read_with(cx, |shell, cx| shell.is_debug_tab(&tab.downgrade(), cx))
    );
    let first_container = tab.read_with(cx, |tab, _| tab.target().container.clone());
    // The old confirm dialog is gone; Reconnect asks again through the options dialog.
    debugs
        .fixture
        .with_window(cx, |window, cx| window.close_all_dialogs(cx));
    assert!(!debugs.has_dialog(cx));
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.reopen_debug_options(&tab.downgrade(), window, cx);
        });
    });
    assert!(debugs.has_dialog(cx), "the options dialog, prefilled");
    // Nothing was sent: the new container is added only after the options and the confirm.
    assert_eq!(commits(&debugs.stg_api).len(), 1);
    // A second start makes a new container name.
    debugs
        .fixture
        .with_window(cx, |window, cx| window.close_all_dialogs(cx));
    debugs.start(
        &debugs.stg,
        "multi-0",
        "web",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the second tab", cx, |cx| debugs.tab_count(cx) == 2);
    let second_container = debugs
        .tabs(cx)
        .remove(1)
        .read_with(cx, |tab, _| tab.target().container.clone());
    assert_ne!(first_container, second_container);
}

// ---- a menu built on one cluster outlives a switch (spec 0046, write-safety.md I9) ----

/// Clicks `item`, as the open menu would.
fn click(item: &PopupMenuItem, debugs: &Debugs, cx: &mut TestAppContext) {
    debugs.fixture.with_window(cx, |window, cx| match item {
        PopupMenuItem::Item {
            handler: Some(handler),
            ..
        }
        | PopupMenuItem::ElementItem {
            handler: Some(handler),
            ..
        } => handler(&gpui_kit::ClickEvent::default(), window, cx),
        _ => panic!("the item has no click handler"),
    });
}

/// Clicks every action of the pod menu that was built on `stg-b` before a switch.
fn click_menu_built_on_stg(
    items: &crate::resource_actions::ShellItems,
    debugs: &Debugs,
    cx: &mut TestAppContext,
) {
    click(&items.open_shell, debugs, cx);
    click(
        items.debug_container.as_ref().expect("the debug item"),
        debugs,
        cx,
    );
}

#[gpui_kit::test]
fn a_menu_built_on_a_acts_on_nothing_after_a_switch(cx: &mut TestAppContext) {
    let debugs = two_clusters("menu-after-switch", Answers::Accepts, cx);
    let items = menu_items(&debugs, &debugs.stg, "api-0", cx);
    let prod_api = debugs.activate(&debugs.prod, cx);
    click_menu_built_on_stg(&items, &debugs, cx);
    assert!(!debugs.has_dialog(cx), "a stale menu opens no dialog");
    assert_eq!(debugs.tab_count(cx), 0);
    assert!(patches(&prod_api).is_empty());
    assert!(commits(&debugs.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_menu_built_on_a_does_nothing_after_switching_back(cx: &mut TestAppContext) {
    let debugs = two_clusters("menu-after-back", Answers::Accepts, cx);
    let items = menu_items(&debugs, &debugs.stg, "api-0", cx);
    let row = debugs
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.row_context_of(&debugs.stg, cx))
        .expect("the open cluster has a row context");
    debugs.activate(&debugs.prod, cx);
    let stg_again = debugs.activate(&debugs.stg, cx);
    // The same cluster is open again, but it is a new session: the menu's one is gone.
    assert!(row.session.upgrade().is_none());
    click_menu_built_on_stg(&items, &debugs, cx);
    assert!(!debugs.has_dialog(cx), "a stale menu opens no dialog");
    assert_eq!(debugs.tab_count(cx), 0);
    assert!(patches(&stg_again).is_empty());
    assert!(commits(&stg_again).is_empty());
}

/// A Secrets row with one key, so Reveal is enabled.
fn secret_row() -> crate::kind_row::KindRow {
    let mut secret = crate::topology_fixtures::secret("db", "Opaque");
    secret.keys.push(cluster::SecretKey {
        name: "password".to_owned(),
        size_bytes: 8,
        is_binary: false,
    });
    crate::kind_row::KindRow {
        namespace: Some("shop".to_owned()),
        name: "db".to_owned(),
        created_at: None,
        status: crate::status_tone::StatusLabel {
            text: "Opaque".into(),
            tone: crate::status_tone::StatusTone::Ok,
        },
        cells: Vec::new(),
        sections: Vec::new(),
        related_pods: None,
        event: None,
        labels: Vec::new(),
        object: crate::kind_row::KindObject::Secret(secret),
    }
}

/// The Reveal item of the `db` Secret, as the row menu of `cluster` builds it now.
fn reveal_item(debugs: &Debugs, cluster: &ClusterRef, cx: &mut TestAppContext) -> PopupMenuItem {
    let row = secret_row();
    let context = debugs
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.row_context_of(cluster, cx))
        .expect("the open cluster has a row context");
    let object = context.object(crate::table_selection::ResourceKey::Kind {
        kind: crate::resource_kind::ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "db".to_owned(),
    });
    let weak = debugs.fixture.shell.downgrade();
    let [reveal, _copy] = debugs.fixture.with_window(cx, |window, cx| {
        crate::resource_actions::secret_menu(
            &row,
            &context,
            object,
            crate::secret_values::ValueAccess::Enabled,
            &weak,
            window,
            cx,
        )
        .expect("a Secret row has a menu")
        .into_items()
    });
    reveal
}

#[gpui_kit::test]
fn a_secret_reveal_built_on_a_does_nothing_after_switching_back(cx: &mut TestAppContext) {
    let debugs = two_clusters("secret-after-back", Answers::Accepts, cx);
    let stale = reveal_item(&debugs, &debugs.stg, cx);
    debugs.activate(&debugs.prod, cx);
    debugs.activate(&debugs.stg, cx);
    // A reveal goes to the Secrets screen first.
    let is_on_secrets = |cx: &mut TestAppContext| {
        debugs.fixture.shell.read_with(cx, |shell, _| {
            shell.screen == Screen::Kind(crate::resource_kind::ResourceKind::Secrets)
        })
    };
    assert!(!is_on_secrets(cx));
    click(&stale, &debugs, cx);
    assert!(!is_on_secrets(cx), "a stale menu reveals nothing");
    // The same item, built on the new session, does act.
    let fresh = reveal_item(&debugs, &debugs.stg, cx);
    click(&fresh, &debugs, cx);
    assert!(is_on_secrets(cx));
}

#[gpui_kit::test]
fn a_logs_item_built_on_a_opens_nothing_after_switching_back(cx: &mut TestAppContext) {
    let debugs = two_clusters("logs-after-back", Answers::Accepts, cx);
    let build = |debugs: &Debugs, cx: &mut TestAppContext| {
        let (menu, connection, row, dock) = debugs.fixture.shell.read_with(cx, |shell, cx| {
            let live = shell.live_of(&debugs.stg, cx).expect("a live slot");
            let pod = live
                .pods
                .items()
                .iter()
                .find(|pod| pod.name == "api-0")
                .expect("the pod is listed");
            (
                crate::resource_actions::LogsMenu::of(pod, &live.access),
                live.connection().clone(),
                shell
                    .row_context_of(&debugs.stg, cx)
                    .expect("a row context"),
                shell.dock.downgrade(),
            )
        });
        debugs.fixture.with_window(cx, |window, cx| {
            menu.item(connection, &row, &dock, window, cx)
        })
    };
    let stale = build(&debugs, cx);
    debugs.activate(&debugs.prod, cx);
    debugs.activate(&debugs.stg, cx);
    click(&stale, &debugs, cx);
    let has_tabs = |cx: &mut TestAppContext| {
        debugs
            .fixture
            .shell
            .read_with(cx, |shell, cx| shell.dock.read(cx).has_tabs())
    };
    assert!(!has_tabs(cx), "a stale menu opens no log tab");
    // The same item, built on the new session, does open one.
    let fresh = build(&debugs, cx);
    click(&fresh, &debugs, cx);
    assert!(has_tabs(cx));
}
