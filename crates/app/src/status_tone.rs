use cluster::{
    ConditionStatus, ContainerKind, ContainerState, ContainerSummary, InitStatus, NodeCondition,
    NodeReadiness, NodeScheduling, NodeStatus, PodStatus, PodSummary, StatusReason,
};
use gpui_kit::component::{ActiveTheme as _, Colorize as _};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusLabel {
    pub(crate) text: SharedString,
    pub(crate) tone: StatusTone,
}

/// The share of its own hue a tone keeps on a light background. It is the `factor` of
/// `mix_oklab`, which computes `self * factor + other * (1 - factor)`, so at 0.6 the tone
/// moves 40% toward the foreground to reach a readable contrast.
const LIGHT_THEME_TONE_SHARE: f32 = 0.6;
/// Amber is the lightest tone, so on white it keeps less of its hue; that gives it the contrast
/// of the green and the red (about 5.9 : 1) instead of a washed-out olive.
const LIGHT_THEME_WARN_SHARE: f32 = 0.5;

/// The only place a tone touches the theme, so every status colour comes from one table.
pub(crate) fn tone_color(tone: StatusTone, cx: &App) -> Hsla {
    let theme = cx.theme();
    let color = match tone {
        StatusTone::Ok => theme.success,
        StatusTone::Warn => theme.warning,
        StatusTone::Bad => theme.danger,
        StatusTone::Info => theme.info,
        // Already a text token; blending it would make "Completed" look like normal text.
        StatusTone::Done => return theme.muted_foreground,
    };
    // The success/warning/danger/info colours are tuned as fills; as text they wash out on white.
    if theme.is_dark() {
        color
    } else {
        let share = match tone {
            StatusTone::Warn => LIGHT_THEME_WARN_SHARE,
            _ => LIGHT_THEME_TONE_SHARE,
        };
        readable_on_light(color, theme.foreground, share)
    }
}

fn readable_on_light(color: Hsla, foreground: Hsla, share: f32) -> Hsla {
    color.mix_oklab(foreground, share)
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

/// The node readiness as the API words it: `Ready`, `NotReady`, or `Unknown`.
pub(crate) fn readiness_text(readiness: NodeReadiness) -> &'static str {
    match readiness {
        NodeReadiness::Ready => "Ready",
        NodeReadiness::NotReady => "NotReady",
        NodeReadiness::Unknown => "Unknown",
    }
}

pub(crate) fn node_status_label(status: NodeStatus) -> StatusLabel {
    let readiness_tone = match status.readiness {
        NodeReadiness::Ready => StatusTone::Ok,
        NodeReadiness::NotReady => StatusTone::Bad,
        NodeReadiness::Unknown => StatusTone::Warn,
    };
    let readiness_text = readiness_text(status.readiness);
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

/// Ready: True is Ok, False is Bad, Unknown is Warn. Every other type (pressure,
/// NetworkUnavailable, node-problem-detector conditions) reports a problem when True: True is
/// Bad, False is Ok, Unknown is Warn.
pub(crate) fn node_condition_tone(condition: &NodeCondition) -> StatusTone {
    let is_healthy_when_true = condition.name == "Ready";
    match (condition.status, is_healthy_when_true) {
        (ConditionStatus::Unknown, _) => StatusTone::Warn,
        (ConditionStatus::True, true) | (ConditionStatus::False, false) => StatusTone::Ok,
        (ConditionStatus::True, false) | (ConditionStatus::False, true) => StatusTone::Bad,
    }
}

pub(crate) fn condition_status_text(status: ConditionStatus) -> &'static str {
    match status {
        ConditionStatus::True => "True",
        ConditionStatus::False => "False",
        ConditionStatus::Unknown => "Unknown",
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
        ContainerState::Waiting { reason, .. } => StatusLabel {
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

pub(crate) fn is_bad_reason(reason: &StatusReason) -> bool {
    matches!(
        reason,
        StatusReason::CrashLoopBackOff
            | StatusReason::ImagePullBackOff
            | StatusReason::ErrImagePull
            | StatusReason::CreateContainerConfigError
            | StatusReason::InvalidImageName
            | StatusReason::ErrImageNeverPull
            | StatusReason::CreateContainerError
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
