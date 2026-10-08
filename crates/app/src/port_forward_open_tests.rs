//! Starting, stopping, auditing, and keeping port forwards in a headless window over two loaded
//! clusters, one active at a time: `prod-a` (Production, locked at open) and `stg-b` (unlocked, a
//! click tier). The fixture starts on `prod-a`, switches to `stg-b`, and makes it live; `activate`
//! does the same for the other one. Each session answers from its own fake API server, so a test
//! sees which cluster a request reached and nothing leaves the machine. A fake never upgrades a
//! connection to a stream, so no
//! test opens a real port-forward. Listeners bind loopback only, on a free port. The fake connections carry
//! their own write policy.

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use cluster::fake_api::{Failure, FakeApi, RecordedRequest};
use cluster::{
    AccessCheck, AccessDecision, AccessReport, AccessReview, ContainerKind, ContainerPort,
    ContainerState, ContainerSummary, NamespaceScope, PodStatus, PodSummary, ReadyCount,
    StatusReason, WritePolicy,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{Cancel, Confirm};
use gpui_kit::{Entity, TestAppContext};

use super::super::app_shell_switch_tests::{SwitchFixture, open_switch_fixture};
use super::*;
use crate::app_shell::Screen;
use crate::cluster_session::{AccessState, ClusterSession};
use crate::confirm_dialog::ConfirmDialog;
use crate::port_forwards::{ForwardFailure, ForwardFixture, ForwardState};
use crate::resource_actions::RowAction;
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};
use crate::write_guard::{DialogConfirm, WriteLock};

const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;
const POD: &str =
    r#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"api-0","namespace":"shop"}}"#;

fn tcp(port: u16) -> ContainerPort {
    ContainerPort {
        name: None,
        port,
        protocol: "TCP".to_owned(),
        host_port: None,
    }
}

fn container(name: &str, ports: Vec<ContainerPort>) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind: ContainerKind::Main,
        state: ContainerState::Running { started_at: None },
        is_ready: true,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports,
        resources: Vec::new(),
        probes: cluster::ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn pod(name: &str, ports: Vec<ContainerPort>) -> PodSummary {
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
        containers: vec![container("app", ports)],
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
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

/// How a fake cluster answers the requests of a forward.
#[derive(Clone, Copy)]
enum Answers {
    /// The pod exists.
    Pod,
    /// Everything is 404: the target is not found.
    NotFound,
    /// Requests are accepted and never answered, so the start never reports.
    Hang,
}

struct Forwards {
    fixture: SwitchFixture,
    /// The fake server of `stg-b`, the cluster the fixture ends on.
    stg_api: FakeApi,
    prod: ClusterRef,
    stg: ClusterRef,
}

fn session_of(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    cx: &mut TestAppContext,
) -> Entity<ClusterSession> {
    fixture
        .shell
        .read_with(cx, |shell, _| shell.session_of(cluster).cloned())
        .expect("a viewed slot")
}

fn go_live(
    fixture: &SwitchFixture,
    cluster: &ClusterRef,
    answers: Answers,
    policy: WritePolicy,
    cx: &mut TestAppContext,
) -> FakeApi {
    // The client's worker is a tokio task, so it must be built inside the runtime.
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        match answers {
            Answers::Pod => FakeApi::connection(policy, |request| {
                if request.path.ends_with("/pods/api-0") {
                    (200, POD.to_owned())
                } else {
                    (404, NOT_FOUND.to_owned())
                }
            }),
            Answers::NotFound => FakeApi::connection(policy, |_| (404, NOT_FOUND.to_owned())),
            Answers::Hang => FakeApi::failing(policy, Failure::Hang),
        }
    };
    let pods = vec![
        pod("api-0", vec![tcp(8080)]),
        pod("multi-0", vec![tcp(8080), tcp(9090)]),
    ];
    let session = session_of(fixture, cluster, cx);
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
        session.set_access_for_test(AccessState::Known(report_denying(&[])), cx);
        session.set_pods_for_test(pods, cx);
    });
    cx.run_until_parked();
    api
}

fn two_clusters(name: &str, stg_answers: Answers, cx: &mut TestAppContext) -> Forwards {
    let fixture = open_switch_fixture(name, cx);
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let stg_api = go_live(&fixture, &stg, stg_answers, WritePolicy::Allowed, cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    Forwards {
        fixture,
        stg_api,
        prod,
        stg,
    }
}

fn pod_spec(pod: &str, port: u16, local: LocalPortSpec) -> ForwardSpec {
    ForwardSpec {
        namespace: "shop".to_owned(),
        target: TargetSpec::pod(pod),
        remote_port: port,
        local_port: local,
    }
}

fn portforward_requests(api: &FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| request.path.contains("/portforward"))
        .collect()
}

/// The requests that belong to a forward: the lookup of its pod and anything to `portforward`. The
/// sessions of the fake clusters send their own list requests, which are not the forward's.
fn forward_requests(api: &FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| {
            request.path.ends_with("/pods/api-0") || request.path.contains("/portforward")
        })
        .collect()
}

fn audit_lines(dir: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(crate::audit_log::audit_path(dir))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON line"))
        .collect()
}

/// 30 s of 20 ms polls: a loaded machine (a full workspace run) can take several seconds to
/// start a forward, and the wait only costs time on a failure.
const WAIT_POLLS: usize = 1_500;

/// A loopback port that is free now.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    listener.local_addr().expect("a local address").port()
}

impl Forwards {
    /// Switches to `cluster` and makes it live over a new fake server that finds the pod. The old
    /// session is gone, so a test never has both clusters live at once.
    fn activate(&self, cluster: &ClusterRef, cx: &mut TestAppContext) -> FakeApi {
        self.fixture
            .shell
            .update(cx, |shell, cx| shell.switch_cluster(cluster, cx));
        cx.run_until_parked();
        let api = go_live(
            &self.fixture,
            cluster,
            Answers::Pod,
            WritePolicy::Allowed,
            cx,
        );
        self.fixture
            .shell
            .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
        cx.run_until_parked();
        self.fixture.draw_twice(cx);
        api
    }

    fn start(&self, cluster: &ClusterRef, spec: ForwardSpec, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            self.fixture.shell.update(cx, |shell, cx| {
                shell.start_forward(cluster, spec, None, window, cx);
            });
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

    fn start_and_confirm(&self, cluster: &ClusterRef, spec: ForwardSpec, cx: &mut TestAppContext) {
        self.start(cluster, spec, cx);
        self.confirm(cx);
    }

    fn rows(&self, cx: &mut TestAppContext) -> Vec<(ForwardId, ForwardState, ClusterRef)> {
        self.fixture.shell.read_with(cx, |shell, cx| {
            shell
                .port_forwards
                .read(cx)
                .forwards()
                .iter()
                .map(|row| (row.id, row.state.clone(), row.cluster.clone()))
                .collect()
        })
    }

    fn only_row(&self, cx: &mut TestAppContext) -> (ForwardId, ForwardState, ClusterRef) {
        let mut rows = self.rows(cx);
        assert_eq!(rows.len(), 1, "one row");
        rows.remove(0)
    }

    fn wait_for(
        &self,
        what: &str,
        cx: &mut TestAppContext,
        done: impl Fn(&Forwards, &mut TestAppContext) -> bool,
    ) {
        for _ in 0..WAIT_POLLS {
            cx.run_until_parked();
            if done(self, cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    fn wait_for_state(&self, state: ForwardState, cx: &mut TestAppContext) {
        self.wait_for("the forward state", cx, |forwards, cx| {
            forwards
                .rows(cx)
                .first()
                .is_some_and(|(_, current, _)| *current == state)
        });
    }

    fn draw(&self, cx: &mut TestAppContext) {
        self.fixture.draw_twice(cx);
    }

    fn set_lock(&self, cluster: &ClusterRef, lock: WriteLock, cx: &mut TestAppContext) {
        let session = session_of(&self.fixture, cluster, cx);
        session.update(cx, |session, cx| session.set_lock(lock, cx));
        cx.run_until_parked();
    }

    fn set_access(&self, cluster: &ClusterRef, access: AccessReport, cx: &mut TestAppContext) {
        let session = session_of(&self.fixture, cluster, cx);
        session.update(cx, |session, cx| {
            session.set_access_for_test(AccessState::Known(access), cx);
        });
        cx.run_until_parked();
    }

    fn press_dialog(&self, answer: impl gpui_kit::Action, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            window.dispatch_action(Box::new(answer), cx);
        });
    }

    /// Settings that save into a fresh folder, so the audit log has one.
    fn audit_folder(&self, name: &str, cx: &mut TestAppContext) -> PathBuf {
        self.install_settings(name, Settings::default(), cx)
    }

    fn install_settings(&self, name: &str, settings: Settings, cx: &mut TestAppContext) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("k8sboard-0035-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the temp dir");
        cx.update(|cx| {
            AppSettings::install(
                LoadedSettings {
                    settings,
                    writes: WriteMode::Enabled(dir.clone()),
                    notice: None,
                },
                cx,
            );
        });
        cx.run_until_parked();
        dir
    }

    fn select(&self, cluster: &ClusterRef, pod: &str, cx: &mut TestAppContext) {
        let object = ClusterObject::new(
            cluster.clone(),
            ResourceKey::Pod {
                namespace: "shop".to_owned(),
                name: pod.to_owned(),
            },
        );
        self.fixture
            .shell
            .update(cx, |shell, cx| shell.change_selection(Some(object), cx));
        cx.run_until_parked();
    }
}

// ---- the guarded start ----

#[gpui_kit::test]
fn a_forward_always_asks_before_it_starts(cx: &mut TestAppContext) {
    let forwards = two_clusters("asks", Answers::Pod, cx);
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    // A click tier still shows a dialog, and nothing started or was requested behind it.
    assert!(forwards.has_dialog(cx));
    assert!(forwards.rows(cx).is_empty());
    assert!(forward_requests(&forwards.stg_api).is_empty());
    let (tier, dry_run) = forwards.dialog(cx).read_with(cx, |dialog, _| {
        (dialog.tier().clone(), dialog.dry_run_state())
    });
    assert_eq!(tier, DialogConfirm::Click);
    assert_eq!(
        dry_run,
        Some(crate::app_shell::write_flow::DryRunState::NotSupported)
    );
}

#[gpui_kit::test]
fn forward_uses_the_rows_cluster(cx: &mut TestAppContext) {
    let forwards = two_clusters("rows-cluster", Answers::Pod, cx);
    let prod_api = forwards.activate(&forwards.prod, cx);
    forwards.set_lock(&forwards.prod, WriteLock::Unlocked, cx);
    // Production, not the click tier of the other cluster: the tier is the target's own.
    forwards.start(
        &forwards.prod,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    let tier = forwards
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.tier().clone());
    assert_eq!(
        tier,
        DialogConfirm::TypeName {
            expected: "api-0".to_owned()
        }
    );
    forwards.confirm(cx);
    forwards.wait_for_state(ForwardState::Active, cx);
    let (_, _, cluster) = forwards.only_row(cx);
    assert_eq!(cluster, forwards.prod);
    // The request reached prod-a, and stg-b saw none.
    assert!(!forward_requests(&prod_api).is_empty());
    assert!(forward_requests(&forwards.stg_api).is_empty());
}

#[gpui_kit::test]
fn confirm_starts_the_forward_on_the_clusters_own_connection(cx: &mut TestAppContext) {
    let forwards = two_clusters("confirm", Answers::Pod, cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    let row = forwards.fixture.shell.read_with(cx, |shell, cx| {
        let row = &shell.port_forwards.read(cx).forwards()[0];
        (
            row.cluster.clone(),
            row.cluster_label.clone(),
            row.pod.clone(),
            row.local,
        )
    });
    assert_eq!(row.0, forwards.stg);
    assert_eq!(row.1.as_ref(), "stg-b");
    assert_eq!(row.2.as_deref(), Some("api-0"));
    // The listener is loopback only.
    assert_eq!(
        row.3.map(|local| *local.ip()),
        Some(std::net::Ipv4Addr::LOCALHOST)
    );
    assert!(
        forwards
            .stg_api
            .requests()
            .iter()
            .any(|request| request.method == "GET" && request.path.ends_with("/pods/api-0"))
    );
    // Resolving a target is a read: nothing asked to forward, and nothing was sent to portforward.
    assert!(portforward_requests(&forwards.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_local_port_typed_in_the_confirm_replaces_the_automatic_one(cx: &mut TestAppContext) {
    let forwards = two_clusters("typed-port", Answers::Pod, cx);
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    let dialog = forwards.dialog(cx);
    forwards.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.type_local_port("18181", window, cx));
    });
    forwards.confirm(cx);
    forwards.wait_for_state(ForwardState::Active, cx);
    let spec = forwards.fixture.shell.read_with(cx, |shell, cx| {
        shell.port_forwards.read(cx).forwards()[0].spec.clone()
    });
    assert_eq!(spec.local_port, LocalPortSpec::Exact(18181));
}

#[gpui_kit::test]
fn ticking_the_audit_note_moves_the_focus_into_its_field(cx: &mut TestAppContext) {
    let forwards = two_clusters("note-focus", Answers::Pod, cx);
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    let dialog = forwards.dialog(cx);
    let is_focused = |forwards: &Forwards, cx: &mut TestAppContext| {
        forwards
            .fixture
            .with_window(cx, |window, cx| dialog.read(cx).is_note_focused(window, cx))
    };
    assert!(!is_focused(&forwards, cx));
    forwards.fixture.with_window(cx, |_, cx| {
        dialog.update(cx, |dialog, cx| dialog.tick_note(cx));
    });
    forwards.fixture.draw_twice(cx);
    assert!(is_focused(&forwards, cx));
}

#[gpui_kit::test]
fn a_local_port_that_is_not_a_port_keeps_the_confirm_open(cx: &mut TestAppContext) {
    let forwards = two_clusters("bad-port", Answers::Pod, cx);
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    let dialog = forwards.dialog(cx);
    forwards.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.type_local_port("70000", window, cx));
    });
    forwards.confirm(cx);
    assert!(forwards.has_dialog(cx));
    assert!(forwards.rows(cx).is_empty());
    let error = dialog.read_with(cx, |dialog, _| dialog.local_port_error());
    assert!(error.is_some());
}

#[gpui_kit::test]
fn the_forward_follows_the_0030_gate(cx: &mut TestAppContext) {
    let forwards = two_clusters("gate", Answers::Pod, cx);
    let prod_api = forwards.activate(&forwards.prod, cx);
    // prod-a is locked at open: no dialog, no row, no request.
    forwards.start(
        &forwards.prod,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    assert!(!forwards.has_dialog(cx));
    forwards.activate(&forwards.stg, cx);
    // A denied verb of the pair says so and starts nothing.
    forwards.set_access(
        &forwards.stg,
        report_denying(&[AccessCheck::CreatePodPortForward]),
        cx,
    );
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    assert!(!forwards.has_dialog(cx));
    assert!(forwards.rows(cx).is_empty());
    assert!(forward_requests(&prod_api).is_empty());
    assert!(forward_requests(&forwards.stg_api).is_empty());
}

#[gpui_kit::test]
fn start_needs_the_active_cluster(cx: &mut TestAppContext) {
    let forwards = two_clusters("not-open", Answers::Pod, cx);
    let gone = ClusterRef {
        kubeconfig: PathBuf::from("elsewhere.yaml"),
        context: "far-away".to_owned(),
    };
    // "Open prod-a to start this forward": prod-a is loaded, but stg-b is the open one. The same
    // for a cluster no kubeconfig has: a notice, no dialog, no row.
    for cluster in [&forwards.prod, &gone] {
        forwards.start(cluster, pod_spec("api-0", 8080, LocalPortSpec::Auto), cx);
        assert!(!forwards.has_dialog(cx));
        assert!(forwards.rows(cx).is_empty());
    }
}

#[gpui_kit::test]
fn cancelling_the_dialog_starts_nothing(cx: &mut TestAppContext) {
    let forwards = two_clusters("cancel", Answers::Pod, cx);
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.press_dialog(Cancel, cx);
    assert!(!forwards.has_dialog(cx));
    assert!(forwards.rows(cx).is_empty());
    assert!(forward_requests(&forwards.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_lock_taken_while_the_dialog_is_open_starts_nothing_and_audits_nothing(
    cx: &mut TestAppContext,
) {
    let forwards = two_clusters("locked-late", Answers::Pod, cx);
    let dir = forwards.audit_folder("locked-late", cx);
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.set_lock(&forwards.stg, WriteLock::Locked, cx);
    forwards.confirm(cx);
    assert!(forwards.rows(cx).is_empty());
    assert!(forward_requests(&forwards.stg_api).is_empty());
    assert!(audit_lines(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_permission_lost_while_the_dialog_is_open_starts_nothing(cx: &mut TestAppContext) {
    let forwards = two_clusters("denied-late", Answers::Pod, cx);
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.set_access(
        &forwards.stg,
        report_denying(&[AccessCheck::GetPodPortForward]),
        cx,
    );
    forwards.confirm(cx);
    assert!(forwards.rows(cx).is_empty());
    assert!(forward_requests(&forwards.stg_api).is_empty());
}

#[gpui_kit::test]
fn twenty_first_forward_is_refused(cx: &mut TestAppContext) {
    let forwards = two_clusters("cap", Answers::Pod, cx);
    forwards.fixture.shell.update(cx, |shell, cx| {
        shell.port_forwards.update(cx, |list, _| {
            for index in 0..MAX_RUNNING_FORWARDS {
                list.insert_fixture(running_fixture(&forwards.stg, &format!("p-{index}")));
            }
        });
    });
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    // "Stop a forward first (20 running)": a notice and no dialog.
    assert!(!forwards.has_dialog(cx));
    assert_eq!(forwards.rows(cx).len(), MAX_RUNNING_FORWARDS);
}

#[gpui_kit::test]
fn quitting_with_a_running_forward_asks_first_and_a_preset_does_not(cx: &mut TestAppContext) {
    let forwards = two_clusters("quit-asks", Answers::Pod, cx);
    let add = |is_preset, cx: &mut TestAppContext| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.port_forwards.update(cx, |list, _| {
                let mut fixture = running_fixture(&forwards.stg, "api-0");
                fixture.is_preset = is_preset;
                list.insert_fixture(fixture);
            });
        });
    };
    let may_close = |cx: &mut TestAppContext| {
        forwards
            .fixture
            .shell
            .update(cx, |shell, cx| shell.main_window_may_close(cx))
    };
    add(true, cx);
    assert!(
        may_close(cx),
        "a saved preset starts again, so it does not ask"
    );
    add(false, cx);
    assert!(!may_close(cx), "the window waits for the answer");
    cx.run_until_parked();
    let asked = forwards
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.last_leaving.clone());
    assert_eq!(asked, Some(vec!["1 port-forward will stop".to_owned()]));
}

fn running_fixture(cluster: &ClusterRef, pod: &str) -> ForwardFixture {
    ForwardFixture {
        cluster: cluster.clone(),
        cluster_label: "stg-b".into(),
        environment: Environment::STAGING,
        spec: pod_spec(pod, 1, LocalPortSpec::Auto),
        state: ForwardState::Active,
        local: None,
        pod: None,
        traffic: cluster::ForwardTraffic::default(),
        events: Vec::new(),
        started_at: None,
        is_preset: false,
    }
}

#[gpui_kit::test]
fn own_port_conflict_is_refused_before_the_dialog(cx: &mut TestAppContext) {
    let forwards = two_clusters("own-port", Answers::Pod, cx);
    let port = free_port();
    forwards.fixture.shell.update(cx, |shell, cx| {
        shell.port_forwards.update(cx, |list, _| {
            let mut held = running_fixture(&forwards.stg, "held");
            held.local = Some(std::net::SocketAddrV4::new(
                std::net::Ipv4Addr::LOCALHOST,
                port,
            ));
            list.insert_fixture(held);
        });
    });
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Exact(port)),
        cx,
    );
    assert!(!forwards.has_dialog(cx));
    assert_eq!(forwards.rows(cx).len(), 1);
}

// ---- audit ----

#[gpui_kit::test]
fn start_appends_one_audit_line(cx: &mut TestAppContext) {
    let forwards = two_clusters("audit", Answers::Pod, cx);
    let dir = forwards.audit_folder("audit", cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    forwards.wait_for("the audit line", cx, |_, _| !audit_lines(&dir).is_empty());
    let lines = audit_lines(&dir);
    assert_eq!(lines.len(), 1, "one line per start");
    let line = &lines[0];
    assert_eq!(line["cluster"], "stg-b");
    assert_eq!(line["action"], "Port-forward");
    assert_eq!(line["object"]["kind"], "Pod");
    assert_eq!(line["object"]["namespace"], "shop");
    assert_eq!(line["object"]["name"], "api-0");
    assert_eq!(line["outcome"], "applied");
    let bound = forwards.fixture.shell.read_with(cx, |shell, cx| {
        shell.port_forwards.read(cx).forwards()[0]
            .local
            .map(|local| local.port())
    });
    let fields: Vec<(String, String)> = line["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .map(|field| {
            (
                field["path"].as_str().expect("a path").to_owned(),
                field["value"].as_str().expect("a value").to_owned(),
            )
        })
        .collect();
    // The line records the port that was bound, which an automatic port may have moved.
    assert_eq!(
        fields,
        [
            ("remote_port".to_owned(), "8080".to_owned()),
            ("local_port".to_owned(), bound.expect("bound").to_string()),
        ]
    );
    // Never traffic: not in the fields, not anywhere on the line.
    let text = line.to_string();
    for word in ["received", "sent", "open_connections", "bytes"] {
        assert!(!text.contains(word), "{word}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_start_that_ends_first_audits_a_failed_line_and_keeps_the_row_for_retry(
    cx: &mut TestAppContext,
) {
    let forwards = two_clusters("failed", Answers::NotFound, cx);
    let dir = forwards.audit_folder("failed", cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for("the audit line", cx, |_, _| !audit_lines(&dir).is_empty());
    let lines = audit_lines(&dir);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["outcome"], "failed");
    assert_eq!(lines[0]["error"], "the forward target was not found");
    let (_, state, _) = forwards.only_row(cx);
    assert!(matches!(
        state,
        ForwardState::Failed(ForwardFailure::Other(_))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_blocked_write_policy_fails_the_forward_before_any_request(cx: &mut TestAppContext) {
    let forwards = two_clusters("blocked", Answers::Pod, cx);
    // The debug kill switch of this connection: writes are blocked.
    let api = go_live(
        &forwards.fixture,
        &forwards.stg,
        Answers::Pod,
        WritePolicy::Blocked,
        cx,
    );
    let dir = forwards.audit_folder("blocked", cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for("the audit line", cx, |_, _| !audit_lines(&dir).is_empty());
    let (_, state, _) = forwards.only_row(cx);
    let ForwardState::Failed(ForwardFailure::Other(text)) = state else {
        panic!("expected a failure");
    };
    assert!(text.contains("writes are blocked"), "{text}");
    assert!(forward_requests(&api).is_empty(), "zero requests");
    assert_eq!(audit_lines(&dir)[0]["outcome"], "failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn stopping_a_start_that_never_reported_audits_it_abandoned(cx: &mut TestAppContext) {
    let forwards = two_clusters("abandoned", Answers::Hang, cx);
    let dir = forwards.audit_folder("abandoned", cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    let (id, state, _) = forwards.only_row(cx);
    assert_eq!(state, ForwardState::Starting);
    assert!(audit_lines(&dir).is_empty(), "nothing reported yet");
    forwards
        .fixture
        .shell
        .update(cx, |shell, cx| shell.stop_forward(id, cx));
    forwards.wait_for("the abandoned line", cx, |_, _| {
        !audit_lines(&dir).is_empty()
    });
    let lines = audit_lines(&dir);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["outcome"], "abandoned");
    assert_eq!(lines[0]["action"], "Port-forward");
    // A non-preset row is gone once stopped.
    assert!(forwards.rows(cx).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_restart_replaces_a_pending_start_and_audits_the_old_one_abandoned(cx: &mut TestAppContext) {
    let forwards = two_clusters("restart", Answers::Hang, cx);
    let dir = forwards.audit_folder("restart", cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    let (id, ..) = forwards.only_row(cx);
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.start_forward_again(id, window, cx);
        });
    });
    forwards.confirm(cx);
    forwards.wait_for("the abandoned line", cx, |_, _| {
        !audit_lines(&dir).is_empty()
    });
    let lines = audit_lines(&dir);
    assert_eq!(lines.len(), 1, "the new start is still pending");
    assert_eq!(lines[0]["outcome"], "abandoned");
    // The row kept its identity, and its restart is Starting again.
    let (again, state, _) = forwards.only_row(cx);
    assert_eq!((again, state), (id, ForwardState::Starting));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- the lock ----

#[gpui_kit::test]
fn locked_cluster_pauses_running_forwards_and_blocks_start(cx: &mut TestAppContext) {
    let forwards = two_clusters("lock", Answers::Pod, cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    forwards.set_lock(&forwards.stg, WriteLock::Locked, cx);
    // New local connections are refused (the stream says so), and the row shows it.
    forwards.wait_for_state(ForwardState::Paused, cx);
    // A new start of a locked cluster is blocked by the gate: no dialog.
    forwards.start(
        &forwards.stg,
        pod_spec("multi-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    assert!(!forwards.has_dialog(cx));
    assert_eq!(forwards.rows(cx).len(), 1);
    // Unlocking resumes them.
    forwards.set_lock(&forwards.stg, WriteLock::Unlocked, cx);
    forwards.wait_for_state(ForwardState::Active, cx);
}

// ---- forwards outlive the view ----

#[gpui_kit::test]
fn forwards_keep_running_after_a_switch(cx: &mut TestAppContext) {
    let forwards = two_clusters("switch-keeps", Answers::Pod, cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    let target = forwards.fixture.cluster("dev-c", cx);
    forwards
        .fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    // Nothing asked: forwards add no "will close" line, and the release went through.
    assert!(!forwards.has_dialog(cx));
    let lines = forwards
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.last_leaving.clone());
    assert_eq!(lines, None);
    let gone = forwards
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.session_of(&forwards.stg).is_none());
    assert!(gone, "stg-b is no longer open");
    // The forward still runs, with the label of the cluster it was started on, and keeps its last
    // control (spec 0046 decision 17): the listener still accepts new local connections.
    let (_, state, cluster) = forwards.only_row(cx);
    assert_eq!(
        (state, cluster),
        (ForwardState::Active, forwards.stg.clone())
    );
    let port = forwards.fixture.shell.read_with(cx, |shell, cx| {
        shell.port_forwards.read(cx).forwards()[0]
            .local
            .map(|local| local.port())
    });
    let address = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port.expect("bound")));
    assert!(
        std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok(),
        "the listener of a cluster that was left still accepts"
    );
    // Its row cannot start again until the cluster is open: the start names the cluster to open.
    let (id, ..) = forwards.only_row(cx);
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.start_forward_again(id, window, cx);
        });
    });
    assert!(!forwards.has_dialog(cx));
}

#[gpui_kit::test]
fn a_switch_with_forwards_asks_nothing(cx: &mut TestAppContext) {
    let forwards = two_clusters("release-quiet", Answers::Pod, cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    forwards
        .fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&forwards.prod, cx));
    cx.run_until_parked();
    // A switch with only forwards running asks nothing: they survive it (decision 20).
    assert!(!forwards.has_dialog(cx));
    let lines = forwards
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.last_leaving.clone());
    assert_eq!(lines, None);
    assert_eq!(forwards.only_row(cx).1, ForwardState::Active);
}

#[gpui_kit::test]
fn status_bar_shows_running_count_and_opens_the_page(cx: &mut TestAppContext) {
    let forwards = two_clusters("status-bar", Answers::Pod, cx);
    forwards.draw(cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    forwards.draw(cx);
    let count = forwards
        .fixture
        .shell
        .read_with(cx, |shell, cx| shell.running_forward_count(cx));
    assert_eq!(count, 1);
    // The click shows the page.
    forwards.fixture.shell.update(cx, |shell, cx| {
        shell.show_screen(Screen::PortForwarding, cx)
    });
    cx.run_until_parked();
    let screen = forwards
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.screen);
    assert_eq!(screen, Screen::PortForwarding);
}

#[gpui_kit::test]
fn page_lists_forwards_of_every_cluster(cx: &mut TestAppContext) {
    let forwards = two_clusters("page", Answers::Pod, cx);
    forwards.activate(&forwards.prod, cx);
    forwards.set_lock(&forwards.prod, WriteLock::Unlocked, cx);
    forwards.start_and_confirm(
        &forwards.prod,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.activate(&forwards.stg, cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for("both forwards", cx, |forwards, cx| {
        let rows = forwards.rows(cx);
        rows.len() == 2
            && rows
                .iter()
                .all(|(_, state, _)| *state == ForwardState::Active)
    });
    forwards.fixture.shell.update(cx, |shell, cx| {
        shell.show_screen(Screen::PortForwarding, cx)
    });
    cx.run_until_parked();
    forwards.fixture.draw_twice(cx);
    assert!(forwards.fixture.is_drawn("forward-new", cx));
    assert!(forwards.fixture.is_drawn("forward-stop-all", cx));
    let (count, clusters) = forwards.fixture.shell.read_with(cx, |shell, cx| {
        (
            shell.port_forward_header_count(cx),
            shell
                .port_forwards
                .read(cx)
                .sorted("")
                .iter()
                .map(|row| row.cluster.context.clone())
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(count, "2 forwards · 2 active");
    assert_eq!(clusters, ["prod-a", "stg-b"]);
}

// ---- presets ----

#[gpui_kit::test]
fn presets_load_as_stopped_rows_when_the_settings_have_them(cx: &mut TestAppContext) {
    let forwards = two_clusters("presets", Answers::Pod, cx);
    let mut settings = Settings::default();
    settings.port_forward.presets.push(ForwardPreset {
        cluster: forwards.stg.clone(),
        spec: pod_spec("api-0", 8080, LocalPortSpec::Exact(18080)),
    });
    let dir = forwards.install_settings("presets", settings, cx);
    let (_, state, cluster) = forwards.only_row(cx);
    assert_eq!(
        (state, cluster),
        (ForwardState::Stopped, forwards.stg.clone())
    );
    // Nothing started by itself.
    assert!(forward_requests(&forwards.stg_api).is_empty());
    assert!(!forwards.has_dialog(cx));
    // Start asks like any other start, on the preset's own cluster.
    let (id, ..) = forwards.only_row(cx);
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.start_forward_again(id, window, cx);
        });
    });
    assert!(forwards.has_dialog(cx));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn save_and_remove_preset_round_trip_through_the_settings(cx: &mut TestAppContext) {
    let forwards = two_clusters("preset-round", Answers::Pod, cx);
    let dir = forwards.audit_folder("preset-round", cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    let (id, ..) = forwards.only_row(cx);
    let presets =
        |cx: &mut TestAppContext| cx.update(|cx| AppSettings::get(cx).port_forward.presets.clone());
    forwards
        .fixture
        .shell
        .update(cx, |shell, cx| shell.save_forward_preset(id, cx));
    forwards
        .fixture
        .shell
        .update(cx, |shell, cx| shell.save_forward_preset(id, cx));
    cx.run_until_parked();
    let saved = presets(cx);
    assert_eq!(saved.len(), 1, "saving twice keeps one preset");
    assert_eq!(saved[0].cluster, forwards.stg);
    assert!(matches!(saved[0].spec.local_port, LocalPortSpec::Exact(_)));
    // Removing the preset of a running forward keeps it running as a plain forward.
    forwards
        .fixture
        .shell
        .update(cx, |shell, cx| shell.remove_forward_preset(id, cx));
    cx.run_until_parked();
    assert!(presets(cx).is_empty());
    let (_, state, _) = forwards.only_row(cx);
    assert_eq!(state, ForwardState::Active);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn change_local_port_restarts_a_running_forward_through_the_guarded_start(cx: &mut TestAppContext) {
    let forwards = two_clusters("change-port", Answers::Pod, cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Auto),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    let (id, ..) = forwards.only_row(cx);
    let port = free_port();
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.change_local_port(id, port, window, cx);
        });
    });
    // It asks first, and the old forward keeps running until the answer.
    assert!(forwards.has_dialog(cx));
    assert_eq!(forwards.only_row(cx).1, ForwardState::Active);
    forwards.confirm(cx);
    forwards.wait_for("the new port", cx, |forwards, cx| {
        forwards.fixture.shell.read_with(cx, |shell, cx| {
            let row = &shell.port_forwards.read(cx).forwards()[0];
            row.state == ForwardState::Active
                && row.spec.local_port == LocalPortSpec::Exact(port)
                && row.local.map(|local| local.port()) == Some(port)
        })
    });
}

#[gpui_kit::test]
fn change_local_port_of_a_stopped_preset_only_stores_it(cx: &mut TestAppContext) {
    let forwards = two_clusters("change-stopped", Answers::Pod, cx);
    let mut settings = Settings::default();
    settings.port_forward.presets.push(ForwardPreset {
        cluster: forwards.stg.clone(),
        spec: pod_spec("api-0", 8080, LocalPortSpec::Exact(18080)),
    });
    let dir = forwards.install_settings("change-stopped", settings, cx);
    let (id, ..) = forwards.only_row(cx);
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.change_local_port(id, 18081, window, cx);
        });
    });
    assert!(!forwards.has_dialog(cx), "a stopped row starts nothing");
    let stored = cx.update(|cx| AppSettings::get(cx).port_forward.presets[0].spec.local_port);
    assert_eq!(stored, LocalPortSpec::Exact(18081));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- entry points ----

#[gpui_kit::test]
fn f_key_with_one_port_asks_to_forward_it_in_the_cursor_cluster(cx: &mut TestAppContext) {
    let forwards = two_clusters("f-one", Answers::Pod, cx);
    forwards.select(&forwards.stg, "api-0", cx);
    forwards.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(crate::keymap::PortForward), cx);
    });
    assert!(forwards.has_dialog(cx));
    let tier = forwards
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.tier().clone());
    assert_eq!(tier, DialogConfirm::Click);
    forwards.confirm(cx);
    forwards.wait_for_state(ForwardState::Active, cx);
    let (spec, cluster) = forwards.fixture.shell.read_with(cx, |shell, cx| {
        let row = &shell.port_forwards.read(cx).forwards()[0];
        (row.spec.clone(), row.cluster.clone())
    });
    assert_eq!(cluster, forwards.stg);
    assert_eq!(
        (spec.remote_port, spec.target),
        (8080, TargetSpec::pod("api-0"))
    );
}

#[gpui_kit::test]
fn f_key_with_several_ports_opens_new_forward(cx: &mut TestAppContext) {
    let forwards = two_clusters("f-several", Answers::Pod, cx);
    forwards.select(&forwards.stg, "multi-0", cx);
    forwards.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(crate::keymap::PortForward), cx);
    });
    // A dialog is open, but it is the form, not a confirm: no start was asked for yet.
    assert!(forwards.has_dialog(cx));
    let confirm = forwards
        .fixture
        .shell
        .read_with(cx, |shell, _| shell.last_dialog.clone());
    assert!(confirm.is_none());
    assert!(forwards.rows(cx).is_empty());
    assert!(forward_requests(&forwards.stg_api).is_empty());
}

#[gpui_kit::test]
fn f_key_menu_and_palette_share_the_arm(cx: &mut TestAppContext) {
    let forwards = two_clusters("f-arm", Answers::Pod, cx);
    forwards.select(&forwards.stg, "api-0", cx);
    // The menu item without a port and the palette entry dispatch the key action of the row.
    forwards.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(RowAction::PortForward.key_action(), cx);
    });
    assert!(forwards.has_dialog(cx));
    forwards.press_dialog(Cancel, cx);
    // The same arm, run for the cursor row directly, gives the same answer.
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.run_row_key(RowAction::PortForward, window, cx);
        });
    });
    assert!(forwards.has_dialog(cx));
    forwards.press_dialog(Cancel, cx);
    // A denied pair is one reason on every surface: nothing opens.
    forwards.set_access(
        &forwards.stg,
        report_denying(&[AccessCheck::GetPodPortForward]),
        cx,
    );
    forwards.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(RowAction::PortForward.key_action(), cx);
    });
    assert!(!forwards.has_dialog(cx));
}

#[gpui_kit::test]
fn the_new_forward_form_shows_the_active_cluster_only(cx: &mut TestAppContext) {
    let forwards = two_clusters("form-cluster", Answers::Pod, cx);
    let open_form = |cluster: &ClusterRef, cx: &mut TestAppContext| {
        forwards.fixture.with_window(cx, |window, cx| {
            forwards.fixture.shell.update(cx, |shell, cx| {
                let prefill = crate::app_shell::port_forward_dialogs::NewForwardPrefill {
                    cluster: cluster.clone(),
                    namespace: "shop".to_owned(),
                    target: None,
                    remote_port: None,
                };
                shell.open_new_forward(prefill, window, cx);
            });
        });
    };
    // A cluster that is not the open one cannot start a forward: nothing opens.
    open_form(&forwards.prod, cx);
    assert!(!forwards.has_dialog(cx));
    open_form(&forwards.stg, cx);
    assert!(forwards.has_dialog(cx));
    forwards.press_dialog(Cancel, cx);
}

#[gpui_kit::test]
fn the_dialogs_open_in_the_open_cluster(cx: &mut TestAppContext) {
    let forwards = two_clusters("dialogs", Answers::Pod, cx);
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            let prefill = crate::app_shell::port_forward_dialogs::NewForwardPrefill {
                cluster: forwards.stg.clone(),
                namespace: "shop".to_owned(),
                target: None,
                remote_port: None,
            };
            shell.open_new_forward(prefill, window, cx);
        });
    });
    assert!(forwards.has_dialog(cx));
    forwards.press_dialog(Cancel, cx);
    assert!(!forwards.has_dialog(cx));
    // Remove preset… is a click-only question about a preset row.
    let mut settings = Settings::default();
    settings.port_forward.presets.push(ForwardPreset {
        cluster: forwards.stg.clone(),
        spec: pod_spec("api-0", 8080, LocalPortSpec::Exact(18080)),
    });
    let dir = forwards.install_settings("dialogs", settings, cx);
    let (id, ..) = forwards.only_row(cx);
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.open_remove_preset(id, window, cx);
        });
    });
    assert!(forwards.has_dialog(cx));
    // Only a click on Remove removes: the confirm action (Enter) does nothing.
    forwards.press_dialog(Confirm { secondary: false }, cx);
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| AppSettings::get(cx).port_forward.presets.len()),
        1
    );
    forwards.press_dialog(Cancel, cx);
    assert!(!forwards.has_dialog(cx));
    forwards
        .fixture
        .shell
        .update(cx, |shell, cx| shell.remove_forward_preset(id, cx));
    cx.run_until_parked();
    let left = cx.update(|cx| AppSettings::get(cx).port_forward.presets.len());
    assert_eq!(left, 0);
    assert!(forwards.rows(cx).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn stopping_a_forward_closes_its_listener(cx: &mut TestAppContext) {
    let forwards = two_clusters("closes", Answers::Pod, cx);
    forwards.start_and_confirm(
        &forwards.stg,
        pod_spec("api-0", 8080, LocalPortSpec::Exact(free_port())),
        cx,
    );
    forwards.wait_for_state(ForwardState::Active, cx);
    let (id, ..) = forwards.only_row(cx);
    let port = forwards.fixture.shell.read_with(cx, |shell, cx| {
        shell.port_forwards.read(cx).forwards()[0]
            .local
            .map(|local| local.port())
    });
    let address = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port.expect("bound")));
    let accepts =
        || std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok();
    assert!(accepts(), "the listener accepts while the forward runs");
    forwards
        .fixture
        .shell
        .update(cx, |shell, cx| shell.stop_forward(id, cx));
    // A fixed free port, not `Auto`: tests run in parallel, and another test's `Auto` forward could
    // take the port this one frees, so "the listener closed" would never hold.
    // Dropping the stream closes the listener; quitting the app drops it the same way.
    forwards.wait_for("the listener to close", cx, |_, _| !accepts());
}

#[gpui_kit::test]
fn change_local_port_of_a_running_preset_is_saved_only_after_the_confirmed_start(
    cx: &mut TestAppContext,
) {
    let forwards = two_clusters("preset-late", Answers::Pod, cx);
    let mut settings = Settings::default();
    settings.port_forward.presets.push(ForwardPreset {
        cluster: forwards.stg.clone(),
        spec: pod_spec("api-0", 8080, LocalPortSpec::Exact(free_port())),
    });
    let dir = forwards.install_settings("preset-late", settings, cx);
    let before = cx.update(|cx| AppSettings::get(cx).port_forward.presets[0].spec.local_port);
    let (id, ..) = forwards.only_row(cx);
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.start_forward_again(id, window, cx);
        });
    });
    forwards.confirm(cx);
    forwards.wait_for_state(ForwardState::Active, cx);
    let port = free_port();
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.change_local_port(id, port, window, cx);
        });
    });
    // The dialog is open, and a click on Back leaves the settings as they were.
    assert!(forwards.has_dialog(cx));
    let now = cx.update(|cx| AppSettings::get(cx).port_forward.presets[0].spec.local_port);
    assert_eq!(now, before, "nothing is saved before the confirm");
    forwards.press_dialog(Cancel, cx);
    let after_cancel = cx.update(|cx| AppSettings::get(cx).port_forward.presets[0].spec.local_port);
    assert_eq!(after_cancel, before);
    // A confirmed restart saves the new port.
    forwards.fixture.with_window(cx, |window, cx| {
        forwards.fixture.shell.update(cx, |shell, cx| {
            shell.change_local_port(id, port, window, cx);
        });
    });
    forwards.confirm(cx);
    let saved = cx.update(|cx| AppSettings::get(cx).port_forward.presets[0].spec.local_port);
    assert_eq!(saved, LocalPortSpec::Exact(port));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_started_notice_names_address_target_and_port() {
    let spec = ForwardSpec {
        namespace: "shop".to_owned(),
        target: TargetSpec {
            kind: crate::port_forwards::TargetKind::Service,
            name: "api".to_owned(),
        },
        remote_port: 80,
        local_port: LocalPortSpec::Auto,
    };
    assert_eq!(forward_started_text(19090), "Forwarding localhost:19090");
    assert_eq!(forward_started_lines(&spec, 10080), ["→ svc/api:80"]);
}

#[test]
fn the_started_notice_names_the_port_the_forward_moved_off() {
    let spec = ForwardSpec {
        namespace: "shop".to_owned(),
        target: TargetSpec {
            kind: crate::port_forwards::TargetKind::Service,
            name: "api".to_owned(),
        },
        remote_port: 8080,
        local_port: LocalPortSpec::Auto,
    };
    assert_eq!(
        forward_started_lines(&spec, 18081),
        ["→ svc/api:8080", "18080 was in use"]
    );
}

#[test]
fn a_long_pod_name_is_cut_in_the_middle_for_the_notice() {
    let spec = pod_spec(
        "web-748d84d44b-kxc6x-with-a-long-generated-name",
        8080,
        LocalPortSpec::Auto,
    );
    let line = forward_target_text(&spec);
    assert!(line.starts_with("pod/web-"), "{line}");
    assert!(line.contains('…') && line.ends_with("-name:8080"), "{line}");
    assert!(line.chars().count() <= "pod/".len() + NOTICE_TARGET_CHARS + ":8080".len());
}

#[test]
fn a_busy_automatic_port_names_who_holds_it_and_the_port_used_instead() {
    let other = pod_spec("web-1", 80, LocalPortSpec::Exact(18080));
    let held = busy_port(18080, Some(18081), Some(&other)).expect("a busy port");
    assert_eq!(
        held.text,
        "Port 18080 is in use locally (by forward web-1:80): using 18081"
    );
    assert_eq!(held.free, Some(18081));
    let foreign = busy_port(18080, Some(18081), None).expect("a busy port");
    assert_eq!(
        foreign.text,
        "Port 18080 is in use locally (by another program): using 18081"
    );
    let crowded = busy_port(18080, None, None).expect("a busy port");
    assert!(
        crowded
            .text
            .ends_with("the next ports are taken too, the system picks one")
    );
    assert!(busy_port(18080, Some(18080), None).is_none());
}

#[gpui_kit::test]
fn the_confirm_says_so_when_the_automatic_port_is_taken_on_this_machine(cx: &mut TestAppContext) {
    let held = TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    let port = held.local_addr().expect("a local address").port();
    let forwards = two_clusters("busy-port", Answers::Pod, cx);
    // The automatic port of remote `p - 10000` is `p`.
    forwards.start(
        &forwards.stg,
        pod_spec("api-0", port - 10_000, LocalPortSpec::Auto),
        cx,
    );
    let dialog = forwards.dialog(cx);
    let warnings = dialog.read_with(cx, |dialog, _| dialog.warning_lines());
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].starts_with(
            format!("Port {port} is in use locally (by another program): using ").as_str()
        ),
        "{}",
        warnings[0]
    );
    drop(held);
}
