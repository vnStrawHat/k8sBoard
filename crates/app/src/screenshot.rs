use cluster::{ContainerState, PodSummary};
#[cfg(feature = "screenshot")]
use gpui_kit::{AnyWindowHandle, App, Entity};

use crate::pod_drawer::default_container;

#[cfg(any(feature = "screenshot", test))]
use crate::launch_options::LaunchScreen;
#[cfg(feature = "screenshot")]
use {
    crate::app_shell::AppShell,
    std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration},
};

#[cfg(feature = "screenshot")]
const POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(feature = "screenshot")]
const SETTLE_TIMEOUT: Duration = Duration::from_secs(30);
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
    /// A drawer screen has its row selected, or found no row to select.
    pub(crate) is_drawer_ready: bool,
    /// A logs screen whose tab is not open yet or still connecting.
    pub(crate) is_log_pending: bool,
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
    match input.target {
        TargetState::Unavailable => true,
        TargetState::Loading => false,
        TargetState::Loaded if screen.has_log_dock() => !input.is_log_pending,
        TargetState::Loaded => !screen.has_drawer() || input.is_drawer_ready,
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
        is_settled = shell.update(cx, |shell, cx| {
            is_screen_settled(request.screen, &shell.settle_input(cx))
        });
        if is_settled {
            break;
        }
        cx.background_executor().timer(POLL_INTERVAL).await;
        waited += POLL_INTERVAL;
    }
    cx.background_executor().timer(SETTLE_DELAY).await;
    window.update(cx, |_, window, _| window.refresh())?;
    cx.background_executor().timer(POLL_INTERVAL).await;

    let image = window.update(cx, |_, window, _| window.render_to_image())??;
    if let Some(parent) = request.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image.save(&request.path)?;
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
            containers: (0..container_count)
                .map(|index| container(&format!("c{index}")))
                .collect(),
        }
    }

    fn input(target: TargetState, is_drawer_ready: bool) -> SettleInput {
        SettleInput {
            target,
            is_drawer_ready,
            is_log_pending: false,
        }
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
