use cluster::{ContainerState, PodSummary};
#[cfg(feature = "screenshot")]
use gpui_kit::{AnyWindowHandle, App, Entity};

#[cfg(any(feature = "screenshot", test))]
use crate::cluster_metrics::{FeedStatus, is_metrics_settled};
use crate::kind_row::{DAEMON_SET_KIND, JOB_KIND, PodOwner, REPLICA_SET_KIND, STATEFUL_SET_KIND};
use crate::pod_drawer::default_container;

#[cfg(any(feature = "screenshot", test))]
use crate::launch_options::LaunchScreen;
#[cfg(feature = "screenshot")]
use {
    crate::app_shell::AppShell,
    gpui_kit::component::WindowExt as _,
    gpui_kit::test::TestWindowExt as _,
    std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration},
};

#[cfg(feature = "screenshot")]
const POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(feature = "screenshot")]
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Kubelet rounds a Monitor screen waits for. A rate needs two samples of one series, and the disk
/// node is first read in an early round that may fall less than a second after the previous
/// scrape (cAdvisor housekeeping), where a rate is not taken; so the first disk rate usually comes
/// with the fourth round. At 15 s a round that takes about 50 s, hence `SETTLE_TIMEOUT`.
#[cfg(any(feature = "screenshot", test))]
const MIN_KUBELET_TICKS: u64 = 4;
/// Animations and the first frame after the screen setup need a moment before capture.
#[cfg(feature = "screenshot")]
const SETTLE_DELAY: Duration = Duration::from_millis(300);

/// The pod the drawer screens open: the first one with at least two containers, so the
/// Containers tab has something to choose from, else the first row.
pub(crate) fn pick_drawer_pod(pods: &[PodSummary]) -> Option<usize> {
    pods.iter()
        .position(|pod| pod.containers.len() >= 2)
        .or_else(|| (!pods.is_empty()).then_some(0))
}

/// The first item that `select` names: `name`, or `namespace/name` for a namespaced one. Items are
/// `(namespace, name)` pairs in list order.
pub(crate) fn pick_selected<'a>(
    select: &str,
    mut items: impl Iterator<Item = (Option<&'a str>, &'a str)>,
) -> Option<usize> {
    items.position(|(namespace, name)| {
        select == name
            || namespace.is_some_and(|namespace| {
                select
                    .strip_prefix(namespace)
                    .and_then(|rest| rest.strip_prefix('/'))
                    == Some(name)
            })
    })
}

/// The pod the logs screens open: the first whose default container is running (a running
/// container usually has log history), else the drawer pod.
pub(crate) fn pick_logs_pod(pods: &[PodSummary]) -> Option<usize> {
    let has_running_default = |pod: &PodSummary| {
        default_container(&pod.containers)
            .and_then(|index| pod.containers.get(index))
            .is_some_and(|container| matches!(container.state, ContainerState::Running { .. }))
    };
    pods.iter()
        .position(has_running_default)
        .or_else(|| pick_drawer_pod(pods))
}

/// The workload that owns `pod`, as the `logs-workload` screen opens it: its controller when that
/// is a ReplicaSet, StatefulSet, DaemonSet, or Job. A bare pod, or one owned by anything else,
/// has none.
pub(crate) fn controller_owner_of(pod: &PodSummary) -> Option<PodOwner> {
    let controller = pod.controller.as_ref()?;
    let kind = [
        REPLICA_SET_KIND,
        STATEFUL_SET_KIND,
        DAEMON_SET_KIND,
        JOB_KIND,
    ]
    .into_iter()
    .find(|kind| *kind == controller.kind)?;
    Some(PodOwner::Controller {
        namespace: pod.namespace.clone(),
        kind,
        name: controller.name.clone(),
    })
}

/// How far the data behind the screen is.
#[cfg(any(feature = "screenshot", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetState {
    Loading,
    /// The kubeconfig, the context, or the session failed. The error screen is the target.
    Unavailable,
    /// The list for the screen is no longer loading (ready, empty, or failed).
    Loaded,
}

#[cfg(any(feature = "screenshot", test))]
pub(crate) struct SettleInput {
    pub(crate) target: TargetState,
    /// The kubeconfig catalog is still loading, which the Settings screens wait for.
    pub(crate) is_catalog_loading: bool,
    /// A drawer screen has its row selected, or found no row to select.
    pub(crate) is_drawer_ready: bool,
    /// A logs screen whose tab is not open yet or still connecting.
    pub(crate) is_log_pending: bool,
    /// A tool dialog whose answer has not arrived.
    pub(crate) is_dialog_pending: bool,
    /// `--screen switcher`: the popover is not open yet, or a probe has not answered.
    pub(crate) is_switcher_pending: bool,
    /// Overview: the change feeds have not delivered their first snapshot.
    pub(crate) is_change_feed_pending: bool,
    /// Topology: a feed that runs has not delivered its first snapshot, or the graph is not built.
    pub(crate) is_topology_pending: bool,
    /// Where the pods metrics feed stands.
    pub(crate) pod_metrics: FeedProgress,
    /// The same for the nodes feed.
    pub(crate) node_metrics: FeedProgress,
    /// The kubelet feed, which the Monitor screens wait for too.
    pub(crate) kubelet: FeedProgress,
}

/// A metrics feed's status and how many ticks it has recorded.
#[cfg(any(feature = "screenshot", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FeedProgress {
    pub(crate) status: FeedStatus,
    pub(crate) ticks: u64,
}

#[cfg(any(feature = "screenshot", test))]
impl FeedProgress {
    /// No session: nothing to wait for.
    pub(crate) fn unavailable() -> Self {
        Self {
            status: FeedStatus::Unavailable(String::new()),
            ticks: 0,
        }
    }

    /// Whether the feed has what a screen needing `min_ticks` ticks asks for.
    fn is_settled(&self, min_ticks: u64) -> bool {
        is_metrics_settled(&self.status, self.ticks, min_ticks)
    }
}

/// The cluster the shell fixture screens name; they need no real one.
#[cfg(feature = "screenshot")]
pub(crate) const SHELL_FIXTURE_CLUSTER: &str = "prod-eu-1";

/// What `--screen shell-find-fixture` searches for: it matches three lines of the transcript.
#[cfg(feature = "screenshot")]
pub(crate) const SHELL_FIXTURE_FIND: &str = "ledger";

/// The pod of the shell fixture screens (the W8b pane).
#[cfg(feature = "screenshot")]
pub(crate) fn shell_fixture_target() -> crate::shell_tab::ShellTarget {
    crate::shell_tab::ShellTarget {
        cluster: crate::cluster_registry::ClusterRef {
            kubeconfig: std::path::PathBuf::from("fixture.yaml"),
            context: SHELL_FIXTURE_CLUSTER.to_owned(),
        },
        namespace: "payments".to_owned(),
        pod: "api-7d9f8c-m8n2p".to_owned(),
        short_pod: "m8n2p".to_owned(),
        container: "api".to_owned(),
    }
}

/// The two clusters of the Port Forwarding fixture screens, with their switcher labels: a
/// Production one and a Staging one. They need no real cluster.
#[cfg(feature = "screenshot")]
pub(crate) fn forward_fixture_clusters() -> (
    (crate::cluster_registry::ClusterRef, &'static str),
    (crate::cluster_registry::ClusterRef, &'static str),
) {
    let cluster = |context: &str| crate::cluster_registry::ClusterRef {
        kubeconfig: std::path::PathBuf::from("fixture.yaml"),
        context: context.to_owned(),
    };
    (
        (cluster("prod-eu-1"), "prod-eu-1"),
        (cluster("stg-eu-1"), "stg-eu-1"),
    )
}

/// The five rows of the W7 Port Forwarding page (`--screen port-forwards`), oldest first. Times
/// are relative to `now`, so the uptimes read `2h 14m` and `38m` whenever it is taken.
#[cfg(feature = "screenshot")]
pub(crate) fn forward_fixtures(now: jiff::Timestamp) -> Vec<crate::port_forwards::ForwardFixture> {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use cluster::ForwardTraffic;

    use crate::environment::Environment;
    use crate::port_forwards::{
        ForwardFailure, ForwardFixture, ForwardLogLine, ForwardSpec, ForwardState, LocalPortSpec,
        TargetKind, TargetSpec,
    };
    use crate::status_tone::StatusTone;

    let (production, staging) = forward_fixture_clusters();
    let ago =
        |minutes: i64| jiff::Timestamp::from_second(now.as_second() - minutes * 60).unwrap_or(now);
    let local = |port: u16| Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    let spec =
        |namespace: &str, kind: TargetKind, name: &str, remote: u16, port: u16| ForwardSpec {
            namespace: namespace.to_owned(),
            target: TargetSpec {
                kind,
                name: name.to_owned(),
            },
            remote_port: remote,
            local_port: LocalPortSpec::Exact(port),
        };
    let line = |minutes: i64, text: &str, tone: Option<StatusTone>| ForwardLogLine {
        at: ago(minutes),
        text: text.to_owned().into(),
        tone,
    };
    let row = |origin: &(crate::cluster_registry::ClusterRef, &str),
               environment: Environment,
               spec: ForwardSpec,
               state: ForwardState| ForwardFixture {
        cluster: origin.0.clone(),
        cluster_label: origin.1.to_owned().into(),
        environment,
        spec,
        state,
        local: None,
        pod: None,
        traffic: ForwardTraffic::default(),
        events: Vec::new(),
        started_at: None,
        is_preset: false,
    };
    vec![
        ForwardFixture {
            local: local(15432),
            pod: Some("postgres-0".to_owned()),
            traffic: ForwardTraffic {
                open_connections: 3,
                received: 182_000_000,
                sent: 9_400_000,
            },
            events: vec![
                line(134, "started for pod/postgres-0", None),
                line(2, "connection lost: pod deleted", Some(StatusTone::Warn)),
                line(1, "reconnected to postgres-0", Some(StatusTone::Ok)),
            ],
            started_at: Some(ago(134)),
            ..row(
                &production,
                Environment::Production,
                spec("payments", TargetKind::Pod, "postgres-0", 5432, 15432),
                ForwardState::Active,
            )
        },
        ForwardFixture {
            local: local(18080),
            pod: Some("payments-api-6d5c7b9f4-q8x2w".to_owned()),
            started_at: Some(ago(38)),
            ..row(
                &production,
                Environment::Production,
                spec("payments", TargetKind::Service, "payments-api", 80, 18080),
                ForwardState::Active,
            )
        },
        ForwardFixture {
            local: local(19090),
            ..row(
                &production,
                Environment::Production,
                spec("payments", TargetKind::Pod, "api-7d9f8c-x2k4q", 9090, 19090),
                ForwardState::Reconnecting { attempt: 2 },
            )
        },
        row(
            &staging,
            Environment::Staging,
            spec("monitoring", TargetKind::Service, "grafana", 3000, 3000),
            ForwardState::Failed(ForwardFailure::PortInUse(3000)),
        ),
        ForwardFixture {
            is_preset: true,
            ..row(
                &staging,
                Environment::Staging,
                spec("data", TargetKind::Service, "kafka-bootstrap", 9092, 9092),
                ForwardState::Stopped,
            )
        },
    ]
}

/// The object `--screen edit-yaml-diff` edits (W10): the masked YAML of a Deployment, without the
/// edit header. The screen changes `replicas` and the memory limit.
#[cfg(feature = "screenshot")]
pub(crate) const EDIT_FIXTURE_BEFORE: &str = "\
apiVersion: apps/v1
kind: Deployment
metadata:
  annotations:
    deployment.kubernetes.io/revision: \"38\"
  labels:
    app: api
    app.kubernetes.io/part-of: payments
    team: payments
  name: api
  namespace: payments
spec:
  replicas: 3
  selector:
    matchLabels:
      app: api
  template:
    metadata:
      labels:
        app: api
    spec:
      containers:
      - image: registry.example.com/payments/api:2.14.0
        name: api
        resources:
          limits:
            memory: 512Mi
          requests:
            cpu: 250m
            memory: 256Mi
";

/// What `--screen shell-fixture` shows in the shell tab: the transcript of the W8b pane. The
/// private OSC 7770 names the shell, as the `Auto` script does.
#[cfg(feature = "screenshot")]
pub(crate) const SHELL_FIXTURE_TRANSCRIPT: &str = concat!(
    "\x1b]7770;sh\x07",
    "\x1b[32m/app $\x1b[0m env | grep LEDGER\r\n",
    "LEDGER_URL=http://ledger-svc.payments:8080\r\n",
    "LEDGER_TIMEOUT_MS=5000\r\n",
    "\x1b[32m/app $\x1b[0m wget -qO- ledger-svc:8080/healthz\r\n",
    "{\"status\":\"degraded\",\"db\":\"slow\"}\r\n",
    "\x1b[32m/app $\x1b[0m cat /proc/meminfo | head -3\r\n",
    "MemTotal:       65011720 kB\r\n",
    "MemFree:         9127444 kB\r\n",
    "MemAvailable:   21504112 kB\r\n",
    "\x1b[32m/app $\x1b[0m ",
);

/// The data of several viewed clusters as one state: any cluster still loading holds the screen,
/// and the target is unavailable only when every cluster is (a failed cluster is a banner next to
/// the rows of the others, which is the target to capture).
#[cfg(any(feature = "screenshot", test))]
pub(crate) fn combine_targets(states: impl Iterator<Item = TargetState>) -> TargetState {
    let mut has_loaded = false;
    for state in states {
        match state {
            TargetState::Loading => return TargetState::Loading,
            TargetState::Loaded => has_loaded = true,
            TargetState::Unavailable => {}
        }
    }
    if has_loaded {
        TargetState::Loaded
    } else {
        TargetState::Unavailable
    }
}

/// The feed that settles last among several clusters': one that is down settles at once, so the
/// others come first, and among those the one with the fewest ticks. An unavailable feed without
/// any.
#[cfg(any(feature = "screenshot", test))]
pub(crate) fn slowest_feed(feeds: impl Iterator<Item = FeedProgress>) -> FeedProgress {
    let is_down = |status: &FeedStatus| {
        matches!(
            status,
            FeedStatus::Unavailable(_) | FeedStatus::Failed(_) | FeedStatus::Interrupted(_)
        )
    };
    feeds
        .min_by_key(|feed| (is_down(&feed.status), feed.ticks))
        .unwrap_or_else(FeedProgress::unavailable)
}

/// The kubelet feed's progress. With no node to poll (a large cluster without demand) no round
/// ever comes, so the feed counts as settled.
#[cfg(any(feature = "screenshot", test))]
pub(crate) fn kubelet_progress(status: FeedStatus, ticks: u64, has_targets: bool) -> FeedProgress {
    let status = if has_targets {
        status
    } else {
        FeedStatus::Unavailable("no nodes to poll".to_owned())
    };
    FeedProgress { status, ticks }
}

/// A drawer screen is ready when its row is selected (or no row was found to select) and its
/// events and YAML are no longer pending.
#[cfg(any(feature = "screenshot", test))]
pub(crate) fn is_drawer_ready(
    has_selection: bool,
    is_launch_pending: bool,
    is_content_pending: bool,
) -> bool {
    (has_selection || !is_launch_pending) && !is_content_pending
}

/// Whether the screen shows what `--screen` asked for, so a screenshot is worth taking.
#[cfg(any(feature = "screenshot", test))]
pub(crate) fn is_screen_settled(screen: LaunchScreen, input: &SettleInput) -> bool {
    // A Settings screen shows the catalog, not the main window's session.
    if screen.settings_screen().is_some() {
        return !input.is_catalog_loading;
    }
    // A dialog drawn from fixed data waits for no cluster, only for its own opening.
    if screen.is_dialog_fixture() {
        return !input.is_dialog_pending;
    }
    match input.target {
        TargetState::Unavailable => true,
        TargetState::Loading => false,
        TargetState::Loaded if screen.has_dock() => !input.is_log_pending,
        TargetState::Loaded if screen.opens_dialog() => !input.is_dialog_pending,
        TargetState::Loaded if screen == LaunchScreen::Switcher => !input.is_switcher_pending,
        TargetState::Loaded if screen == LaunchScreen::Overview && input.is_change_feed_pending => {
            false
        }
        TargetState::Loaded if screen.shows_topology() && input.is_topology_pending => false,
        TargetState::Loaded
            if screen.shows_pod_usage()
                && !input.pod_metrics.is_settled(screen.min_metrics_ticks()) =>
        {
            false
        }
        TargetState::Loaded
            if screen.shows_node_usage()
                && !input.node_metrics.is_settled(screen.min_metrics_ticks()) =>
        {
            false
        }
        TargetState::Loaded
            if screen.shows_kubelet_stats() && !input.kubelet.is_settled(MIN_KUBELET_TICKS) =>
        {
            false
        }
        TargetState::Loaded => !screen.selects_row() || input.is_drawer_ready,
    }
}

#[cfg(any(feature = "screenshot", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScreenshotOutcome {
    /// Saved after the screen settled.
    Saved,
    /// Saved, but the screen had not settled when the timeout ran out.
    TimedOut,
    Failed,
}

#[cfg(any(feature = "screenshot", test))]
impl ScreenshotOutcome {
    pub(crate) fn exit_code(self) -> u8 {
        match self {
            Self::Saved => 0,
            Self::Failed => 1,
            Self::TimedOut => 3,
        }
    }
}

#[cfg(feature = "screenshot")]
pub(crate) struct ScreenshotRequest {
    pub(crate) path: PathBuf,
    pub(crate) screen: LaunchScreen,
}

/// Waits for the screen to settle, then renders the window to a PNG and quits the app.
/// The outcome is left in `outcome` for `main`.
#[cfg(feature = "screenshot")]
pub(crate) fn capture(
    window: AnyWindowHandle,
    shell: Entity<AppShell>,
    request: ScreenshotRequest,
    outcome: Rc<Cell<ScreenshotOutcome>>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        match capture_when_settled(&window, &shell, &request, cx).await {
            Ok(result) => outcome.set(result),
            Err(error) => {
                eprintln!("screenshot failed: {error:#}");
                outcome.set(ScreenshotOutcome::Failed);
            }
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

#[cfg(feature = "screenshot")]
async fn capture_when_settled(
    window: &AnyWindowHandle,
    shell: &Entity<AppShell>,
    request: &ScreenshotRequest,
    cx: &mut gpui_kit::AsyncApp,
) -> anyhow::Result<ScreenshotOutcome> {
    let mut waited = Duration::ZERO;
    let mut is_settled = false;
    while waited < SETTLE_TIMEOUT {
        let (failure, settled) = shell.update(cx, |shell, cx| {
            let failure = shell.launch_failure().map(str::to_owned);
            (
                failure,
                is_screen_settled(request.screen, &shell.settle_input(cx)),
            )
        });
        // A request for a CRD that does not exist captures nothing: the fallback is not the target.
        if let Some(message) = failure {
            anyhow::bail!(message);
        }
        is_settled = settled;
        if is_settled {
            break;
        }
        cx.background_executor().timer(POLL_INTERVAL).await;
        waited += POLL_INTERVAL;
    }
    cx.background_executor().timer(SETTLE_DELAY).await;
    if request.screen.opens_menu() {
        // The ⋯ menu of the drawer opens as a click opens it, so the capture shows its items.
        window.update(cx, |_, window, cx| {
            window.refresh();
            window.click("drawer-menu", cx);
        })?;
        cx.background_executor().timer(SETTLE_DELAY).await;
    }
    window.update(cx, |_, window, _| window.refresh())?;
    cx.background_executor().timer(POLL_INTERVAL).await;

    let image = window.update(cx, |_, window, _| window.render_to_image())??;
    if let Some(parent) = request.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image.save(&request.path)?;
    // An open popover leaves its input focused, and the blink timer of a focused input is a handle the
    // leak check of this build reports at exit. Closing the switcher and the value popover moves the
    // focus back first, and so does closing the dialogs: the command palette leaves its query input
    // focused.
    shell.update(cx, |shell, cx| {
        shell.close_cluster_switcher(cx);
        shell.close_value_popover(cx);
    });
    window.update(cx, |_, window, cx| window.close_all_dialogs(cx))?;
    window.update(cx, |_, window, _| window.refresh())?;
    cx.background_executor().timer(SETTLE_DELAY).await;
    if is_settled {
        eprintln!(
            "screenshot saved: {} ({}x{})",
            request.path.display(),
            image.width(),
            image.height()
        );
        Ok(ScreenshotOutcome::Saved)
    } else {
        eprintln!("screenshot saved after timeout (screen not settled)");
        Ok(ScreenshotOutcome::TimedOut)
    }
}

#[cfg(test)]
mod tests {
    use cluster::{
        ContainerKind, ContainerState, ContainerSummary, PodStatus, ReadyCount, StatusReason,
    };

    use super::*;
    use crate::drawer::DrawerTab;
    use crate::resource_kind::ResourceKind;

    fn container(name: &str) -> ContainerSummary {
        ContainerSummary {
            name: name.to_owned(),
            image: "img".to_owned(),
            kind: ContainerKind::Main,
            state: ContainerState::NotReported,
            is_ready: false,
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

    fn pod(name: &str, container_count: usize) -> PodSummary {
        PodSummary {
            namespace: "ns".to_owned(),
            name: name.to_owned(),
            status: PodStatus::Reason(StatusReason::Running),
            ready: ReadyCount { ready: 0, total: 0 },
            restarts: 0,
            node_name: None,
            created_at: None,
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: None,
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            containers: (0..container_count)
                .map(|index| container(&format!("c{index}")))
                .collect(),
        }
    }

    fn progress(status: FeedStatus, ticks: u64) -> FeedProgress {
        FeedProgress { status, ticks }
    }

    fn input(target: TargetState, is_drawer_ready: bool) -> SettleInput {
        SettleInput {
            target,
            is_catalog_loading: false,
            is_drawer_ready,
            is_log_pending: false,
            is_dialog_pending: false,
            is_switcher_pending: false,
            is_change_feed_pending: false,
            is_topology_pending: false,
            pod_metrics: progress(FeedStatus::Live, 1),
            node_metrics: progress(FeedStatus::Live, 1),
            kubelet: progress(FeedStatus::Live, 4),
        }
    }

    #[test]
    fn a_fixed_dialog_is_settled_without_a_cluster() {
        for screen in [
            LaunchScreen::NodeShellConfirm,
            LaunchScreen::ShellConfirmFixture,
        ] {
            for target in [
                TargetState::Loading,
                TargetState::Unavailable,
                TargetState::Loaded,
            ] {
                assert!(
                    is_screen_settled(screen, &input(target, false)),
                    "{screen:?}"
                );
            }
            // Until its dialog has opened.
            let pending = SettleInput {
                is_dialog_pending: true,
                ..input(TargetState::Loading, false)
            };
            assert!(!is_screen_settled(screen, &pending), "{screen:?}");
        }
    }

    #[test]
    fn settings_screen_waits_for_catalog() {
        use crate::settings_window::{SettingsPage, SettingsSize};
        let screen = LaunchScreen::Settings(SettingsPage::Clusters, SettingsSize::Standard);
        let loading = SettleInput {
            is_catalog_loading: true,
            ..input(TargetState::Loading, false)
        };
        assert!(!is_screen_settled(screen, &loading));
        // The main window's session plays no part.
        assert!(is_screen_settled(
            screen,
            &input(TargetState::Loading, false)
        ));
    }

    #[test]
    fn settled_when_session_failed() {
        for screen in [
            LaunchScreen::Pods,
            LaunchScreen::PodDrawer(DrawerTab::Containers),
        ] {
            assert!(is_screen_settled(
                screen,
                &input(TargetState::Unavailable, false)
            ));
        }
    }

    #[test]
    fn not_settled_while_target_list_loading() {
        assert!(!is_screen_settled(
            LaunchScreen::Pods,
            &input(TargetState::Loading, false)
        ));
        assert!(is_screen_settled(
            LaunchScreen::Nodes,
            &input(TargetState::Loaded, false)
        ));
    }

    #[test]
    fn kind_drawer_screen_needs_selection() {
        let screen = LaunchScreen::KindDrawer(ResourceKind::Namespaces, DrawerTab::Overview);
        assert!(!is_screen_settled(
            screen,
            &input(TargetState::Loaded, false)
        ));
        assert!(is_screen_settled(screen, &input(TargetState::Loaded, true)));
        let list_screen = LaunchScreen::Kind(ResourceKind::Namespaces);
        assert!(is_screen_settled(
            list_screen,
            &input(TargetState::Loaded, false)
        ));
        assert!(!is_screen_settled(
            list_screen,
            &input(TargetState::Loading, false)
        ));
    }

    #[test]
    fn switcher_screen_waits_for_the_popover_and_its_probes() {
        let pending = SettleInput {
            is_switcher_pending: true,
            ..input(TargetState::Loaded, false)
        };
        assert!(!is_screen_settled(LaunchScreen::Switcher, &pending));
        assert!(is_screen_settled(
            LaunchScreen::Switcher,
            &input(TargetState::Loaded, false)
        ));
        // Other screens ignore the switcher.
        assert!(is_screen_settled(LaunchScreen::Pods, &pending));
    }

    #[test]
    fn dialog_screen_waits_for_its_answer() {
        let mut pending = input(TargetState::Loaded, false);
        pending.is_dialog_pending = true;
        assert!(!is_screen_settled(LaunchScreen::WhoCan, &pending));
        assert!(is_screen_settled(
            LaunchScreen::WhoCan,
            &input(TargetState::Loaded, false)
        ));
        // Other screens ignore a dialog.
        assert!(is_screen_settled(LaunchScreen::Pods, &pending));
    }

    #[test]
    fn drawer_screen_needs_selection_to_settle() {
        for screen in [
            LaunchScreen::PodDrawer(DrawerTab::Overview),
            LaunchScreen::PodDrawer(DrawerTab::Containers),
            LaunchScreen::PodDrawer(DrawerTab::Events),
            LaunchScreen::NodeDrawer(DrawerTab::Overview),
            LaunchScreen::NodeDrawer(DrawerTab::Events),
            LaunchScreen::KindDrawer(ResourceKind::Deployments, DrawerTab::Overview),
            LaunchScreen::KindDrawer(ResourceKind::Deployments, DrawerTab::Events),
        ] {
            assert!(!is_screen_settled(
                screen,
                &input(TargetState::Loaded, false)
            ));
            assert!(is_screen_settled(screen, &input(TargetState::Loaded, true)));
        }
    }

    #[test]
    fn logs_screen_waits_for_log_stream() {
        for screen in [LaunchScreen::LogsDock, LaunchScreen::LogsZoomed] {
            let pending = SettleInput {
                is_log_pending: true,
                ..input(TargetState::Loaded, false)
            };
            assert!(!is_screen_settled(screen, &pending));
            assert!(is_screen_settled(
                screen,
                &input(TargetState::Loaded, false)
            ));
            assert!(is_screen_settled(
                screen,
                &input(TargetState::Unavailable, false)
            ));
        }
    }

    #[test]
    fn usage_screens_wait_for_their_metrics_feed() {
        let feeds = |pods: u64, nodes: u64| SettleInput {
            pod_metrics: progress(FeedStatus::Waiting, pods),
            node_metrics: progress(FeedStatus::Waiting, nodes),
            ..input(TargetState::Loaded, true)
        };
        let pod_screens = [
            LaunchScreen::Pods,
            LaunchScreen::PodDrawer(DrawerTab::Containers),
        ];
        for screen in pod_screens {
            assert!(!is_screen_settled(screen, &feeds(0, 1)));
            assert!(is_screen_settled(screen, &feeds(1, 0)));
        }
        for screen in [
            LaunchScreen::Nodes,
            LaunchScreen::NodeDrawer(DrawerTab::Overview),
        ] {
            assert!(!is_screen_settled(screen, &feeds(1, 0)));
            assert!(is_screen_settled(screen, &feeds(0, 1)));
        }
        // Regression screens do not depend on metrics.
        let kind = LaunchScreen::Kind(ResourceKind::Deployments);
        assert!(is_screen_settled(kind, &feeds(0, 0)));
        assert!(is_screen_settled(LaunchScreen::LogsDock, &feeds(0, 0)));
        // A failed feed is "settled": the screen shows its dashes.
        let failed = SettleInput {
            pod_metrics: progress(FeedStatus::Failed("no".to_owned()), 0),
            ..feeds(0, 0)
        };
        assert!(is_screen_settled(LaunchScreen::Pods, &failed));
    }

    #[test]
    fn monitor_screens_wait_for_two_ticks() {
        let feeds = |pods: u64, nodes: u64| SettleInput {
            pod_metrics: progress(FeedStatus::Live, pods),
            node_metrics: progress(FeedStatus::Live, nodes),
            ..input(TargetState::Loaded, true)
        };
        let pod = LaunchScreen::PodDrawer(DrawerTab::Monitor);
        let workload = LaunchScreen::KindDrawer(ResourceKind::Deployments, DrawerTab::Monitor);
        for screen in [pod, workload] {
            assert!(!is_screen_settled(screen, &feeds(1, 5)));
            assert!(is_screen_settled(screen, &feeds(2, 0)));
        }
        let node = LaunchScreen::NodeDrawer(DrawerTab::Monitor);
        assert!(!is_screen_settled(node, &feeds(5, 1)));
        assert!(is_screen_settled(node, &feeds(0, 2)));
        // An unavailable feed settles at once.
        let denied = SettleInput {
            pod_metrics: progress(FeedStatus::Unavailable("denied".to_owned()), 0),
            ..feeds(0, 0)
        };
        assert!(is_screen_settled(pod, &denied));
    }

    #[test]
    fn overview_waits_for_issue_summary() {
        // The issue board still running is part of `TargetState::Loading` (`settle_input` reads
        // `is_issues_pending`), so Needs attention is never captured as its spinner.
        assert!(!is_screen_settled(
            LaunchScreen::Overview,
            &input(TargetState::Loading, true)
        ));
        assert!(is_screen_settled(
            LaunchScreen::Overview,
            &input(TargetState::Loaded, true)
        ));
    }

    #[test]
    fn overview_waits_for_change_feed() {
        let pending = SettleInput {
            is_change_feed_pending: true,
            ..input(TargetState::Loaded, true)
        };
        assert!(!is_screen_settled(LaunchScreen::Overview, &pending));
        // Other screens do not read the change feed.
        assert!(is_screen_settled(LaunchScreen::Pods, &pending));
        assert!(is_screen_settled(
            LaunchScreen::Overview,
            &input(TargetState::Loaded, true)
        ));
    }

    #[test]
    fn topology_waits_for_feeds_and_build() {
        let pending = SettleInput {
            is_topology_pending: true,
            ..input(TargetState::Loaded, true)
        };
        for screen in [
            LaunchScreen::Topology,
            LaunchScreen::TopologyProblems,
            LaunchScreen::TopologySelected,
        ] {
            assert!(!is_screen_settled(screen, &pending));
            assert!(is_screen_settled(screen, &input(TargetState::Loaded, true)));
        }
        // Other screens do not read the feeds.
        assert!(is_screen_settled(LaunchScreen::Pods, &pending));
    }

    #[test]
    fn overview_waits_for_node_metrics_and_kubelet() {
        let overview = LaunchScreen::Overview;
        let feeds = |nodes: u64, kubelet: u64| SettleInput {
            node_metrics: progress(FeedStatus::Waiting, nodes),
            kubelet: progress(FeedStatus::Live, kubelet),
            ..input(TargetState::Loaded, true)
        };
        assert!(!is_screen_settled(overview, &feeds(0, 4)));
        assert!(!is_screen_settled(overview, &feeds(1, 3)));
        assert!(is_screen_settled(overview, &feeds(1, 4)));
        // Pod metrics play no part: Overview shows no pod usage.
        let pods_pending = SettleInput {
            pod_metrics: progress(FeedStatus::Waiting, 0),
            ..feeds(1, 4)
        };
        assert!(is_screen_settled(overview, &pods_pending));
    }

    #[test]
    fn feed_without_targets_is_settled() {
        let waiting = kubelet_progress(FeedStatus::Waiting, 0, false);
        assert!(waiting.is_settled(4));
        let with_targets = kubelet_progress(FeedStatus::Waiting, 0, true);
        assert!(!with_targets.is_settled(4));
    }

    #[test]
    fn monitor_screens_also_wait_for_kubelet_ticks() {
        let kubelet = |status: FeedStatus, ticks: u64| SettleInput {
            pod_metrics: progress(FeedStatus::Live, 5),
            node_metrics: progress(FeedStatus::Live, 5),
            kubelet: progress(status, ticks),
            ..input(TargetState::Loaded, true)
        };
        let screens = [
            LaunchScreen::PodDrawer(DrawerTab::Monitor),
            LaunchScreen::NodeDrawer(DrawerTab::Monitor),
            LaunchScreen::KindDrawer(ResourceKind::Deployments, DrawerTab::Monitor),
        ];
        for screen in screens {
            assert!(!is_screen_settled(screen, &kubelet(FeedStatus::Waiting, 0)));
            assert!(!is_screen_settled(screen, &kubelet(FeedStatus::Live, 3)));
            assert!(is_screen_settled(screen, &kubelet(FeedStatus::Live, 4)));
            // A feed that is denied, failed, or interrupted will not get better soon.
            let down = FeedStatus::Unavailable("denied".to_owned());
            assert!(is_screen_settled(screen, &kubelet(down, 0)));
            let failed = FeedStatus::Failed("no".to_owned());
            assert!(is_screen_settled(screen, &kubelet(failed, 0)));
        }
        // Screens without Network and Disk I/O do not wait for the kubelet.
        let pods = LaunchScreen::Pods;
        assert!(is_screen_settled(pods, &kubelet(FeedStatus::Waiting, 0)));
        let containers = LaunchScreen::PodDrawer(DrawerTab::Containers);
        assert!(is_screen_settled(
            containers,
            &kubelet(FeedStatus::Waiting, 0)
        ));
    }

    #[test]
    fn logs_pod_prefers_running_default_container() {
        let mut running = pod("b", 1);
        running.containers[0].state = ContainerState::Running { started_at: None };
        let pods = [pod("a", 3), running];
        assert_eq!(pick_logs_pod(&pods), Some(1));
    }

    #[test]
    fn logs_pod_falls_back_to_drawer_pod_without_running_container() {
        let none_running = [pod("a", 1), pod("b", 3)];
        assert_eq!(pick_logs_pod(&none_running), pick_drawer_pod(&none_running));
    }

    #[test]
    fn logs_pod_is_none_for_empty_list() {
        assert_eq!(pick_logs_pod(&[]), None);
    }

    #[test]
    fn controller_owner_of_maps_known_kinds() {
        let owned_by = |kind: &str| {
            let mut owned = pod("p", 1);
            owned.controller = Some(cluster::ControllerRef {
                kind: kind.to_owned(),
                name: "owner".to_owned(),
            });
            owned
        };
        for kind in [
            REPLICA_SET_KIND,
            STATEFUL_SET_KIND,
            DAEMON_SET_KIND,
            JOB_KIND,
        ] {
            assert_eq!(
                controller_owner_of(&owned_by(kind)),
                Some(PodOwner::Controller {
                    namespace: "ns".to_owned(),
                    kind,
                    name: "owner".to_owned(),
                })
            );
        }
        assert_eq!(controller_owner_of(&owned_by("Node")), None);
        assert_eq!(controller_owner_of(&pod("bare", 1)), None);
    }
    #[test]
    fn pods_multi_settles_when_slots_are_live_or_failed() {
        let settled = |states: &[TargetState]| {
            let input = input(combine_targets(states.iter().copied()), false);
            is_screen_settled(LaunchScreen::PodsMulti, &input)
        };
        // Every slot Live with a loaded list, or Failed.
        assert!(settled(&[TargetState::Loaded, TargetState::Loaded]));
        assert!(settled(&[TargetState::Loaded, TargetState::Unavailable]));
        // Every slot failed: the error screen is the target.
        assert!(settled(&[
            TargetState::Unavailable,
            TargetState::Unavailable
        ]));
        // One slot still connecting or loading holds the screen.
        assert!(!settled(&[TargetState::Loaded, TargetState::Loading]));
        assert!(!settled(&[TargetState::Unavailable, TargetState::Loading]));
    }

    #[test]
    fn pods_multi_waits_for_the_slowest_metrics_feed() {
        let waiting = progress(FeedStatus::Waiting, 0);
        let live = progress(FeedStatus::Live, 3);
        let down = progress(FeedStatus::Unavailable("denied".to_owned()), 0);
        // The cluster that has not ticked holds the screen; a denied feed does not.
        assert_eq!(
            slowest_feed([live.clone(), waiting.clone()].into_iter()),
            waiting
        );
        assert_eq!(slowest_feed([down.clone(), live.clone()].into_iter()), live);
        assert_eq!(
            slowest_feed(std::iter::empty()),
            FeedProgress::unavailable()
        );
        let screen = LaunchScreen::PodsMulti;
        let pending = SettleInput {
            pod_metrics: slowest_feed([live, waiting].into_iter()),
            ..input(TargetState::Loaded, false)
        };
        assert!(!is_screen_settled(screen, &pending));
    }

    #[test]
    fn outcome_exit_codes() {
        assert_eq!(ScreenshotOutcome::Saved.exit_code(), 0);
        assert_eq!(ScreenshotOutcome::TimedOut.exit_code(), 3);
        assert_eq!(ScreenshotOutcome::Failed.exit_code(), 1);
    }

    #[test]
    fn pod_drawer_prefers_first_multi_container_pod() {
        let pods = [pod("a", 1), pod("b", 3), pod("c", 2)];
        assert_eq!(pick_drawer_pod(&pods), Some(1));
        let single = [pod("a", 1), pod("b", 1)];
        assert_eq!(pick_drawer_pod(&single), Some(0));
        assert_eq!(pick_drawer_pod(&[]), None);
    }

    #[test]
    fn select_picks_by_name_or_namespace_and_name() {
        let items = || [(Some("a"), "api"), (Some("b"), "api"), (None, "node-1")].into_iter();
        assert_eq!(pick_selected("api", items()), Some(0));
        assert_eq!(pick_selected("b/api", items()), Some(1));
        assert_eq!(pick_selected("node-1", items()), Some(2));
    }

    #[test]
    fn select_finds_nothing_for_an_unknown_or_partial_name() {
        let items = || [(Some("a"), "api")].into_iter();
        assert_eq!(pick_selected("web", items()), None);
        assert_eq!(pick_selected("c/api", items()), None);
        assert_eq!(pick_selected("a/ap", items()), None);
        // A dash is not a slash: `a-api` names no row, so no drawer opens (the screen still settles).
        assert_eq!(pick_selected("a-api", items()), None);
    }

    #[test]
    fn drawer_waits_for_its_content() {
        // Selected, nothing pending.
        assert!(is_drawer_ready(true, false, false));
        // Selected, but the events are still pending.
        assert!(!is_drawer_ready(true, false, true));
        // The launch request found no row to select.
        assert!(is_drawer_ready(false, false, false));
        // The launch request still waits for its list.
        assert!(!is_drawer_ready(false, true, false));
    }
}
