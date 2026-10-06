use cluster::{
    ConditionStatus, ContainerKind, ContainerState, ContainerSummary, InitStatus, NodeCondition,
    NodeReadiness, NodeScheduling, NodeStatus, PodStatus, PodSummary, StatusReason,
};
use gpui_kit::component::{ActiveTheme as _, Colorize as _};
use gpui_kit::{App, Div, Hsla, ParentElement as _, Rgba, SharedString, Styled as _, div};

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

/// The smallest share of its own hue a tone keeps on a light background (the cap of the mix:
/// the tone moves at most 40% toward the foreground). The share is the `factor` of `mix_oklab`,
/// which computes `self * factor + other * (1 - factor)`.
const LIGHT_THEME_TONE_SHARE: f32 = 0.6;
/// Amber mixed toward near-black turns grey-brown at small sizes, so it keeps more of its hue.
const LIGHT_THEME_WARN_SHARE: f32 = 0.7;
/// The contrast ratio (WCAG) a tone needs against the background to read as text.
const TEXT_CONTRAST: f32 = 4.5;
/// Steps of the search for the largest share that still reaches `TEXT_CONTRAST`.
const SHARE_SEARCH_STEPS: u32 = 12;

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
        readable_on_light(color, theme.foreground, theme.background, share)
    }
}

/// `color` moved toward `foreground` only as far as it needs to reach `TEXT_CONTRAST` on
/// `background`, and never further than `min_share` allows. A fill that already reads is kept,
/// and a muted fill keeps its hue instead of a fixed mix turning it grey.
fn readable_on_light(color: Hsla, foreground: Hsla, background: Hsla, min_share: f32) -> Hsla {
    let reads = |share: f32| {
        let mixed = color.mix_oklab(foreground, share);
        (contrast(mixed, background) >= TEXT_CONTRAST).then_some(mixed)
    };
    // Checked on the color itself: a mix at share 1 still round-trips through Oklab.
    if contrast(color, background) >= TEXT_CONTRAST {
        return color;
    }
    // Contrast grows as the share falls, so a bisection finds the largest share that reads.
    let (mut low, mut high) = (min_share, 1.);
    for _ in 0..SHARE_SEARCH_STEPS {
        let middle = (low + high) / 2.;
        if reads(middle).is_some() {
            low = middle;
        } else {
            high = middle;
        }
    }
    reads(low).unwrap_or_else(|| color.mix_oklab(foreground, min_share))
}

/// The contrast ratio (WCAG 2) of two colors.
pub(crate) fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The relative luminance of an sRGB color (WCAG 2).
fn luminance(color: Hsla) -> f32 {
    let rgba = Rgba::from(color);
    let linear = |channel: f32| {
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b)
}

/// The share of its own hue a chart color keeps as text. Chart colors are fills tuned for bars,
/// so as text they are pulled toward the foreground: toward black on a light theme, toward white
/// on a dark one.
const LIGHT_THEME_CHART_TEXT_SHARE: f32 = 0.45;
const DARK_THEME_CHART_TEXT_SHARE: f32 = 0.55;

/// A chart color that stays readable as text or a small dot on the current theme background.
pub(crate) fn readable_chart_color(color: Hsla, cx: &App) -> Hsla {
    let theme = cx.theme();
    chart_text_color(color, theme.foreground, theme.is_dark())
}

fn chart_text_color(color: Hsla, foreground: Hsla, is_dark: bool) -> Hsla {
    let share = if is_dark {
        DARK_THEME_CHART_TEXT_SHARE
    } else {
        LIGHT_THEME_CHART_TEXT_SHARE
    };
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

/// The kubelet's resource pressure condition types, in the order they are named.
pub(crate) const PRESSURE_CONDITIONS: [&str; 3] = ["MemoryPressure", "DiskPressure", "PIDPressure"];

/// The pressure conditions that are `True` now.
pub(crate) fn active_pressures(
    conditions: &[NodeCondition],
) -> impl Iterator<Item = &NodeCondition> {
    PRESSURE_CONDITIONS.iter().filter_map(|name| {
        conditions
            .iter()
            .find(|condition| condition.name == *name && condition.status == ConditionStatus::True)
    })
}

/// The status of a node in the list and the drawer: readiness and scheduling, then the active
/// pressure conditions (`Ready · DiskPressure`), which turn an otherwise green label to a warning.
pub(crate) fn node_status_label(status: NodeStatus, conditions: &[NodeCondition]) -> StatusLabel {
    let label = readiness_label(status);
    let pressures: Vec<&str> = active_pressures(conditions)
        .map(|condition| condition.name.as_str())
        .collect();
    if pressures.is_empty() {
        return label;
    }
    StatusLabel {
        text: format!("{} · {}", label.text, pressures.join(", ")).into(),
        tone: match label.tone {
            StatusTone::Ok => StatusTone::Warn,
            tone => tone,
        },
    }
}

fn readiness_label(status: NodeStatus) -> StatusLabel {
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
        // A cordoned but healthy node is a warning; an unhealthy one stays as bad. Readiness is
        // spelled out only when it is the worse news, so the plain word fits a narrow column.
        NodeScheduling::Disabled if readiness_tone == StatusTone::Ok => StatusLabel {
            text: "Cordoned".into(),
            tone: StatusTone::Warn,
        },
        NodeScheduling::Disabled => StatusLabel {
            text: format!("{readiness_text} · Cordoned").into(),
            tone: readiness_tone,
        },
    }
}

/// The drawer's Scheduling row: whether new pods may land on the node.
pub(crate) fn scheduling_label(scheduling: NodeScheduling) -> StatusLabel {
    match scheduling {
        NodeScheduling::Enabled => StatusLabel {
            text: "Schedulable".into(),
            tone: StatusTone::Ok,
        },
        NodeScheduling::Disabled => StatusLabel {
            text: "Cordoned".into(),
            tone: StatusTone::Warn,
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
            let mut text = termination
                .reason
                .as_ref()
                .map_or_else(|| fallback.to_owned(), ToString::to_string);
            // A failed container names its exit code, so the reason (Error, OOMKilled) is not all
            // there is to read; the reason stays as the server gave it.
            if !is_clean_exit {
                text.push_str(&format!(" · exit {}", termination.exit_code));
            }
            StatusLabel {
                text: text.into(),
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
