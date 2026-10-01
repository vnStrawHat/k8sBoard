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
