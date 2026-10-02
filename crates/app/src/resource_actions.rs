use cluster::{AccessCheck, NodeSummary, PodSummary, SecretKey};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    App, ClipboardItem, ParentElement as _, SharedString, Styled as _, WeakEntity, Window, div,
};

use crate::access_bindings::role_key;
use crate::app_shell::{AppShell, Screen};
use crate::cluster_session::{AccessState, LiveCluster};
use crate::custom_kind::CustomKind;
use crate::drawer::DrawerTab;
use crate::kind_row::{EventDetail, JOB_KIND, KindObject, KindRow, PodOwner};
use crate::live_sections::claim_pods;
use crate::log_dock::LogDock;
use crate::log_target::{LogTarget, workload_label};
use crate::network_rows::ingress_urls;
use crate::resource_kind::ResourceKind;
use crate::secret_values::{SecretAction, ValueAccess};
use crate::table_selection::ResourceKey;
use crate::yaml_view::object_ref;

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

/// The label and availability of the logs item of a workload row; `None` for a row that has
/// no workload (a ConfigMap, a node).
fn workload_logs_entry(
    related_pods: Option<&PodOwner>,
    access: &AccessState,
) -> Option<(&'static str, ActionAvailability)> {
    let owner = related_pods?;
    workload_label(owner)?;
    let is_job = matches!(owner, PodOwner::Controller { kind, .. } if *kind == JOB_KIND);
    let label = if is_job {
        "View logs"
    } else {
        "View logs (all pods)"
    };
    Some((label, action_availability(ResourceAction::ViewLogs, access)))
}

/// Merges the logs of every pod of the workload into one dock tab.
fn workload_logs_item(
    row: &KindRow,
    access: &AccessState,
    shell: &WeakEntity<AppShell>,
) -> Option<PopupMenuItem> {
    let (label, availability) = workload_logs_entry(row.related_pods.as_ref(), access)?;
    Some(match availability {
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
        ActionAvailability::Enabled => {
            let owner = row.related_pods.clone()?;
            let shell = shell.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.open_workload_logs(owner.clone(), window, cx);
                });
            })
        }
    })
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

/// The items a release adds before its disabled ones: each opens the drawer on a Helm tab.
const HELM_VIEW_ITEMS: [(&str, DrawerTab); 2] = [
    ("View values", DrawerTab::Values),
    ("View manifest", DrawerTab::Manifest),
];

/// The row context menu and the drawer ⋯ menu of an explorer kind. Every item except View YAML and
/// Copy name (and, for events, Go to object and Copy message) is disabled, because this version
/// is read-only.
pub(crate) fn kind_menu(
    menu: PopupMenu,
    kind: ResourceKind,
    row: &KindRow,
    access: &AccessState,
    pods: &[PodSummary],
    shell: &WeakEntity<AppShell>,
    extras: MenuExtras,
) -> PopupMenu {
    let mut menu = menu;
    if let Some(item) = workload_logs_item(row, access, shell) {
        menu = menu.item(item);
    }
    if let Some(secret) = extras.secret {
        menu = menu.item(secret.reveal).item(secret.copy).separator();
    }
    if let Some(event) = &row.event {
        menu = menu
            .item(go_to_object_item(event, shell))
            .item(filter_similar_item(event, shell))
            .item(copy_message_item(event))
            .separator();
    }
    if let Some(browse) = extras.browse {
        menu = menu.item(browse);
    }
    let key = ResourceKey::of_row(kind, row);
    // A key without an object reference (a Helm release) has no YAML tab.
    if kind == ResourceKind::HelmReleases {
        for (label, tab) in HELM_VIEW_ITEMS {
            menu = menu.item(view_tab_item(label, key.clone(), tab, shell));
        }
    }
    if object_ref(&key).is_some() {
        menu = menu.item(view_yaml_item(key, shell));
    }
    if let Some(item) = extras.open_url {
        menu = menu.item(item);
    }
    if has_go_to_target(kind) {
        menu = menu.item(go_to_target_item(row, shell));
    }
    if has_go_to_owner(kind) {
        menu = menu.item(go_to_owner_item(row, shell));
    }
    match kind {
        ResourceKind::PersistentVolumeClaims => {
            menu = menu.item(go_to_pod_item(row, pods, shell));
        }
        ResourceKind::PersistentVolumes => menu = menu.item(go_to_claim_item(row, shell)),
        ResourceKind::RoleBindings | ResourceKind::ClusterRoleBindings => {
            menu = menu.item(go_to_role_item(row, shell));
        }
        _ => {}
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
        menu = menu.item(disabled_menu_item(*label, READ_ONLY_MODE_REASON.into()));
    }
    menu.separator()
        .item(copy_name_item(&row.name, access))
        .separator()
        .item(disabled_menu_item(
            kind.delete_label(),
            READ_ONLY_MODE_REASON.into(),
        ))
}

/// The custom kind a CRD row opens: the served kind with its name, `None` while the CRD is not
/// Established or has no served version.
pub(crate) fn browse_target(crd_name: &str, kinds: &[CustomKind]) -> Option<CustomKind> {
    kinds
        .iter()
        .find(|kind| kind.crd_name() == crd_name)
        .copied()
}

/// Browse instances of a CRD row: opens the custom kind, or says why it cannot.
pub(crate) fn browse_instances_item(
    row: &KindRow,
    kinds: &[CustomKind],
    shell: &WeakEntity<AppShell>,
) -> Option<PopupMenuItem> {
    const LABEL: &str = "Browse instances";
    let KindObject::Crd(crd) = &row.object else {
        return None;
    };
    let Some(kind) = browse_target(&crd.name, kinds) else {
        return Some(disabled_menu_item(
            LABEL,
            "Not established or not served".into(),
        ));
    };
    let shell = shell.clone();
    Some(PopupMenuItem::new(LABEL).on_click(move |_, _, cx| {
        let screen = Screen::Kind(ResourceKind::Custom(kind));
        let _ = shell.update(cx, |shell, cx| shell.show_screen(screen, cx));
    }))
}

/// Items that need the window or the app to be built, so the caller builds them before it borrows
/// the session (a submenu needs the app mutably).
#[derive(Default)]
pub(crate) struct MenuExtras {
    pub(crate) open_url: Option<PopupMenuItem>,
    pub(crate) secret: Option<SecretMenu>,
    /// A CRD row's Browse instances item.
    pub(crate) browse: Option<PopupMenuItem>,
}

/// The Reveal and Copy items of a Secret.
pub(crate) struct SecretMenu {
    reveal: PopupMenuItem,
    copy: PopupMenuItem,
}

/// Why an item is disabled, or that it is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuState {
    Enabled,
    Disabled(&'static str),
}

/// One entry of the Copy submenu. `key` is `None` for the "No data" placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CopyEntry {
    pub(crate) label: String,
    pub(crate) key: Option<String>,
    pub(crate) state: MenuState,
}

/// What a Secret's menu offers, decided from the keys and the one access field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SecretMenuModel {
    pub(crate) reveal: MenuState,
    pub(crate) copies: Vec<CopyEntry>,
}

const SECRET_BLOCKED_REASON: &str = "Disabled in screenshot runs";

/// Reveal values (30s), then Copy value with one `Copy {key}` per key. Blocked access disables
/// everything; a binary key cannot be copied; no keys leaves one disabled "No data".
pub(crate) fn secret_menu_model(keys: &[SecretKey], access: ValueAccess) -> SecretMenuModel {
    let is_blocked = access == ValueAccess::Blocked;
    let blocked = MenuState::Disabled(SECRET_BLOCKED_REASON);
    if keys.is_empty() {
        return SecretMenuModel {
            reveal: if is_blocked {
                blocked
            } else {
                MenuState::Disabled("No data")
            },
            copies: vec![CopyEntry {
                label: "No data".to_owned(),
                key: None,
                state: MenuState::Disabled(if is_blocked {
                    SECRET_BLOCKED_REASON
                } else {
                    "No data"
                }),
            }],
        };
    }
    SecretMenuModel {
        reveal: if is_blocked {
            blocked
        } else {
            MenuState::Enabled
        },
        copies: keys
            .iter()
            .map(|key| CopyEntry {
                label: format!("Copy {}", key.name),
                key: Some(key.name.clone()),
                state: match (is_blocked, key.is_binary) {
                    (true, _) => blocked,
                    (false, true) => MenuState::Disabled("Binary value"),
                    (false, false) => MenuState::Enabled,
                },
            })
            .collect(),
    }
}

/// The Reveal and Copy items of a Secrets row; `None` for any other row.
pub(crate) fn secret_menu(
    row: &KindRow,
    key: ResourceKey,
    access: ValueAccess,
    shell: &WeakEntity<AppShell>,
    window: &mut Window,
    cx: &mut App,
) -> Option<SecretMenu> {
    let KindObject::Secret(secret) = &row.object else {
        return None;
    };
    let model = secret_menu_model(&secret.keys, access);
    let reveal = match model.reveal {
        MenuState::Enabled => secret_action_item(
            "Reveal values (30s)".into(),
            key.clone(),
            SecretAction::RevealAll,
            shell,
        ),
        MenuState::Disabled(reason) => disabled_menu_item("Reveal values (30s)", reason.into()),
    };
    let shell = shell.clone();
    let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
        model.copies.iter().fold(submenu, |submenu, entry| {
            let item = match (&entry.state, &entry.key) {
                (MenuState::Enabled, Some(name)) => secret_action_item(
                    entry.label.clone().into(),
                    key.clone(),
                    SecretAction::Copy(name.clone()),
                    &shell,
                ),
                (MenuState::Disabled(reason), _) => {
                    disabled_menu_item(entry.label.clone(), (*reason).into())
                }
                (MenuState::Enabled, None) => {
                    disabled_menu_item(entry.label.clone(), "No data".into())
                }
            };
            submenu.item(item)
        })
    });
    Some(SecretMenu {
        reveal,
        copy: PopupMenuItem::submenu("Copy value", submenu),
    })
}

/// A Reveal or Copy item: it opens the drawer of `key` and runs `action` through the shell.
fn secret_action_item(
    label: SharedString,
    key: ResourceKey,
    action: SecretAction,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| {
            shell.run_secret_action(key.clone(), action.clone(), cx);
        });
    })
}

/// How many URLs the Open URL submenu lists.
const MAX_OPEN_URLS: usize = 10;

/// What the Open URL item of an Ingress offers.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OpenUrl {
    /// No rule has a host to open.
    Unavailable,
    One(String),
    Several(Vec<String>),
}

pub(crate) fn open_url_choice(row: &KindRow) -> OpenUrl {
    let KindObject::Ingress(ingress) = &row.object else {
        return OpenUrl::Unavailable;
    };
    let mut urls = ingress_urls(ingress);
    match urls.len() {
        0 => OpenUrl::Unavailable,
        1 => OpenUrl::One(urls.swap_remove(0)),
        _ => {
            urls.truncate(MAX_OPEN_URLS);
            OpenUrl::Several(urls)
        }
    }
}

/// The Open URL item after View YAML: the default browser, through the platform's open-URL call.
/// Nothing is logged; the URLs hold only plain hosts and paths (`ingress_urls`). The caller builds
/// it before it borrows the session, because a submenu needs the app mutably.
pub(crate) fn open_url_menu_item(
    choice: OpenUrl,
    window: &mut Window,
    cx: &mut App,
) -> PopupMenuItem {
    // The kit wires the parent of `PopupMenuItem::submenu` in `PopupMenu::render`, so `build` is the
    // sanctioned path from the table context menu.
    const LABEL: &str = "Open URL";
    match choice {
        OpenUrl::Unavailable => disabled_menu_item(LABEL, "No host to open".into()),
        OpenUrl::One(url) => open_url_item(LABEL, url),
        OpenUrl::Several(urls) => {
            let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
                urls.iter().fold(submenu, |submenu, url| {
                    submenu.item(open_url_item(url.clone(), url.clone()))
                })
            });
            PopupMenuItem::submenu(LABEL, submenu)
        }
    }
}

fn open_url_item(label: impl Into<SharedString>, url: String) -> PopupMenuItem {
    PopupMenuItem::new(label).on_click(move |_, _, cx| cx.open_url(&url))
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

/// A menu item that reveals `target`; disabled with `reason` when there is none.
fn go_to_item(
    label: &'static str,
    target: Option<ResourceKey>,
    reason: SharedString,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let Some(key) = target else {
        return disabled_menu_item(label, reason);
    };
    let shell = shell.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.reveal(key.clone(), cx));
    })
}

/// Reveals the owner (a Deployment, usually); disabled when there is none.
fn go_to_owner_item(row: &KindRow, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    go_to_item("Go to owner", owner_target(row), "No owner".into(), shell)
}

/// Only HPAs offer Go to target.
fn has_go_to_target(kind: ResourceKind) -> bool {
    kind == ResourceKind::HorizontalPodAutoscalers
}

/// The key of the workload an HPA scales; `None` when k8sBoard has no screen for its kind.
fn scale_target(row: &KindRow) -> Option<ResourceKey> {
    let KindObject::HorizontalPodAutoscaler(hpa) = &row.object else {
        return None;
    };
    ResourceKey::of_owner(&hpa.namespace, &hpa.target)
}

/// Why Go to target is disabled: the kind has no screen.
fn no_target_screen_reason(row: &KindRow) -> SharedString {
    match &row.object {
        KindObject::HorizontalPodAutoscaler(hpa) => {
            format!("No screen for {}", hpa.target.kind).into()
        }
        _ => "No target".into(),
    }
}

fn go_to_target_item(row: &KindRow, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    go_to_item(
        "Go to target",
        scale_target(row),
        no_target_screen_reason(row),
        shell,
    )
}

/// The first pod, by name, that mounts the claim of a PVCs row.
fn claim_pod_target(row: &KindRow, pods: &[PodSummary]) -> Option<ResourceKey> {
    let KindObject::PersistentVolumeClaim(claim) = &row.object else {
        return None;
    };
    let (pod, _) = claim_pods(&claim.namespace, &claim.name, pods)
        .into_iter()
        .next()?;
    Some(ResourceKey::of_pod(pod))
}

fn go_to_pod_item(
    row: &KindRow,
    pods: &[PodSummary],
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    go_to_item(
        "Go to pod",
        claim_pod_target(row, pods),
        "Not mounted by any pod".into(),
        shell,
    )
}

/// The claim a PVs row is bound to.
fn volume_claim_target(row: &KindRow) -> Option<ResourceKey> {
    let KindObject::PersistentVolume(volume) = &row.object else {
        return None;
    };
    let claim = volume.claim.as_ref()?;
    ResourceKey::of_object("PersistentVolumeClaim", Some(&claim.namespace), &claim.name)
}

fn go_to_claim_item(row: &KindRow, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    go_to_item(
        "Go to claim",
        volume_claim_target(row),
        "No claim".into(),
        shell,
    )
}

/// The role a bindings row names; `None` for a kind k8sBoard has no screen for.
fn binding_role_target(row: &KindRow) -> Option<ResourceKey> {
    let KindObject::Binding(binding) = &row.object else {
        return None;
    };
    role_key(binding)
}

fn go_to_role_item(row: &KindRow, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    go_to_item(
        "Go to role",
        binding_role_target(row),
        "No screen for this role".into(),
        shell,
    )
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
    view_tab_item("View YAML", key, DrawerTab::Yaml, shell)
}

/// Opens the drawer of `key` on `tab`; the same item serves View YAML, View values, and View
/// manifest.
fn view_tab_item(
    label: &'static str,
    key: ResourceKey,
    tab: DrawerTab,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.open_drawer_tab(key.clone(), tab, cx));
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
pub(crate) fn disabled_menu_item(
    label: impl Into<SharedString>,
    reason: SharedString,
) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |_, cx| {
        v_flex().child(div().child(label.clone())).child(
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
