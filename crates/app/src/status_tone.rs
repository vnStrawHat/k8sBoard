use cluster::{
    ContainerKind, ContainerState, ContainerSummary, InitStatus, NodeReadiness, NodeScheduling,
    NodeStatus, PodStatus, PodSummary, StatusReason,
};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Div, Hsla, ParentElement as _, SharedString, Styled as _, div};

/// How a status reads at a glance. Each tone maps to one theme token in `tone_color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusTone {
    Ok,
    Warn,
    Bad,
    Info,
    Done,
}

pub(crate) struct StatusLabel {
    pub(crate) text: SharedString,
    pub(crate) tone: StatusTone,
}

/// The only place a tone touches the theme, so every status colour comes from one table.
pub(crate) fn tone_color(tone: StatusTone, cx: &App) -> Hsla {
    let theme = cx.theme();
    match tone {
        // The pressed-state green is darker, which reads better as text on a light background.
        StatusTone::Ok if theme.is_dark() => theme.success,
        StatusTone::Ok => theme.success_active,
        StatusTone::Warn => theme.warning,
        StatusTone::Bad => theme.danger,
        StatusTone::Info => theme.info,
        StatusTone::Done => theme.muted_foreground,
    }
}

/// The label text coloured by its tone.
pub(crate) fn toned_text(label: StatusLabel, cx: &App) -> Div {
    div()
        .text_color(tone_color(label.tone, cx))
        .child(label.text)
}

pub(crate) fn pod_status_label(pod: &PodSummary) -> StatusLabel {
    if is_readiness_failed(pod) {
        return StatusLabel {
            text: "Readiness failed".into(),
            tone: StatusTone::Warn,
        };
    }
    let tone = match &pod.status {
        PodStatus::Reason(reason) => reason_tone(reason),
        PodStatus::Init(InitStatus::Reason(reason)) => reason_tone(reason),
        PodStatus::Init(InitStatus::Progress { .. }) | PodStatus::Terminating => StatusTone::Info,
        PodStatus::NotReady => StatusTone::Warn,
    };
    StatusLabel {
        text: pod.status.to_string().into(),
        tone,
    }
}

/// A running pod whose containers all run but are not all ready is failing its readiness
/// probe. kubectl still prints `Running` for it, so this is a UI rule on top of the cluster
/// status. It also shows during a probe's initial delay.
fn is_readiness_failed(pod: &PodSummary) -> bool {
    pod.status == PodStatus::Reason(StatusReason::Running)
        && pod.ready.ready < pod.ready.total
        && pod
            .containers
            .iter()
            .filter(|container| container.kind != ContainerKind::Init)
            .all(|container| matches!(container.state, ContainerState::Running { .. }))
}

pub(crate) fn node_status_label(status: NodeStatus) -> StatusLabel {
    let (readiness_text, readiness_tone) = match status.readiness {
        NodeReadiness::Ready => ("Ready", StatusTone::Ok),
        NodeReadiness::NotReady => ("NotReady", StatusTone::Bad),
        NodeReadiness::Unknown => ("Unknown", StatusTone::Warn),
    };
    match status.scheduling {
        NodeScheduling::Enabled => StatusLabel {
            text: readiness_text.into(),
            tone: readiness_tone,
        },
        NodeScheduling::Disabled => StatusLabel {
            text: format!("{readiness_text} · SchedulingDisabled").into(),
            // A cordoned but healthy node is a warning; an unhealthy one stays as bad.
            tone: if readiness_tone == StatusTone::Ok {
                StatusTone::Warn
            } else {
                readiness_tone
            },
        },
    }
}

pub(crate) fn container_state_label(container: &ContainerSummary) -> StatusLabel {
    match &container.state {
        ContainerState::Running { .. } => StatusLabel {
            text: "Running".into(),
            tone: if container.is_ready {
                StatusTone::Ok
            } else {
                StatusTone::Warn
            },
        },
        ContainerState::Waiting { reason } => StatusLabel {
            text: reason
                .as_ref()
                .map_or_else(|| "Waiting".to_owned(), ToString::to_string)
                .into(),
            tone: match reason {
                Some(reason) if is_bad_reason(reason) => StatusTone::Bad,
                _ => StatusTone::Info,
            },
        },
        ContainerState::Terminated(termination) => {
            let is_clean_exit = termination.exit_code == 0;
            let fallback = if is_clean_exit { "Completed" } else { "Error" };
            StatusLabel {
                text: termination
                    .reason
                    .as_ref()
                    .map_or_else(|| fallback.to_owned(), ToString::to_string)
                    .into(),
                tone: if is_clean_exit {
                    StatusTone::Done
                } else {
                    StatusTone::Bad
                },
            }
        }
        ContainerState::NotReported => StatusLabel {
            text: "Not reported".into(),
            tone: StatusTone::Done,
        },
    }
}

fn reason_tone(reason: &StatusReason) -> StatusTone {
    if is_bad_reason(reason) {
        return StatusTone::Bad;
    }
    match reason {
        StatusReason::Running => StatusTone::Ok,
        StatusReason::ContainerCreating | StatusReason::PodInitializing => StatusTone::Info,
        StatusReason::Completed | StatusReason::Succeeded => StatusTone::Done,
        _ => StatusTone::Warn,
    }
}

fn is_bad_reason(reason: &StatusReason) -> bool {
    matches!(
        reason,
        StatusReason::CrashLoopBackOff
            | StatusReason::ImagePullBackOff
            | StatusReason::ErrImagePull
            | StatusReason::CreateContainerConfigError
            | StatusReason::OomKilled
            | StatusReason::Error
            | StatusReason::ContainerCannotRun
            | StatusReason::Failed
            | StatusReason::Evicted
            | StatusReason::Signal(_)
            | StatusReason::ExitCode(_)
    )
}

#[cfg(test)]
#[path = "status_tone_tests.rs"]
mod status_tone_tests;
