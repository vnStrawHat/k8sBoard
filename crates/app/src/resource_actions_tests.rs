use cluster::{AccessDecision, AccessReport, AccessReview};
use gpui_kit::Task;

use super::*;

fn checking() -> AccessState {
    AccessState::Checking {
        _task: Task::ready(()),
    }
}

fn unknown() -> AccessState {
    AccessState::Unknown
}

/// A report that allows every check except those listed.
fn known_denying(denied: &[AccessCheck]) -> AccessState {
    let reviews = AccessCheck::ALL
        .into_iter()
        .map(|check| AccessReview {
            check,
            decision: if denied.contains(&check) {
                AccessDecision::Denied { reason: None }
            } else {
                AccessDecision::Allowed
            },
        })
        .collect();
    AccessState::Known(AccessReport { reviews })
}

fn reason(availability: ActionAvailability) -> String {
    match availability {
        ActionAvailability::Disabled { reason } => reason.to_string(),
        ActionAvailability::Enabled => panic!("expected a disabled action"),
    }
}

#[test]
fn copy_name_always_enabled() {
    for access in [
        checking(),
        unknown(),
        known_denying(&[]),
        known_denying(&AccessCheck::ALL),
    ] {
        assert_eq!(
            action_availability(ResourceAction::CopyName, &access),
            ActionAvailability::Enabled
        );
    }
}

#[test]
fn cordon_and_drain_always_disabled_read_only_mode() {
    for access in [checking(), unknown(), known_denying(&[])] {
        for action in [ResourceAction::Cordon, ResourceAction::Drain] {
            assert_eq!(
                reason(action_availability(action, &access)),
                "Read-only mode"
            );
        }
    }
}

#[test]
fn gated_actions_disabled_while_checking() {
    for action in [
        ResourceAction::ViewLogs,
        ResourceAction::OpenShell,
        ResourceAction::PortForward,
        ResourceAction::OpenNodeShell,
    ] {
        assert_eq!(
            reason(action_availability(action, &checking())),
            "Checking permissions…"
        );
        assert_eq!(
            reason(action_availability(action, &unknown())),
            "Permissions could not be checked"
        );
    }
}

#[test]
fn shell_denied_reason_names_access_check() {
    let access = known_denying(&[
        AccessCheck::CreatePodExec,
        AccessCheck::CreatePodPortForward,
    ]);
    assert_eq!(
        reason(action_availability(ResourceAction::OpenShell, &access)),
        "Not permitted: create pods/exec"
    );
    assert_eq!(
        reason(action_availability(ResourceAction::PortForward, &access)),
        "Not permitted: create pods/portforward"
    );
}

#[test]
fn shell_allowed_still_disabled_in_read_only_mode() {
    let access = known_denying(&[]);
    assert_eq!(
        reason(action_availability(ResourceAction::OpenShell, &access)),
        "Not available in read-only mode"
    );
    assert_eq!(
        reason(action_availability(ResourceAction::PortForward, &access)),
        "Not available in read-only mode"
    );
}

#[test]
fn node_shell_gated_by_create_pods_exec() {
    assert_eq!(
        reason(action_availability(
            ResourceAction::OpenNodeShell,
            &known_denying(&[AccessCheck::CreatePodExec])
        )),
        "Not permitted: create pods/exec"
    );
    assert_eq!(
        reason(action_availability(
            ResourceAction::OpenNodeShell,
            &known_denying(&[AccessCheck::CreatePodPortForward])
        )),
        "Not available in read-only mode"
    );
}

#[test]
fn logs_allowed_is_enabled() {
    assert_eq!(
        action_availability(ResourceAction::ViewLogs, &known_denying(&[])),
        ActionAvailability::Enabled
    );
}

#[test]
fn logs_denied_reason_names_access_check() {
    assert_eq!(
        reason(action_availability(
            ResourceAction::ViewLogs,
            &known_denying(&[AccessCheck::GetPodLogs])
        )),
        "Not permitted: get pods/log"
    );
}

#[test]
fn forward_button_reason_follows_the_port_forward_gate() {
    assert_eq!(
        port_forward_reason(&known_denying(&[AccessCheck::CreatePodPortForward])),
        "Not permitted: create pods/portforward"
    );
    assert_eq!(
        port_forward_reason(&known_denying(&[])),
        "Not available in read-only mode"
    );
    assert_eq!(port_forward_reason(&checking()), "Checking permissions…");
}

#[test]
fn kubectl_command_quotes_only_unsafe_parts() {
    assert_eq!(
        kubectl_describe_command("readonly@Monitor", "shop", "api-0"),
        "kubectl --context readonly@Monitor -n shop describe pod api-0"
    );
    assert_eq!(
        kubectl_describe_command("my ctx", "shop", "it's"),
        "kubectl --context 'my ctx' -n shop describe pod 'it'\\''s'"
    );
}

fn pod_on(node: Option<&str>) -> PodSummary {
    PodSummary {
        namespace: "ns".to_owned(),
        name: "pod".to_owned(),
        status: cluster::PodStatus::Reason(cluster::StatusReason::Running),
        ready: cluster::ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: node.map(str::to_owned),
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        containers: Vec::new(),
        status_message: None,
        labels: Vec::new(),
    }
}

#[test]
fn view_pods_on_node_counts_pods_on_that_node() {
    let pods = [
        pod_on(Some("wk-01")),
        pod_on(Some("wk-02")),
        pod_on(Some("wk-01")),
        pod_on(None),
    ];
    assert_eq!(pods_on_node(&pods, "wk-01"), 2);
    assert_eq!(pods_on_node(&pods, "wk-02"), 1);
    // An empty node still has the item, with a zero.
    assert_eq!(pods_on_node(&pods, "wk-09"), 0);
}

fn event_with_reason(reason: Option<&str>) -> EventDetail {
    EventDetail {
        title: "t".into(),
        reason: reason.map(SharedString::from),
        object: None,
        source: None,
        message: "m".into(),
    }
}

#[test]
fn filter_similar_disabled_without_reason() {
    assert_eq!(similar_reason(&event_with_reason(None)), None);
    assert_eq!(similar_reason(&event_with_reason(Some(""))), None);
    assert_eq!(
        similar_reason(&event_with_reason(Some("BackOff"))).map(SharedString::as_ref),
        Some("BackOff")
    );
}
