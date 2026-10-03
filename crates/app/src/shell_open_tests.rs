//! Opening, confirming, auditing, and releasing shell tabs in a headless window over two viewed
//! clusters: `prod-a` (the primary, Production, locked at open) and `stg-b` (unlocked, a click
//! tier). Each cluster answers from its own fake API server, so a test sees which cluster a request
//! reached and nothing leaves the machine. No test sets `K8SBOARD_ALLOW_WRITES`; the fake
//! connections carry their own write policy, and a fake never upgrades a connection to a stream.

use std::path::PathBuf;
use std::time::Duration;

use cluster::fake_api::{FakeApi, RecordedRequest};
use cluster::{
    AccessCheck, AccessDecision, AccessReport, AccessReview, ContainerKind, ContainerState,
    ContainerSummary, NamespaceScope, PodStatus, PodSummary, ReadyCount, StatusReason, WritePolicy,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{Cancel, Confirm};
use gpui_kit::{Entity, TestAppContext, WeakEntity};

use super::super::write_flow::DryRunState;
use super::*;
use crate::app_shell::Screen;
use crate::app_shell::app_shell_switch_tests::{SwitchFixture, open_switch_fixture};
use crate::cluster_session::{AccessState, ClusterSession};
use crate::confirm_dialog::ConfirmDialog;
use crate::resource_actions::RowAction;
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};
use crate::write_guard::{DialogConfirm, WriteLock};

const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;

fn container(name: &str, kind: ContainerKind, is_running: bool) -> ContainerSummary {
    ContainerSummary {
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

fn pod(name: &str, containers: Vec<ContainerSummary>) -> PodSummary {
    PodSummary {
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
    }
}

/// A report that allows every check except those listed.
fn report_denying(denied: &[AccessCheck]) -> AccessReport {
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

struct Shells {
    fixture: SwitchFixture,
    prod_api: FakeApi,
    stg_api: FakeApi,
    prod: ClusterRef,
    stg: ClusterRef,
}

fn slot_session(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    cx: &mut TestAppContext,
) -> Entity<ClusterSession> {
    fixture
        .shell
        .read_with(cx, |shell, _| shell.slot_session(cluster).cloned())
        .expect("a viewed slot")
}

fn go_live(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    pods: Vec<PodSummary>,
    cx: &mut TestAppContext,
) -> FakeApi {
    // The client's worker is a tokio task, so it must be built inside the runtime.
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        FakeApi::connection(WritePolicy::Allowed, |_| (404, NOT_FOUND.to_owned()))
    };
    let session = slot_session(fixture, cluster, cx);
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
        session.set_access_for_test(AccessState::Known(report_denying(&[])), cx);
        session.set_pods_for_test(pods, cx);
    });
    cx.run_until_parked();
    api
}

fn two_clusters(name: &str, cx: &mut TestAppContext) -> Shells {
    let fixture = open_switch_fixture(name, cx);
    let wanted = [fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx)];
    fixture
        .shell
        .update(cx, |shell, cx| shell.view_clusters(&wanted, cx));
    cx.run_until_parked();
    let [prod, stg] = wanted;
    let pods = || {
        vec![
            pod("api-0", vec![container("app", ContainerKind::Main, true)]),
            pod(
                "multi-0",
                vec![
                    container("init", ContainerKind::Init, false),
                    container("proxy", ContainerKind::Sidecar, true),
                    container("web", ContainerKind::Main, true),
                ],
            ),
        ]
    };
    let prod_api = go_live(&fixture, &prod, pods(), cx);
    let stg_api = go_live(&fixture, &stg, pods(), cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    Shells {
        fixture,
        prod_api,
        stg_api,
        prod,
        stg,
    }
}

fn exec_requests(api: &FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| request.path.ends_with("/exec"))
        .collect()
}

impl Shells {
    fn open(&self, cluster: &ClusterRef, pod: &str, container: &str, cx: &mut TestAppContext) {
        let open = ShellOpen {
            cluster: cluster.clone(),
            namespace: "shop".to_owned(),
            pod: pod.to_owned(),
            container: container.to_owned(),
        };
        self.fixture.with_window(cx, |window, cx| {
            self.fixture
                .shell
                .update(cx, |shell, cx| shell.start_shell(open, window, cx));
        });
    }

    fn dialog(&self, cx: &mut TestAppContext) -> Entity<ConfirmDialog> {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.last_dialog.clone())
            .and_then(|dialog| dialog.upgrade())
            .expect("a confirm dialog is open")
    }

    fn has_dialog(&self, cx: &mut TestAppContext) -> bool {
        self.fixture
            .with_window(cx, |window, cx| window.has_active_dialog(cx))
    }

    /// Presses the confirm button, typing the cluster name first when the tier asks for it.
    fn confirm(&self, cx: &mut TestAppContext) {
        let dialog = self.dialog(cx);
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

    fn open_and_confirm(
        &self,
        cluster: &ClusterRef,
        pod: &str,
        container: &str,
        cx: &mut TestAppContext,
    ) {
        self.open(cluster, pod, container, cx);
        self.confirm(cx);
    }

    fn tabs_of(&self, cluster: &ClusterRef, cx: &mut TestAppContext) -> usize {
        self.fixture.shell.read_with(cx, |shell, cx| {
            shell
                .dock
                .read(cx)
                .shell_count_of(std::slice::from_ref(cluster), cx)
        })
    }

    fn tab_count(&self, cx: &mut TestAppContext) -> usize {
        self.fixture
            .shell
            .read_with(cx, |shell, cx| shell.dock.read(cx).tab_count())
    }

    fn set_lock(&self, cluster: &ClusterRef, lock: WriteLock, cx: &mut TestAppContext) {
        let session = slot_session(&self.fixture, cluster, cx);
        session.update(cx, |session, cx| session.set_lock(lock, cx));
        cx.run_until_parked();
    }

    fn set_access(&self, cluster: &ClusterRef, access: AccessReport, cx: &mut TestAppContext) {
        let session = slot_session(&self.fixture, cluster, cx);
        session.update(cx, |session, cx| {
            session.set_access_for_test(AccessState::Known(access), cx);
        });
        cx.run_until_parked();
    }

    fn wait_for(&self, what: &str, cx: &mut TestAppContext, done: impl Fn() -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    fn press_dialog(&self, answer: impl gpui_kit::Action, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            window.dispatch_action(Box::new(answer), cx);
        });
    }

    /// The tabs opened so far, newest last.
    fn tabs(&self, cx: &mut TestAppContext) -> Vec<Entity<ShellTab>> {
        self.fixture.shell.read_with(cx, |shell, cx| {
            shell
                .dock
                .read(cx)
                .shell_tab_entities()
                .into_iter()
                .collect()
        })
    }

    /// Settings that save into a fresh folder, so the audit log has one.
    fn audit_folder(&self, name: &str, cx: &mut TestAppContext) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("k8sboard-0036-{name}-{}", std::process::id()));
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

fn audit_lines(dir: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(crate::audit_log::audit_path(dir))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON line"))
        .collect()
}

// ---- the guarded start ----

#[gpui_kit::test]
fn a_shell_always_asks_before_it_opens(cx: &mut TestAppContext) {
    let shells = two_clusters("asks", cx);
    shells.open(&shells.stg, "api-0", "app", cx);
    // A click tier still shows a dialog, and nothing has been sent or opened behind it.
    assert!(shells.has_dialog(cx));
    assert_eq!(shells.tab_count(cx), 0);
    assert!(exec_requests(&shells.stg_api).is_empty());
    let dialog = shells.dialog(cx);
    let (tier, dry_run) = dialog.read_with(cx, |dialog, _| {
        (dialog.tier().clone(), dialog.dry_run_state())
    });
    assert_eq!(tier, DialogConfirm::Click);
    assert_eq!(dry_run, Some(DryRunState::NotSupported));
}

#[gpui_kit::test]
fn open_shell_uses_the_cursor_slot(cx: &mut TestAppContext) {
    let shells = two_clusters("slot", cx);
    shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    assert_eq!(shells.tab_count(cx), 1);
    // The request reached stg-b, the pod's own cluster, and the primary saw none.
    shells.wait_for("the exec request", cx, || {
        !exec_requests(&shells.stg_api).is_empty()
    });
    assert!(exec_requests(&shells.prod_api).is_empty());
    let tab = shells.tabs(cx).remove(0);
    let (cluster, label) = tab.read_with(cx, |tab, _| {
        (tab.cluster().clone(), tab.cluster_label().to_owned())
    });
    assert_eq!(cluster, shells.stg);
    assert_eq!(label, "stg-b");
}

#[gpui_kit::test]
fn open_shell_follows_the_0030_gate(cx: &mut TestAppContext) {
    let shells = two_clusters("gate", cx);
    // prod-a is locked at open: no dialog, no tab, no request.
    shells.open(&shells.prod, "api-0", "app", cx);
    assert!(!shells.has_dialog(cx));
    assert_eq!(shells.tab_count(cx), 0);
    // A denied verb of the pair says so and opens nothing.
    shells.set_access(&shells.stg, report_denying(&[AccessCheck::GetPodExec]), cx);
    shells.open(&shells.stg, "api-0", "app", cx);
    assert!(!shells.has_dialog(cx));
    // Unlocked and allowed, a Production cluster types its name.
    shells.set_lock(&shells.prod, WriteLock::Unlocked, cx);
    shells.open(&shells.prod, "api-0", "app", cx);
    assert!(shells.has_dialog(cx));
    let tier = shells
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.tier().clone());
    assert_eq!(
        tier,
        DialogConfirm::TypeName {
            expected: "prod-a".to_owned()
        }
    );
    assert!(exec_requests(&shells.prod_api).is_empty());
    assert!(exec_requests(&shells.stg_api).is_empty());
}

#[gpui_kit::test]
fn start_connect_never_opens_when_the_dialog_is_cancelled(cx: &mut TestAppContext) {
    let shells = two_clusters("cancel", cx);
    shells.open(&shells.stg, "api-0", "app", cx);
    shells.press_dialog(Cancel, cx);
    assert!(!shells.has_dialog(cx));
    assert_eq!(shells.tab_count(cx), 0);
    assert!(exec_requests(&shells.stg_api).is_empty());
}

#[gpui_kit::test]
fn start_connect_never_opens_when_locked_at_confirm_time(cx: &mut TestAppContext) {
    let shells = two_clusters("locked-late", cx);
    let dir = shells.audit_folder("locked-late", cx);
    shells.open(&shells.stg, "api-0", "app", cx);
    shells.set_lock(&shells.stg, WriteLock::Locked, cx);
    shells.confirm(cx);
    assert_eq!(shells.tab_count(cx), 0);
    assert!(exec_requests(&shells.stg_api).is_empty());
    assert!(
        audit_lines(&dir).is_empty(),
        "nothing opened, nothing audited"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn start_connect_never_opens_without_a_permit_at_confirm_time(cx: &mut TestAppContext) {
    let shells = two_clusters("denied-late", cx);
    shells.open(&shells.stg, "api-0", "app", cx);
    shells.set_access(
        &shells.stg,
        report_denying(&[AccessCheck::CreatePodExec]),
        cx,
    );
    shells.confirm(cx);
    assert_eq!(shells.tab_count(cx), 0);
    assert!(exec_requests(&shells.stg_api).is_empty());
}

#[gpui_kit::test]
fn start_connect_audits_one_line_without_bytes(cx: &mut TestAppContext) {
    let shells = two_clusters("audit", cx);
    let dir = shells.audit_folder("audit", cx);
    shells.open_and_confirm(&shells.stg, "multi-0", "web", cx);
    shells.wait_for("the audit line", cx, || !audit_lines(&dir).is_empty());
    let lines = audit_lines(&dir);
    assert_eq!(lines.len(), 1, "one line per session start");
    let line = &lines[0];
    assert_eq!(line["cluster"], "stg-b");
    assert_eq!(line["action"], "Open shell");
    assert_eq!(line["object"]["kind"], "Pod");
    assert_eq!(line["object"]["namespace"], "shop");
    assert_eq!(line["object"]["name"], "multi-0");
    // The fake never upgrades a connection, so the start failed and says why.
    assert_eq!(line["outcome"], "failed");
    let fields: Vec<(&str, &str)> = line["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .map(|field| {
            (
                field["path"].as_str().expect("a path"),
                field["value"].as_str().expect("a value"),
            )
        })
        .collect();
    assert_eq!(fields, [("container", "web"), ("command", "auto")]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn an_exec_connection_that_came_up_is_audited_as_applied(cx: &mut TestAppContext) {
    let shells = two_clusters("applied", cx);
    let dir = shells.audit_folder("applied", cx);
    shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    // Wait out the failed start of the fake, then report an exec that came up.
    shells.wait_for("the failed start", cx, || !audit_lines(&dir).is_empty());
    let tab = shells.tabs(cx).remove(0);
    tab.update(cx, |tab, cx| {
        tab.mark_start_unreported_for_test();
        tab.apply(cluster::ShellUpdate::Started, cx);
    });
    shells.wait_for("the applied line", cx, || audit_lines(&dir).len() == 2);
    let lines = audit_lines(&dir);
    assert_eq!(lines[0]["outcome"], "failed");
    assert_eq!(lines[1]["outcome"], "applied");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- entry points ----

#[gpui_kit::test]
fn s_opens_the_default_container_of_the_cursor_pod_in_its_cluster(cx: &mut TestAppContext) {
    let shells = two_clusters("s-key", cx);
    let object = ClusterObject::new(
        shells.stg.clone(),
        ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: "multi-0".to_owned(),
        },
    );
    shells
        .fixture
        .shell
        .update(cx, |shell, cx| shell.change_selection(Some(object), cx));
    cx.run_until_parked();
    shells.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(crate::keymap::OpenShell), cx);
    });
    // The first running main container, not the sidecar before it or the init container.
    assert!(shells.has_dialog(cx));
    shells.confirm(cx);
    assert_eq!(shells.tabs_of(&shells.stg, cx), 1);
    assert_eq!(shells.tabs_of(&shells.prod, cx), 0);
    let tab = shells.tabs(cx).remove(0);
    let container = tab.read_with(cx, |tab, _| tab.target().container.clone());
    assert_eq!(container, "web");
}

#[gpui_kit::test]
fn s_menu_dock_and_palette_share_the_arm(cx: &mut TestAppContext) {
    let shells = two_clusters("arm", cx);
    // Nothing selected: the dock item says why, and the key does nothing.
    let reason = shells
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.selected_shell_reason(cx));
    assert_eq!(reason.as_deref(), Some("Select a pod first"));
    let object = ClusterObject::new(
        shells.stg.clone(),
        ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: "api-0".to_owned(),
        },
    );
    shells
        .fixture
        .shell
        .update(cx, |shell, cx| shell.change_selection(Some(object), cx));
    cx.run_until_parked();
    let reason = shells
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.selected_shell_reason(cx));
    assert_eq!(reason, None);
    // The dock item runs the same arm as the key, with the same result.
    shells.fixture.with_window(cx, |window, cx| {
        shells.fixture.shell.update(cx, |shell, cx| {
            shell.run_row_key(RowAction::OpenShell, window, cx);
        });
    });
    assert!(shells.has_dialog(cx));
    shells.press_dialog(Cancel, cx);
    // A denied pair is the same reason on every surface.
    shells.set_access(
        &shells.stg,
        report_denying(&[AccessCheck::CreatePodExec]),
        cx,
    );
    let reason = shells
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.selected_shell_reason(cx));
    assert_eq!(
        reason.as_deref(),
        Some("Not permitted: get and create pods/exec")
    );
}

// ---- the dock ----

#[gpui_kit::test]
fn ninth_shell_tab_is_refused(cx: &mut TestAppContext) {
    let shells = two_clusters("ninth", cx);
    for _ in 0..crate::dock::MAX_SHELL_TABS {
        shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    }
    assert_eq!(shells.tab_count(cx), crate::dock::MAX_SHELL_TABS);
    shells.open(&shells.stg, "api-0", "app", cx);
    assert!(
        !shells.has_dialog(cx),
        "the cap is checked before the dialog"
    );
    assert_eq!(shells.tab_count(cx), crate::dock::MAX_SHELL_TABS);
}

#[gpui_kit::test]
fn close_all_ends_shell_sessions(cx: &mut TestAppContext) {
    let shells = two_clusters("close-all", cx);
    shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    let tab: WeakEntity<ShellTab> = shells.tabs(cx).remove(0).downgrade();
    shells.fixture.shell.update(cx, |shell, cx| {
        shell.dock.update(cx, |dock, cx| dock.close_all(cx));
    });
    shells.wait_for("the tab to drop", cx, || tab.upgrade().is_none());
    assert_eq!(shells.tab_count(cx), 0);
}

// ---- releasing clusters with open shells ----

#[gpui_kit::test]
fn switch_with_open_shells_asks_first(cx: &mut TestAppContext) {
    let shells = two_clusters("switch-asks", cx);
    shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    shells.open_and_confirm(&shells.stg, "multi-0", "web", cx);
    assert_eq!(shells.tab_count(cx), 2);
    let target = shells.fixture.cluster("dev-c", cx);
    shells
        .fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    let lines = shells
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.last_leaving.clone());
    assert_eq!(lines, Some(vec!["2 shells will close".to_owned()]));
    assert!(shells.has_dialog(cx));
    // Nothing was released while the question is open, and Esc keeps everything.
    assert_eq!(shells.tab_count(cx), 2);
    shells.press_dialog(Cancel, cx);
    assert!(!shells.has_dialog(cx));
    assert_eq!(shells.tab_count(cx), 2);
    let viewed = shells
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.view.clusters());
    assert_eq!(viewed.len(), 2, "both clusters are still viewed");
    // Asking again and confirming releases them and ends the shells.
    shells
        .fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    shells.press_dialog(Confirm { secondary: false }, cx);
    assert_eq!(shells.tab_count(cx), 0);
}

#[gpui_kit::test]
fn a_switch_without_shells_asks_nothing(cx: &mut TestAppContext) {
    let shells = two_clusters("switch-quiet", cx);
    let target = shells.fixture.cluster("dev-c", cx);
    shells
        .fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    assert!(!shells.has_dialog(cx));
    let lines = shells
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.last_leaving.clone());
    assert_eq!(lines, None);
}

#[gpui_kit::test]
fn slot_release_closes_only_its_shells(cx: &mut TestAppContext) {
    let shells = two_clusters("release", cx);
    shells.set_lock(&shells.prod, WriteLock::Unlocked, cx);
    shells.open_and_confirm(&shells.prod, "api-0", "app", cx);
    shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    assert_eq!(
        (
            shells.tabs_of(&shells.prod, cx),
            shells.tabs_of(&shells.stg, cx)
        ),
        (1, 1)
    );
    shells
        .fixture
        .shell
        .update(cx, |shell, cx| shell.remove_from_view(&shells.stg, cx));
    cx.run_until_parked();
    let lines = shells
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.last_leaving.clone());
    assert_eq!(lines, Some(vec!["1 shell will close".to_owned()]));
    shells.press_dialog(Confirm { secondary: false }, cx);
    assert_eq!(
        shells.tabs_of(&shells.stg, cx),
        0,
        "the released slot's shell is gone"
    );
    assert_eq!(
        shells.tabs_of(&shells.prod, cx),
        1,
        "the other cluster's shell runs"
    );
}

#[gpui_kit::test]
fn a_view_change_that_keeps_every_shell_asks_nothing(cx: &mut TestAppContext) {
    let shells = two_clusters("view-keeps", cx);
    shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    let wanted = [shells.prod.clone(), shells.stg.clone()];
    shells
        .fixture
        .shell
        .update(cx, |shell, cx| shell.view_clusters(&wanted, cx));
    cx.run_until_parked();
    assert!(!shells.has_dialog(cx));
    assert_eq!(shells.tab_count(cx), 1);
}

// ---- reconnect ----

#[gpui_kit::test]
fn reconnect_asks_again_and_runs_in_the_same_tab(cx: &mut TestAppContext) {
    let shells = two_clusters("reconnect", cx);
    shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    shells.wait_for("the first exec", cx, || {
        exec_requests(&shells.stg_api).len() == 1
    });
    let tab = shells.tabs(cx).remove(0);
    let weak = tab.downgrade();
    shells.fixture.with_window(cx, |window, cx| {
        shells.fixture.shell.update(cx, |shell, cx| {
            shell.reconnect_shell(&weak, ShellCommand::Bash, window, cx);
        });
    });
    assert!(
        shells.has_dialog(cx),
        "Reconnect goes through the 0030 dialog"
    );
    assert_eq!(
        exec_requests(&shells.stg_api).len(),
        1,
        "nothing before the answer"
    );
    shells.confirm(cx);
    shells.wait_for("the second exec", cx, || {
        exec_requests(&shells.stg_api).len() == 2
    });
    assert_eq!(shells.tab_count(cx), 1, "a new session, not a new tab");
    assert_eq!(
        tab.read_with(cx, |tab, _| tab.command()),
        ShellCommand::Bash
    );
}

#[gpui_kit::test]
fn reconnect_is_blocked_while_the_cluster_is_locked(cx: &mut TestAppContext) {
    let shells = two_clusters("reconnect-locked", cx);
    shells.open_and_confirm(&shells.stg, "api-0", "app", cx);
    let weak = shells.tabs(cx).remove(0).downgrade();
    shells.set_lock(&shells.stg, WriteLock::Locked, cx);
    shells.fixture.with_window(cx, |window, cx| {
        shells.fixture.shell.update(cx, |shell, cx| {
            shell.reconnect_shell(&weak, ShellCommand::Auto, window, cx);
        });
    });
    assert!(!shells.has_dialog(cx));
}

#[test]
fn the_audit_fields_name_the_container_and_the_shell() {
    let target = ShellTarget {
        cluster: ClusterRef {
            kubeconfig: PathBuf::from("k.yaml"),
            context: "stg-b".to_owned(),
        },
        namespace: "shop".to_owned(),
        pod: "api-0".to_owned(),
        container: "app".to_owned(),
    };
    let (object, fields) = shell_audit(&target, ShellCommand::Bash);
    assert_eq!(object.kind, "Pod");
    assert_eq!(object.namespace.as_deref(), Some("shop"));
    let values: Vec<(&str, Option<&str>)> = fields
        .iter()
        .map(|field| (field.path.as_str(), field.value.as_deref()))
        .collect();
    assert_eq!(
        values,
        [("container", Some("app")), ("command", Some("bash"))]
    );
}
