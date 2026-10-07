use cluster::{ReadyCount, Termination};
use gpui_kit::component::ThemeColor;

use super::*;

fn container(kind: ContainerKind, state: ContainerState, is_ready: bool) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: "c".to_owned(),
        image: "img".to_owned(),
        kind,
        state,
        is_ready,
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

fn running() -> ContainerState {
    ContainerState::Running { started_at: None }
}

fn termination(reason: Option<StatusReason>, exit_code: i32) -> Termination {
    Termination {
        reason,
        exit_code,
        signal: None,
        started_at: None,
        finished_at: None,
    }
}

fn pod(status: PodStatus, ready: u32, containers: Vec<ContainerSummary>) -> PodSummary {
    let total = u32::try_from(containers.len()).unwrap_or_default();
    PodSummary {
        is_finished: false,
        namespace: "ns".to_owned(),
        name: "pod".to_owned(),
        status,
        ready: ReadyCount { ready, total },
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
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        containers,
    }
}

fn reason_pod(reason: StatusReason) -> PodSummary {
    pod(PodStatus::Reason(reason), 0, Vec::new())
}

#[test]
fn pod_running_all_ready_is_ok() {
    let all_ready = pod(
        PodStatus::Reason(StatusReason::Running),
        1,
        vec![container(ContainerKind::Main, running(), true)],
    );
    let label = pod_status_label(&all_ready);
    assert_eq!(label.text, "Running");
    assert_eq!(label.tone, StatusTone::Ok);
}

#[test]
fn pod_readiness_failed_when_running_and_not_all_ready() {
    let finished_init = ContainerState::Terminated(termination(Some(StatusReason::Completed), 0));
    let not_ready = pod(
        PodStatus::Reason(StatusReason::Running),
        0,
        vec![
            container(ContainerKind::Init, finished_init, false),
            container(ContainerKind::Main, running(), false),
        ],
    );
    let label = pod_status_label(&not_ready);
    assert_eq!(label.text, "Readiness failed");
    assert_eq!(label.tone, StatusTone::Warn);
}

#[test]
fn pod_running_not_ready_with_waiting_container_keeps_status_text() {
    let waiting = pod(
        PodStatus::Reason(StatusReason::Running),
        0,
        vec![
            container(ContainerKind::Main, running(), false),
            container(
                ContainerKind::Main,
                ContainerState::Waiting {
                    reason: None,
                    message: None,
                },
                false,
            ),
        ],
    );
    let label = pod_status_label(&waiting);
    assert_eq!(label.text, "Running");
    assert_eq!(label.tone, StatusTone::Ok);
}

#[test]
fn pod_bad_reasons_are_bad() {
    let bad = [
        StatusReason::CrashLoopBackOff,
        StatusReason::ImagePullBackOff,
        StatusReason::ErrImagePull,
        StatusReason::CreateContainerConfigError,
        StatusReason::InvalidImageName,
        StatusReason::ErrImageNeverPull,
        StatusReason::CreateContainerError,
        StatusReason::OomKilled,
        StatusReason::Error,
        StatusReason::ContainerCannotRun,
        StatusReason::Failed,
        StatusReason::Evicted,
        StatusReason::Signal(9),
        StatusReason::ExitCode(2),
    ];
    for reason in bad {
        let label = pod_status_label(&reason_pod(reason.clone()));
        assert_eq!(label.tone, StatusTone::Bad, "{reason}");
        assert_eq!(label.text, reason.to_string());
    }
}

#[test]
fn pod_init_reason_takes_reason_tone() {
    let init_bad = pod(
        PodStatus::Init(InitStatus::Reason(StatusReason::CrashLoopBackOff)),
        0,
        Vec::new(),
    );
    let label = pod_status_label(&init_bad);
    assert_eq!(label.text, "Init:CrashLoopBackOff");
    assert_eq!(label.tone, StatusTone::Bad);

    let progress = pod(
        PodStatus::Init(InitStatus::Progress {
            first_incomplete: 1,
            total: 2,
        }),
        0,
        Vec::new(),
    );
    let label = pod_status_label(&progress);
    assert_eq!(label.text, "Init:1/2");
    assert_eq!(label.tone, StatusTone::Info);
}

#[test]
fn pod_completed_is_done_and_terminating_is_info() {
    assert_eq!(
        pod_status_label(&reason_pod(StatusReason::Completed)).tone,
        StatusTone::Done
    );
    assert_eq!(
        pod_status_label(&reason_pod(StatusReason::Succeeded)).tone,
        StatusTone::Done
    );
    let terminating = pod(PodStatus::Terminating, 0, Vec::new());
    assert_eq!(pod_status_label(&terminating).tone, StatusTone::Info);
}

#[test]
fn node_ready_scheduling_disabled_reads_cordoned_in_warn() {
    let label = node_status_label(
        NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling: NodeScheduling::Disabled,
        },
        &[],
    );
    assert_eq!(label.text, "Cordoned");
    assert_eq!(label.tone, StatusTone::Warn);
}

#[test]
fn node_not_ready_is_bad() {
    let cordoned = node_status_label(
        NodeStatus {
            readiness: NodeReadiness::NotReady,
            scheduling: NodeScheduling::Disabled,
        },
        &[],
    );
    assert_eq!(cordoned.text, "NotReady · Cordoned");
    assert_eq!(cordoned.tone, StatusTone::Bad);
    let enabled = node_status_label(
        NodeStatus {
            readiness: NodeReadiness::NotReady,
            scheduling: NodeScheduling::Enabled,
        },
        &[],
    );
    assert_eq!(enabled.text, "NotReady");
    assert_eq!(enabled.tone, StatusTone::Bad);
}

#[test]
fn node_with_a_silent_kubelet_is_bad_like_not_ready() {
    let label = node_status_label(
        NodeStatus {
            readiness: NodeReadiness::Unknown,
            scheduling: NodeScheduling::Enabled,
        },
        &[],
    );
    assert_eq!(label.text, "Unknown");
    assert_eq!(label.tone, StatusTone::Bad);
}

fn node_condition(name: &str, status: ConditionStatus) -> NodeCondition {
    NodeCondition {
        name: name.to_owned(),
        status,
        reason: None,
        message: None,
        changed_at: None,
        last_heartbeat_at: None,
    }
}

const READY_AND_SCHEDULABLE: NodeStatus = NodeStatus {
    readiness: NodeReadiness::Ready,
    scheduling: NodeScheduling::Enabled,
};

#[test]
fn a_ready_node_under_pressure_reads_the_pressure_in_warn() {
    let one = node_status_label(
        READY_AND_SCHEDULABLE,
        &[
            node_condition("Ready", ConditionStatus::True),
            node_condition("DiskPressure", ConditionStatus::True),
        ],
    );
    assert_eq!(one.text, "Ready · DiskPressure");
    assert_eq!(one.tone, StatusTone::Warn);
    // Named in the kubelet's order, not the API's.
    let two = node_status_label(
        READY_AND_SCHEDULABLE,
        &[
            node_condition("PIDPressure", ConditionStatus::True),
            node_condition("MemoryPressure", ConditionStatus::True),
            node_condition("DiskPressure", ConditionStatus::False),
        ],
    );
    assert_eq!(two.text, "Ready · MemoryPressure, PIDPressure");
}

#[test]
fn pressure_that_is_not_true_leaves_the_label_alone() {
    let label = node_status_label(
        READY_AND_SCHEDULABLE,
        &[
            node_condition("MemoryPressure", ConditionStatus::False),
            node_condition("DiskPressure", ConditionStatus::Unknown),
        ],
    );
    assert_eq!(label.text, "Ready");
    assert_eq!(label.tone, StatusTone::Ok);
}

#[test]
fn pressure_is_appended_after_cordoned_and_never_softens_a_bad_node() {
    let pressure = [node_condition("DiskPressure", ConditionStatus::True)];
    let cordoned = node_status_label(
        NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling: NodeScheduling::Disabled,
        },
        &pressure,
    );
    assert_eq!(cordoned.text, "Cordoned · DiskPressure");
    assert_eq!(cordoned.tone, StatusTone::Warn);
    let not_ready = node_status_label(
        NodeStatus {
            readiness: NodeReadiness::NotReady,
            scheduling: NodeScheduling::Enabled,
        },
        &pressure,
    );
    assert_eq!(not_ready.text, "NotReady · DiskPressure");
    assert_eq!(not_ready.tone, StatusTone::Bad);
}

#[test]
fn container_terminated_exit_zero_is_done_nonzero_is_bad() {
    let done = container(
        ContainerKind::Init,
        ContainerState::Terminated(termination(Some(StatusReason::Completed), 0)),
        false,
    );
    assert_eq!(container_state_label(&done).tone, StatusTone::Done);
    let failed = container(
        ContainerKind::Main,
        ContainerState::Terminated(termination(Some(StatusReason::Error), 1)),
        false,
    );
    let label = container_state_label(&failed);
    assert_eq!(label.text, "Error · exit 1");
    assert_eq!(label.tone, StatusTone::Bad);
}

const WHITE: (f32, f32, f32) = (0., 0., 1.);

fn white() -> Hsla {
    gpui_kit::hsla(WHITE.0, WHITE.1, WHITE.2, 1.)
}

fn near_black() -> Hsla {
    gpui_kit::hsla(0., 0., 0.05, 1.)
}

#[test]
fn light_theme_text_reaches_the_text_contrast_or_the_cap() {
    let foreground = near_black();
    for hue in [0.02, 0.12, 0.38, 0.58] {
        for lightness in [0.35, 0.5, 0.65] {
            let fill = gpui_kit::hsla(hue, 0.6, lightness, 1.);
            let cap = LIGHT_THEME_TONE_SHARE;
            let text = readable_on_light(fill, foreground, white(), cap);
            let at_cap = fill.mix_oklab(foreground, cap);
            assert!(
                contrast(text, white()) >= TEXT_CONTRAST - 0.02
                    || contrast(text, white()) >= contrast(at_cap, white()) - 0.02,
                "hue {hue} lightness {lightness} cap {cap}"
            );
            // Never darker than the cap allows.
            assert!(text.l >= at_cap.l - 0.01, "hue {hue} lightness {lightness}");
        }
    }
}

#[test]
fn light_theme_text_keeps_more_hue_than_the_fixed_mix() {
    let foreground = near_black();
    // A muted green that needs less than the full cap to read.
    let fill = gpui_kit::hsla(0.33, 0.35, 0.42, 1.);
    let text = readable_on_light(fill, foreground, white(), LIGHT_THEME_TONE_SHARE);
    let capped = fill.mix_oklab(foreground, LIGHT_THEME_TONE_SHARE);
    assert!(contrast(text, white()) >= TEXT_CONTRAST - 0.02);
    assert!(text.l > capped.l, "{} vs {}", text.l, capped.l);
}

#[test]
fn light_theme_tone_that_already_reads_is_not_mixed() {
    let dark_green = gpui_kit::hsla(0.38, 0.6, 0.2, 1.);
    assert!(contrast(dark_green, white()) >= TEXT_CONTRAST);
    assert_eq!(
        readable_on_light(dark_green, near_black(), white(), LIGHT_THEME_TONE_SHARE),
        dark_green
    );
}

#[test]
fn light_theme_text_is_darker_than_the_fill_colour() {
    let green = gpui_kit::hsla(0.38, 0.6, 0.5, 1.);
    let foreground = near_black();
    let text = readable_on_light(green, foreground, white(), LIGHT_THEME_TONE_SHARE);
    assert!(text.l < green.l);
    assert!(text.l > foreground.l);
}

#[test]
fn default_light_tones_stay_close_to_the_old_fixed_mix() {
    let theme = ThemeColor::light();
    for (fill, cap) in [
        (theme.success, LIGHT_THEME_TONE_SHARE),
        (theme.warning, LIGHT_THEME_TONE_SHARE),
        (theme.danger, LIGHT_THEME_TONE_SHARE),
        (theme.info, LIGHT_THEME_TONE_SHARE),
    ] {
        let now = readable_on_light(fill, theme.foreground, theme.background, cap);
        let old = fill.mix_oklab(theme.foreground, cap);
        // Never less readable than the old mix, nor than the text contrast.
        let needed = TEXT_CONTRAST.min(contrast(old, theme.background));
        assert!(contrast(now, theme.background) >= needed - 0.02);
        assert!(
            (now.l - old.l).abs() < 0.2,
            "lightness {} vs {}",
            now.l,
            old.l
        );
    }
}

#[test]
fn new_reasons_are_bad() {
    for reason in [
        StatusReason::InvalidImageName,
        StatusReason::ErrImageNeverPull,
        StatusReason::CreateContainerError,
    ] {
        assert!(is_bad_reason(&reason), "{reason}");
        let waiting = container(
            ContainerKind::Main,
            ContainerState::Waiting {
                reason: Some(reason.clone()),
                message: None,
            },
            false,
        );
        assert_eq!(
            container_state_label(&waiting).tone,
            StatusTone::Bad,
            "{reason}"
        );
    }
}

#[test]
fn node_condition_tone_table() {
    let tone = |name: &str, status| {
        node_condition_tone(&NodeCondition {
            name: name.to_owned(),
            status,
            reason: None,
            message: None,
            changed_at: None,
            last_heartbeat_at: None,
        })
    };
    use ConditionStatus::{False, True, Unknown};
    assert_eq!(tone("Ready", True), StatusTone::Ok);
    assert_eq!(tone("Ready", False), StatusTone::Bad);
    assert_eq!(tone("Ready", Unknown), StatusTone::Bad);
    for problem in [
        "MemoryPressure",
        "DiskPressure",
        "PIDPressure",
        "KernelDeadlock",
    ] {
        assert_eq!(tone(problem, True), StatusTone::Bad, "{problem}");
        assert_eq!(tone(problem, False), StatusTone::Ok, "{problem}");
        assert_eq!(tone(problem, Unknown), StatusTone::Warn, "{problem}");
    }
}

#[test]
fn condition_status_text_names_each_status() {
    use ConditionStatus::{False, True, Unknown};
    assert_eq!(condition_status_text(True), "True");
    assert_eq!(condition_status_text(False), "False");
    assert_eq!(condition_status_text(Unknown), "Unknown");
}

#[test]
fn chart_text_color_moves_toward_the_foreground_on_both_themes() {
    let pale_blue = gpui_kit::hsla(0.58, 0.8, 0.8, 1.);
    let black = gpui_kit::hsla(0., 0., 0.05, 1.);
    assert!(chart_text_color(pale_blue, black, false).l < pale_blue.l);
    let dark_blue = gpui_kit::hsla(0.62, 0.7, 0.3, 1.);
    let white = gpui_kit::hsla(0., 0., 0.95, 1.);
    assert!(chart_text_color(dark_blue, white, true).l > dark_blue.l);
}

#[test]
fn scheduling_label_names_schedulable_and_cordoned() {
    let schedulable = scheduling_label(NodeScheduling::Enabled);
    assert_eq!(
        (schedulable.text.as_ref(), schedulable.tone),
        ("Schedulable", StatusTone::Ok)
    );
    let cordoned = scheduling_label(NodeScheduling::Disabled);
    assert_eq!(
        (cordoned.text.as_ref(), cordoned.tone),
        ("Cordoned", StatusTone::Warn)
    );
}

#[test]
fn a_failed_container_names_its_exit_code_and_keeps_its_reason() {
    let text_of = |reason, exit_code| {
        let state = ContainerState::Terminated(termination(reason, exit_code));
        container_state_label(&container(ContainerKind::Main, state, false)).text
    };
    assert_eq!(
        text_of(Some(StatusReason::OomKilled), 137),
        "OOMKilled · exit 137"
    );
    // Without a reason the label falls back to Error, still with the code.
    assert_eq!(text_of(None, 2), "Error · exit 2");
    assert_eq!(text_of(Some(StatusReason::Completed), 0), "Completed");
    assert_eq!(text_of(None, 0), "Completed");
}

/// A colour of One Light, read raw because the kit's base colour fields are private.
fn one_light(key: &str) -> Hsla {
    let file: serde_json::Value =
        serde_json::from_str(include_str!("../themes/zed-one.json")).expect("the theme is JSON");
    let theme = file["themes"]
        .as_array()
        .and_then(|themes| themes.iter().find(|theme| theme["name"] == "One Light"))
        .expect("One Light is in the file");
    let raw = theme["colors"][key].as_str().expect("the key is set");
    gpui_kit::component::try_parse_color(raw).expect("the colour parses")
}

#[test]
fn light_theme_warning_text_reads_on_the_light_backgrounds() {
    let (fill, foreground) = (one_light("warning.background"), one_light("foreground"));
    for key in ["background", "sidebar.background", "table.head.background"] {
        let background = one_light(key);
        let text = readable_on_light(fill, foreground, background, LIGHT_THEME_TONE_SHARE);
        let ratio = contrast(text, background);
        assert!(ratio >= TEXT_CONTRAST, "warning on {key}: {ratio}");
    }
}

#[test]
fn default_light_warning_text_reaches_the_text_contrast() {
    // The kit's amber fill only reads at 4.5:1 once it is pulled well toward the foreground.
    let theme = ThemeColor::light();
    let text = readable_on_light(
        theme.warning,
        theme.foreground,
        theme.background,
        LIGHT_THEME_TONE_SHARE,
    );
    assert!(contrast(text, theme.background) >= TEXT_CONTRAST - 0.02);
}
