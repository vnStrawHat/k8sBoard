use cluster::PodSummary;
#[cfg(feature = "screenshot")]
use gpui_kit::{AnyWindowHandle, App, Entity};

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
    pub(crate) has_selection: bool,
}

/// Whether the screen shows what `--screen` asked for, so a screenshot is worth taking.
#[cfg(any(feature = "screenshot", test))]
pub(crate) fn is_screen_settled(screen: LaunchScreen, input: &SettleInput) -> bool {
    let is_drawer_screen = matches!(
        screen,
        LaunchScreen::PodDrawer | LaunchScreen::PodContainers | LaunchScreen::NodeDrawer
    );
    match input.target {
        TargetState::Unavailable => true,
        TargetState::Loading => false,
        TargetState::Loaded => !is_drawer_screen || input.has_selection,
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

    fn input(target: TargetState, has_selection: bool) -> SettleInput {
        SettleInput {
            target,
            has_selection,
        }
    }

    #[test]
    fn settled_when_session_failed() {
        for screen in [LaunchScreen::Pods, LaunchScreen::PodContainers] {
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
    fn drawer_screen_needs_selection_to_settle() {
        for screen in [
            LaunchScreen::PodDrawer,
            LaunchScreen::PodContainers,
            LaunchScreen::NodeDrawer,
        ] {
            assert!(!is_screen_settled(
                screen,
                &input(TargetState::Loaded, false)
            ));
            assert!(is_screen_settled(screen, &input(TargetState::Loaded, true)));
        }
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
}
