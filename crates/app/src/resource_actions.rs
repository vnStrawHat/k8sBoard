use cluster::{AccessCheck, NodeSummary, PodSummary};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{ClipboardItem, ParentElement as _, SharedString, Styled as _, WeakEntity, div};

use crate::cluster_session::{AccessState, LiveCluster};
use crate::kind_row::KindRow;
use crate::log_dock::LogDock;
use crate::log_tab::LogTarget;
use crate::resource_kind::ResourceKind;

const READ_ONLY_FEATURE_REASON: &str = "Not available in read-only mode";
const READ_ONLY_MODE_REASON: &str = "Read-only mode";
/// Why "View YAML" is disabled on every kind.
pub(crate) const YAML_DEFERRED_REASON: &str = "YAML view comes in a later version";

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
/// when that permission is granted. `None` means the permission is all the action needs.
struct ActionGate {
    check: AccessCheck,
    read_only_reason: Option<&'static str>,
}

impl ResourceAction {
    fn gate(self) -> Option<ActionGate> {
        let (check, read_only_reason) = match self {
            Self::ViewLogs => (AccessCheck::GetPodLogs, None),
            Self::OpenShell => (AccessCheck::CreatePodExec, Some(READ_ONLY_FEATURE_REASON)),
            Self::PortForward => (
                AccessCheck::CreatePodPortForward,
                Some(READ_ONLY_FEATURE_REASON),
            ),
            // The node shell is a debug pod, so it needs the same right as a pod shell.
            Self::OpenNodeShell => (AccessCheck::CreatePodExec, Some(READ_ONLY_FEATURE_REASON)),
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
        AccessState::Known(_) => match gate.read_only_reason {
            Some(reason) => disabled(reason),
            None => ActionAvailability::Enabled,
        },
    }
}

fn disabled(reason: impl Into<SharedString>) -> ActionAvailability {
    ActionAvailability::Disabled {
        reason: reason.into(),
    }
}

/// Shared by the row context menu and the drawer header menu, so both always agree.
pub(crate) fn pod_menu(
    menu: PopupMenu,
    pod: &PodSummary,
    live: &LiveCluster,
    dock: &WeakEntity<LogDock>,
) -> PopupMenu {
    let access = &live.access;
    menu.item(view_logs_item(pod, live, dock))
        .item(action_item(ResourceAction::OpenShell, "Open shell", access))
        .item(action_item(
            ResourceAction::PortForward,
            "Port-forward",
            access,
        ))
        .separator()
        .item(copy_name_item(&pod.name, access))
}

/// Opens the pod in the log dock. Without containers there is nothing to read.
fn view_logs_item(
    pod: &PodSummary,
    live: &LiveCluster,
    dock: &WeakEntity<LogDock>,
) -> PopupMenuItem {
    const LABEL: &str = "View logs";
    match action_availability(ResourceAction::ViewLogs, &live.access) {
        ActionAvailability::Disabled { reason } => disabled_menu_item(LABEL, reason),
        ActionAvailability::Enabled => match LogTarget::of_pod(pod) {
            None => disabled_menu_item(LABEL, "The pod has no containers".into()),
            Some(target) => {
                let connection = live.connection().clone();
                let dock = dock.clone();
                PopupMenuItem::new(LABEL).on_click(move |_, window, cx| {
                    let _ = dock.update(cx, |dock, cx| {
                        dock.open(connection.clone(), target.clone(), window, cx)
                    });
                })
            }
        },
    }
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

/// The row context menu and the drawer ⋯ menu of an explorer kind. Every item except Copy name
/// is disabled, because this version is read-only and has no YAML view.
pub(crate) fn kind_menu(
    menu: PopupMenu,
    kind: ResourceKind,
    row: &KindRow,
    access: &AccessState,
) -> PopupMenu {
    let mut menu = menu.item(disabled_menu_item("View YAML", YAML_DEFERRED_REASON.into()));
    if kind.has_port_forward() {
        menu = menu.item(action_item(
            ResourceAction::PortForward,
            "Port-forward",
            access,
        ));
    }
    let change_actions = kind.read_only_actions();
    if !change_actions.is_empty() {
        menu = menu.separator();
    }
    for label in change_actions {
        menu = menu.item(disabled_menu_item(label, READ_ONLY_MODE_REASON.into()));
    }
    menu.separator()
        .item(copy_name_item(&row.name, access))
        .separator()
        .item(disabled_menu_item(
            kind.delete_label(),
            READ_ONLY_MODE_REASON.into(),
        ))
}

/// The tooltip of a disabled Forward button. Port-forward is never enabled in this version.
pub(crate) fn port_forward_reason(access: &AccessState) -> SharedString {
    match action_availability(ResourceAction::PortForward, access) {
        ActionAvailability::Disabled { reason } => reason,
        ActionAvailability::Enabled => READ_ONLY_FEATURE_REASON.into(),
    }
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
