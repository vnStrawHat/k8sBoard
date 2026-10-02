use cluster::{AccessCheck, NodeSummary, PodSummary};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{ClipboardItem, ParentElement as _, SharedString, Styled as _, WeakEntity, div};

use crate::app_shell::AppShell;
use crate::cluster_session::{AccessState, LiveCluster};
use crate::kind_row::{EventDetail, KindObject, KindRow};
use crate::log_dock::LogDock;
use crate::log_tab::LogTarget;
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

const READ_ONLY_FEATURE_REASON: &str = "Not available in read-only mode";
pub(crate) const READ_ONLY_MODE_REASON: &str = "Read-only mode";

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
    context: &str,
    dock: &WeakEntity<LogDock>,
    shell: &WeakEntity<AppShell>,
) -> PopupMenu {
    let access = &live.access;
    menu.item(view_logs_item(pod, live, dock))
        .item(action_item(ResourceAction::OpenShell, "Open shell", access))
        .item(action_item(
            ResourceAction::PortForward,
            "Port-forward",
            access,
        ))
        .item(view_yaml_item(ResourceKey::of_pod(pod), shell))
        .separator()
        .item(copy_name_item(&pod.name, access))
        .item(copy_kubectl_command_item(context, pod))
}

/// Copies the read-only `kubectl describe` command for the pod.
fn copy_kubectl_command_item(context: &str, pod: &PodSummary) -> PopupMenuItem {
    let command = kubectl_describe_command(context, &pod.namespace, &pod.name);
    PopupMenuItem::new("Copy kubectl command").on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
    })
}

/// `kubectl --context C -n NS describe pod NAME`. It has no `--kubeconfig`: a path is specific
/// to this machine.
pub(crate) fn kubectl_describe_command(context: &str, namespace: &str, name: &str) -> String {
    format!(
        "kubectl --context {} -n {} describe pod {}",
        shell_quote(context),
        shell_quote(namespace),
        shell_quote(name)
    )
}

/// Single-quotes a part that has a character outside the shell-safe set.
// ponytail: POSIX sh quoting only; PowerShell and cmd need other rules — add a per-shell variant if Windows users paste into them.
fn shell_quote(text: &str) -> String {
    let is_safe = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./_-".contains(c));
    if is_safe {
        return text.to_owned();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
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

pub(crate) fn node_menu(
    menu: PopupMenu,
    node: &NodeSummary,
    live: &LiveCluster,
    shell: &WeakEntity<AppShell>,
) -> PopupMenu {
    let access = &live.access;
    menu.item(action_item(
        ResourceAction::OpenNodeShell,
        "Open node shell",
        access,
    ))
    .item(view_yaml_item(ResourceKey::of_node(node), shell))
    .item(view_pods_on_node_item(node, live.pods.items(), shell))
    .separator()
    .item(action_item(ResourceAction::Cordon, "Cordon", access))
    .item(action_item(ResourceAction::Drain, "Drain…", access))
    .separator()
    .item(copy_name_item(&node.name, access))
}

/// Switches to Pods with only the pods of the node. Always enabled, even for an empty node.
fn view_pods_on_node_item(
    node: &NodeSummary,
    pods: &[PodSummary],
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let name = node.name.clone();
    let shell = shell.clone();
    PopupMenuItem::new(format!(
        "View pods on node · {}",
        pods_on_node(pods, &node.name)
    ))
    .on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.view_pods_on_node(&name, cx));
    })
}

/// How many of `pods` are scheduled on `node`.
fn pods_on_node(pods: &[PodSummary], node: &str) -> usize {
    pods.iter()
        .filter(|pod| pod.node_name.as_deref() == Some(node))
        .count()
}

/// The row context menu and the drawer ⋯ menu of an explorer kind. Every item except View YAML and
/// Copy name (and, for events, Go to object and Copy message) is disabled, because this version
/// is read-only.
pub(crate) fn kind_menu(
    menu: PopupMenu,
    kind: ResourceKind,
    row: &KindRow,
    access: &AccessState,
    shell: &WeakEntity<AppShell>,
) -> PopupMenu {
    let mut menu = menu;
    if let Some(event) = &row.event {
        menu = menu
            .item(go_to_object_item(event, shell))
            .item(filter_similar_item(event, shell))
            .item(copy_message_item(event))
            .separator();
    }
    menu = menu.item(view_yaml_item(ResourceKey::of_row(kind, row), shell));
    if has_go_to_owner(kind) {
        menu = menu.item(go_to_owner_item(row, shell));
    }
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

/// Only ReplicaSets offer Go to owner: a Job's owner is a link in its drawer.
fn has_go_to_owner(kind: ResourceKind) -> bool {
    kind == ResourceKind::ReplicaSets
}

/// The key of the controller that owns a ReplicaSet row; `None` without an owner or when
/// k8sBoard has no screen for its kind.
fn owner_target(row: &KindRow) -> Option<ResourceKey> {
    let KindObject::ReplicaSet(set) = &row.object else {
        return None;
    };
    ResourceKey::of_owner(&set.namespace, set.owner.as_ref()?)
}

/// Reveals the owner (a Deployment, usually); disabled when there is none.
fn go_to_owner_item(row: &KindRow, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    const LABEL: &str = "Go to owner";
    let Some(key) = owner_target(row) else {
        return disabled_menu_item(LABEL, "No owner".into());
    };
    let shell = shell.clone();
    PopupMenuItem::new(LABEL).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.reveal(key.clone(), cx));
    })
}

/// Reveals the involved object on its own screen; disabled when there is none.
fn go_to_object_item(event: &EventDetail, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    const LABEL: &str = "Go to object";
    let Some(key) = event.object.clone() else {
        return disabled_menu_item(LABEL, "No screen for this kind yet".into());
    };
    let shell = shell.clone();
    PopupMenuItem::new(LABEL).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.reveal(key.clone(), cx));
    })
}

/// Filters the Events list to the reason of this event; disabled when it has none.
fn filter_similar_item(event: &EventDetail, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    const LABEL: &str = "Filter similar";
    let Some(reason) = similar_reason(event) else {
        return disabled_menu_item(LABEL, "This event has no reason".into());
    };
    let reason = reason.clone();
    let shell = shell.clone();
    PopupMenuItem::new(LABEL).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.filter_similar(&reason, cx));
    })
}

/// What Filter similar matches on: the reason, when the event has one.
fn similar_reason(event: &EventDetail) -> Option<&SharedString> {
    event.reason.as_ref().filter(|reason| !reason.is_empty())
}

fn copy_message_item(event: &EventDetail) -> PopupMenuItem {
    let message = event.message.clone();
    PopupMenuItem::new("Copy message").on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(message.to_string()));
    })
}

/// Opens the drawer of `key` on its YAML tab. Always enabled: a missing right shows inline there.
fn view_yaml_item(key: ResourceKey, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new("View YAML").on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.open_yaml(key.clone(), cx));
    })
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
