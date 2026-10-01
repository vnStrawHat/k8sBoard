use cluster::{AccessCheck, NodeSummary, PodSummary};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{ClipboardItem, ParentElement as _, SharedString, Styled as _, div};

use crate::cluster_session::AccessState;

const LOGS_REASON: &str = "Logs open in the dock, coming in a later version";
const READ_ONLY_FEATURE_REASON: &str = "Not available in read-only mode";
const READ_ONLY_MODE_REASON: &str = "Read-only mode";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceAction {
    ViewLogs,
    OpenShell,
    PortForward,
    OpenNodeShell,
    Cordon,
    Drain,
    CopyName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ActionAvailability {
    Enabled,
    Disabled { reason: SharedString },
}

/// What an action needs: the permission it is gated on, and why it is still unavailable
/// when that permission is granted (this phase never runs anything but Copy name).
struct ActionGate {
    check: AccessCheck,
    read_only_reason: &'static str,
}

impl ResourceAction {
    fn gate(self) -> Option<ActionGate> {
        let (check, read_only_reason) = match self {
            Self::ViewLogs => (AccessCheck::GetPodLogs, LOGS_REASON),
            Self::OpenShell => (AccessCheck::CreatePodExec, READ_ONLY_FEATURE_REASON),
            Self::PortForward => (AccessCheck::CreatePodPortForward, READ_ONLY_FEATURE_REASON),
            // The node shell is a debug pod, so it needs the same right as a pod shell.
            Self::OpenNodeShell => (AccessCheck::CreatePodExec, READ_ONLY_FEATURE_REASON),
            Self::Cordon | Self::Drain | Self::CopyName => return None,
        };
        Some(ActionGate {
            check,
            read_only_reason,
        })
    }
}

pub(crate) fn action_availability(
    action: ResourceAction,
    access: &AccessState,
) -> ActionAvailability {
    let Some(gate) = action.gate() else {
        return match action {
            ResourceAction::CopyName => ActionAvailability::Enabled,
            _ => disabled(READ_ONLY_MODE_REASON),
        };
    };
    match access {
        AccessState::Checking { .. } => disabled("Checking permissions…"),
        AccessState::Unknown => disabled("Permissions could not be checked"),
        AccessState::Known(report) if !report.is_allowed(gate.check) => {
            disabled(format!("Not permitted: {}", gate.check))
        }
        AccessState::Known(_) => disabled(gate.read_only_reason),
    }
}

fn disabled(reason: impl Into<SharedString>) -> ActionAvailability {
    ActionAvailability::Disabled {
        reason: reason.into(),
    }
}

/// Shared by the row context menu and the drawer header menu, so both always agree.
pub(crate) fn pod_menu(menu: PopupMenu, pod: &PodSummary, access: &AccessState) -> PopupMenu {
    menu.item(action_item(ResourceAction::ViewLogs, "View logs", access))
        .item(action_item(ResourceAction::OpenShell, "Open shell", access))
        .item(action_item(
            ResourceAction::PortForward,
            "Port-forward",
            access,
        ))
        .separator()
        .item(copy_name_item(&pod.name, access))
}

pub(crate) fn node_menu(menu: PopupMenu, node: &NodeSummary, access: &AccessState) -> PopupMenu {
    menu.item(action_item(
        ResourceAction::OpenNodeShell,
        "Open node shell",
        access,
    ))
    .separator()
    .item(action_item(ResourceAction::Cordon, "Cordon", access))
    .item(action_item(ResourceAction::Drain, "Drain…", access))
    .separator()
    .item(copy_name_item(&node.name, access))
}

/// Disabled items stay visible with their reason, so users learn what exists.
fn action_item(action: ResourceAction, label: &'static str, access: &AccessState) -> PopupMenuItem {
    match action_availability(action, access) {
        ActionAvailability::Enabled => PopupMenuItem::new(label),
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
    }
}

/// A `PopupMenuItem` has no tooltip, so the reason sits under the label in smaller text.
pub(crate) fn disabled_menu_item(label: &'static str, reason: SharedString) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        v_flex().child(div().child(label)).child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(reason.clone()),
        )
    })
    .disabled(true)
}

fn copy_name_item(name: &str, access: &AccessState) -> PopupMenuItem {
    match action_availability(ResourceAction::CopyName, access) {
        ActionAvailability::Disabled { reason } => disabled_menu_item("Copy name", reason),
        ActionAvailability::Enabled => {
            let name = name.to_owned();
            PopupMenuItem::new("Copy name").on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
            })
        }
    }
}

#[cfg(test)]
#[path = "resource_actions_tests.rs"]
mod resource_actions_tests;
