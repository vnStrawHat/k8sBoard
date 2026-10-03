use cluster::{
    AccessCheck, ContainerKind, ContainerState, ContainerSummary, NamespaceScope, NodeSummary,
    PodSummary, SecretKey,
};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    Action, App, ClipboardItem, ParentElement as _, SharedString, Styled as _, WeakEntity, Window,
    div,
};

use crate::access_bindings::role_key;
use crate::access_query::who_can_prefill;
use crate::app_shell::shell_open::ShellOpen;
use crate::app_shell::write_flow::cordon_label;
use crate::app_shell::{AppShell, Screen};
use crate::cluster_registry::ClusterRef;
use crate::cluster_rows::RowContext;
use crate::cluster_session::{AccessState, LiveCluster, scope_includes};
use crate::custom_kind::CustomKind;
use crate::dock::{Dock, LogOrigin};
use crate::drawer::DrawerTab;
use crate::keymap::{
    CopyName, Cordon, Delete, Drain, EditYaml, OpenShell, PortForward, RestartRollout, Scale,
    ViewLogs, ViewYaml,
};
use crate::kind_row::{EventDetail, JOB_KIND, KindObject, KindRow, PodOwner};
use crate::live_sections::claim_pods;
use crate::log_target::{LogTarget, workload_label};
use crate::network_rows::ingress_urls;
use crate::pod_drawer::kind_tag_text;
use crate::resource_kind::ResourceKind;
use crate::secret_values::{SecretAction, ValueAccess};
use crate::shell_tab::short_pod_name;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::write_guard::{ActionRisk, ClusterGuard, WriteLock};
use crate::yaml_view::object_ref;

/// Why a mutating action is off while its spec has not shipped.
pub(crate) const NOT_SHIPPED_REASON: &str = "Comes in a later version";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceAction {
    ViewLogs,
    OpenShell,
    PortForward,
    OpenNodeShell,
    Cordon,
    Drain,
    CopyName,
    ViewYaml,
    EditYaml,
    Delete,
    RestartRollout,
    Scale,
}

/// A row action as a key, a menu hint, or the palette names it, before the subject is known:
/// the one name a key has, whichever `ResourceAction` the subject resolves it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowAction {
    ViewLogs,
    ViewYaml,
    CopyName,
    OpenShell,
    PortForward,
    Cordon,
    Drain,
    EditYaml,
    RestartRollout,
    Scale,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ActionAvailability {
    Enabled,
    Disabled { reason: SharedString },
}

/// What a row key does on the cursor row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeyAvailability {
    /// The key runs the resolved action of the subject.
    Run(ResourceAction),
    /// The key is offered but unavailable; the shell shows the reason.
    Disabled { reason: SharedString },
    /// The subject has no such action (L on a Service): the key does nothing, silently.
    NotOffered,
}

/// What an action needs before the gate lets it run. A mutating action carries a permission
/// check, so it cannot skip RBAC; one whose check is not defined yet is `Planned` and is never
/// enabled.
enum ActionGate {
    /// Gated by permissions alone: the read-only lock does not apply. `None` needs no permission.
    ReadOnly { check: Option<AccessCheck> },
    /// Changes the cluster or opens a session on it: its spec must have shipped, its permission
    /// must be granted, and the cluster must be unlocked.
    Mutating {
        /// Every permission the action needs; the first one that is not allowed is the reason.
        checks: Vec<AccessCheck>,
        is_shipped: bool,
    },
    /// A mutating action whose spec, and so whose permission check, has not been written yet.
    Planned,
}

impl ResourceAction {
    fn gate(self) -> ActionGate {
        match self {
            Self::ViewLogs => ActionGate::ReadOnly {
                check: Some(AccessCheck::GetPodLogs),
            },
            Self::CopyName | Self::ViewYaml => ActionGate::ReadOnly { check: None },
            // A WebSocket exec is authorized as `get` before Kubernetes 1.35 and as `create` too from
            // 1.35 (spec 0036), so a shell needs both.
            Self::OpenShell => ActionGate::Mutating {
                checks: vec![AccessCheck::GetPodExec, AccessCheck::CreatePodExec],
                is_shipped: true,
            },
            Self::PortForward => ActionGate::Mutating {
                checks: vec![AccessCheck::CreatePodPortForward],
                is_shipped: false,
            },
            // The node shell is a debug pod, so it needs the same right as a pod shell.
            Self::OpenNodeShell => ActionGate::Mutating {
                checks: vec![AccessCheck::CreatePodExec],
                is_shipped: false,
            },
            // Spec 0030: the first shipped mutating action.
            Self::Cordon => ActionGate::Mutating {
                checks: vec![AccessCheck::PatchNodes],
                is_shipped: true,
            },
            Self::Drain | Self::EditYaml | Self::Delete | Self::RestartRollout | Self::Scale => {
                ActionGate::Planned
            }
        }
    }

    /// The kind-less name the key layer, menus, and palette know the action by. The node shell
    /// shares the key of the pod shell.
    pub(crate) fn row_action(self) -> RowAction {
        match self {
            Self::ViewLogs => RowAction::ViewLogs,
            Self::OpenShell | Self::OpenNodeShell => RowAction::OpenShell,
            Self::PortForward => RowAction::PortForward,
            Self::Cordon => RowAction::Cordon,
            Self::Drain => RowAction::Drain,
            Self::CopyName => RowAction::CopyName,
            Self::ViewYaml => RowAction::ViewYaml,
            Self::EditYaml => RowAction::EditYaml,
            Self::Delete => RowAction::Delete,
            Self::RestartRollout => RowAction::RestartRollout,
            Self::Scale => RowAction::Scale,
        }
    }
}

impl RowAction {
    /// The key action behind the row action, which menus show as a hint and the palette
    /// dispatches.
    pub(crate) fn key_action(self) -> Box<dyn Action> {
        match self {
            Self::ViewLogs => Box::new(ViewLogs),
            Self::OpenShell => Box::new(OpenShell),
            Self::PortForward => Box::new(PortForward),
            Self::Cordon => Box::new(Cordon),
            Self::Drain => Box::new(Drain),
            Self::CopyName => Box::new(CopyName),
            Self::ViewYaml => Box::new(ViewYaml),
            Self::EditYaml => Box::new(EditYaml),
            Self::Delete => Box::new(Delete),
            Self::RestartRollout => Box::new(RestartRollout),
            Self::Scale => Box::new(Scale),
        }
    }
}

/// What an action can do to the cluster; the confirm dialog's button style follows it.
pub(crate) fn action_risk(action: ResourceAction) -> ActionRisk {
    match action {
        ResourceAction::Delete | ResourceAction::Drain => ActionRisk::Destructive,
        ResourceAction::ViewLogs
        | ResourceAction::OpenShell
        | ResourceAction::PortForward
        | ResourceAction::OpenNodeShell
        | ResourceAction::Cordon
        | ResourceAction::CopyName
        | ResourceAction::ViewYaml
        | ResourceAction::EditYaml
        | ResourceAction::RestartRollout
        | ResourceAction::Scale => ActionRisk::Change,
    }
}

/// The menu and notice text of an action; one source for both.
pub(crate) fn action_label(action: ResourceAction) -> &'static str {
    match action {
        ResourceAction::ViewLogs => "View logs",
        ResourceAction::OpenShell => "Open shell",
        ResourceAction::PortForward => "Port-forward",
        ResourceAction::OpenNodeShell => "Open node shell",
        ResourceAction::Cordon => "Cordon",
        ResourceAction::Drain => "Drain",
        ResourceAction::CopyName => "Copy name",
        ResourceAction::ViewYaml => "View YAML",
        ResourceAction::EditYaml => "Edit YAML",
        ResourceAction::Delete => "Delete",
        ResourceAction::RestartRollout => "Restart rollout",
        ResourceAction::Scale => "Scale",
    }
}

/// The notice of an offered key that is unavailable, such as "Edit YAML is unavailable: Read-only
/// mode".
pub(crate) fn unavailable_text(label: &str, reason: &str) -> String {
    format!("{label} is unavailable: {reason}")
}

/// The action the key `row` runs on `subject`: S opens the node shell on a node. `None` means the
/// subject does not offer it (L on a Service).
pub(crate) fn subject_action(row: RowAction, subject: &ResourceKey) -> Option<ResourceAction> {
    match row {
        RowAction::ViewLogs => {
            matches!(subject, ResourceKey::Pod { .. }).then_some(ResourceAction::ViewLogs)
        }
        RowAction::OpenShell => match subject {
            ResourceKey::Pod { .. } => Some(ResourceAction::OpenShell),
            ResourceKey::Node { .. } => Some(ResourceAction::OpenNodeShell),
            ResourceKey::Kind { .. } => None,
        },
        RowAction::PortForward => match subject {
            ResourceKey::Pod { .. } => true,
            ResourceKey::Node { .. } => false,
            ResourceKey::Kind { kind, .. } => kind.has_port_forward(),
        }
        .then_some(ResourceAction::PortForward),
        RowAction::Cordon => {
            matches!(subject, ResourceKey::Node { .. }).then_some(ResourceAction::Cordon)
        }
        RowAction::Drain => {
            matches!(subject, ResourceKey::Node { .. }).then_some(ResourceAction::Drain)
        }
        RowAction::ViewYaml => object_ref(subject)
            .is_some()
            .then_some(ResourceAction::ViewYaml),
        RowAction::EditYaml => object_ref(subject)
            .is_some()
            .then_some(ResourceAction::EditYaml),
        // The kind table is the one source of which workload kinds offer the action.
        RowAction::RestartRollout | RowAction::Scale => match subject {
            ResourceKey::Kind { kind, .. } => kind
                .read_only_actions()
                .iter()
                .find_map(|item| item.action.filter(|action| action.row_action() == row)),
            ResourceKey::Pod { .. } | ResourceKey::Node { .. } => None,
        },
        RowAction::CopyName => Some(ResourceAction::CopyName),
        RowAction::Delete => Some(ResourceAction::Delete),
    }
}

/// What a key for `row` does on `subject`; menus and keys read the same gates.
pub(crate) fn key_availability(
    row: RowAction,
    subject: &ResourceKey,
    live: &LiveCluster,
    guard: &ClusterGuard<'_>,
) -> KeyAvailability {
    let pod = live.pods.items().iter().find(|pod| subject.is_pod(pod));
    key_availability_of(row, subject, pod, guard)
}

/// `key_availability` with what it reads from the live cluster passed in: the pod of a pod subject
/// (`None` when its row is gone). `guard` is the guard of the subject's own cluster.
pub(crate) fn key_availability_of(
    row: RowAction,
    subject: &ResourceKey,
    pod: Option<&PodSummary>,
    guard: &ClusterGuard<'_>,
) -> KeyAvailability {
    let Some(action) = subject_action(row, subject) else {
        return KeyAvailability::NotOffered;
    };
    if action == ResourceAction::ViewLogs {
        let Some(pod) = pod else {
            return KeyAvailability::NotOffered;
        };
        return match logs_launch(pod, None, guard.access) {
            Ok(_) => KeyAvailability::Run(action),
            Err(reason) => KeyAvailability::Disabled { reason },
        };
    }
    match action_availability(action, guard) {
        ActionAvailability::Enabled => KeyAvailability::Run(action),
        ActionAvailability::Disabled { reason } => KeyAvailability::Disabled { reason },
    }
}

/// The gate, in order: a mutating action whose spec has not shipped, then the permission state,
/// then the denied permission, then the cluster's read-only lock. A read-only action skips the
/// first and the last, so it needs no guard: see `availability_before_lock`. The first failing reason
/// is the one shown. Menus, keys, and the palette all read this function.
pub(crate) fn action_availability(
    action: ResourceAction,
    guard: &ClusterGuard<'_>,
) -> ActionAvailability {
    gate_availability(&action.gate(), guard)
}

fn gate_availability(gate: &ActionGate, guard: &ClusterGuard<'_>) -> ActionAvailability {
    if let Some(reason) = before_lock_reason(gate, guard.access) {
        return disabled(reason);
    }
    if matches!(gate, ActionGate::Mutating { .. }) && guard.lock == WriteLock::Locked {
        return disabled(format!("{} is read-only", guard.display_name()));
    }
    ActionAvailability::Enabled
}

/// Everything of the gate but the lock. For an action that does not mutate (View logs, View
/// YAML, Copy name) that is the whole gate, so they need no guard. The Forward and shell tooltips
/// ask it too: they only explain why a feature that has not shipped is off.
fn availability_before_lock(action: ResourceAction, access: &AccessState) -> ActionAvailability {
    match before_lock_reason(&action.gate(), access) {
        Some(reason) => disabled(reason),
        None => ActionAvailability::Enabled,
    }
}

/// Rows 2-4 of the gate: why the action is off before the lock is even asked.
fn before_lock_reason(gate: &ActionGate, access: &AccessState) -> Option<SharedString> {
    match gate {
        ActionGate::Planned
        | ActionGate::Mutating {
            is_shipped: false, ..
        } => Some(NOT_SHIPPED_REASON.into()),
        // An action with no check is never held back by the permission state.
        ActionGate::ReadOnly { check } => {
            check.and_then(|check| permission_reason(&[check], access))
        }
        ActionGate::Mutating { checks, .. } => permission_reason(checks, access),
    }
}

/// Why the permissions do not allow the action: the state while they are not known, else the
/// first check of `checks` that is not allowed. A denied verb with its sibling verb on the same
/// resource in the list names both (`get and create pods/exec`), because the user needs both.
fn permission_reason(checks: &[AccessCheck], access: &AccessState) -> Option<SharedString> {
    match access {
        AccessState::Checking { .. } => Some("Checking permissions…".into()),
        AccessState::Unknown => Some("Permissions could not be checked".into()),
        AccessState::Known(report) => {
            let denied = checks.iter().find(|check| !report.is_allowed(**check))?;
            Some(denied_text(*denied, checks).into())
        }
    }
}

/// The verb pairs that read as one right: a server before Kubernetes 1.35 asks for `get`, one after
/// asks for both.
const VERB_PAIRS: [(AccessCheck, AccessCheck, &str); 2] = [
    (
        AccessCheck::GetPodExec,
        AccessCheck::CreatePodExec,
        "pods/exec",
    ),
    (
        AccessCheck::GetPodPortForward,
        AccessCheck::CreatePodPortForward,
        "pods/portforward",
    ),
];

fn denied_text(denied: AccessCheck, checks: &[AccessCheck]) -> String {
    let pair = VERB_PAIRS.iter().find(|(get, create, _)| {
        (denied == *get || denied == *create) && checks.contains(get) && checks.contains(create)
    });
    match pair {
        Some((_, _, resource)) => format!("Not permitted: get and create {resource}"),
        None => format!("Not permitted: {denied}"),
    }
}

fn disabled(reason: impl Into<SharedString>) -> ActionAvailability {
    ActionAvailability::Disabled {
        reason: reason.into(),
    }
}

/// The handles the items of a pod menu act through.
pub(crate) struct PodMenuLinks<'a> {
    pub(crate) dock: &'a WeakEntity<Dock>,
    pub(crate) shell: &'a WeakEntity<AppShell>,
}

/// Shared by the row context menu and the drawer header menu, so both always agree. `open_shell`
/// is built by the caller (`ShellMenu::item`) because a submenu needs the app.
pub(crate) fn pod_menu(
    menu: PopupMenu,
    pod: &PodSummary,
    live: &LiveCluster,
    guard: &ClusterGuard<'_>,
    row: &RowContext,
    links: &PodMenuLinks<'_>,
    open_shell: PopupMenuItem,
) -> PopupMenu {
    let PodMenuLinks { dock, shell } = *links;
    let access = guard.access;
    let menu = menu
        .item(view_logs_item(pod, None, live, row, dock))
        .item(open_shell)
        .item(action_item(ResourceAction::PortForward, guard))
        .item(view_yaml_item(row.object(ResourceKey::of_pod(pod)), shell))
        .separator()
        .item(copy_name_item(&pod.name, access))
        .item(copy_kubectl_command_item(&row.context, pod));
    with_cluster_filter(menu, row, shell)
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

/// What View logs opens for `pod`, on `container` when the pod has one of that name: the log
/// target, or why it is unavailable (no log access, or no container to read). The menu item and the
/// Overview button both call it.
pub(crate) fn logs_launch(
    pod: &PodSummary,
    container: Option<&str>,
    access: &AccessState,
) -> Result<LogTarget, SharedString> {
    if let ActionAvailability::Disabled { reason } =
        availability_before_lock(ResourceAction::ViewLogs, access)
    {
        return Err(reason);
    }
    let named = container.and_then(|name| LogTarget::of_container(pod, name));
    named
        .or_else(|| LogTarget::of_pod(pod))
        .ok_or_else(|| "The pod has no containers".into())
}

/// Opens the pod in the log dock; see `logs_launch`.
pub(crate) fn view_logs_item(
    pod: &PodSummary,
    container: Option<&str>,
    live: &LiveCluster,
    row: &RowContext,
    dock: &WeakEntity<Dock>,
) -> PopupMenuItem {
    let label = action_label(ResourceAction::ViewLogs);
    match logs_launch(pod, container, &live.access) {
        Err(reason) => disabled_menu_item(label, reason),
        Ok(target) => {
            let connection = live.connection().clone();
            let row = row.clone();
            let dock = dock.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = dock.update(cx, |dock, cx| {
                    let origin = LogOrigin::new(&row, connection.clone());
                    dock.open(origin, target.clone(), window, cx)
                });
            })
        }
    }
    .action(RowAction::ViewLogs.key_action())
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
    Some((
        label,
        availability_before_lock(ResourceAction::ViewLogs, access),
    ))
}

/// Merges the logs of every pod of the workload into one dock tab.
fn workload_logs_item(
    row: &KindRow,
    access: &AccessState,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> Option<PopupMenuItem> {
    let (label, availability) = workload_logs_entry(row.related_pods.as_ref(), access)?;
    Some(match availability {
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
        ActionAvailability::Enabled => {
            let owner = row.related_pods.clone()?;
            let cluster = context.cluster.clone();
            let shell = shell.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.open_workload_logs(&cluster, owner.clone(), window, cx);
                });
            })
        }
    })
}

pub(crate) fn node_menu(
    menu: PopupMenu,
    node: &NodeSummary,
    live: &LiveCluster,
    guard: &ClusterGuard<'_>,
    row: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenu {
    let access = guard.access;
    let menu = menu
        .item(action_item(ResourceAction::OpenNodeShell, guard))
        .item(view_yaml_item(
            row.object(ResourceKey::of_node(node)),
            shell,
        ))
        .item(view_pods_on_node_item(node, live.pods.items(), row, shell))
        .separator()
        .item(cordon_item(node, guard, row, shell))
        .item(action_item(ResourceAction::Drain, guard))
        .separator()
        .item(copy_name_item(&node.name, access));
    with_cluster_filter(menu, row, shell)
}

/// Switches to Pods with only the pods of the node. Always enabled, even for an empty node.
fn view_pods_on_node_item(
    node: &NodeSummary,
    pods: &[PodSummary],
    row: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let name = node.name.clone();
    let cluster = row.cluster.clone();
    let shell = shell.clone();
    PopupMenuItem::new(format!(
        "View pods on node · {}",
        pods_on_node(pods, &node.name)
    ))
    .on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.view_pods_on_node(&cluster, &name, cx));
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

/// The cluster a kind menu is built for: the guard, the pod list, and the row context of the row's
/// own slot.
pub(crate) struct MenuCluster<'a> {
    pub(crate) guard: &'a ClusterGuard<'a>,
    pub(crate) pods: &'a [PodSummary],
    pub(crate) context: &'a RowContext,
}

/// The row context menu and the drawer ⋯ menu of an explorer kind. Every item except View YAML and
/// Copy name (and, for events, Go to object and Copy message) is disabled, because this version
/// is read-only.
pub(crate) fn kind_menu(
    menu: PopupMenu,
    kind: ResourceKind,
    row: &KindRow,
    cluster: &MenuCluster<'_>,
    shell: &WeakEntity<AppShell>,
    extras: MenuExtras,
) -> PopupMenu {
    let MenuCluster {
        guard,
        pods,
        context,
    } = *cluster;
    let access = guard.access;
    let mut menu = menu;
    if has_who_can(kind) {
        menu = menu.item(who_can_item(row, context, shell));
    }
    if has_check_permissions(kind) {
        menu = menu.item(check_permissions_item(row, context, shell));
    }
    if has_test_traffic(kind) {
        menu = menu.item(test_traffic_item(row, context, shell));
    }
    if let Some(item) = workload_logs_item(row, access, context, shell) {
        menu = menu.item(item);
    }
    if let Some(secret) = extras.secret {
        menu = menu.item(secret.reveal).item(secret.copy).separator();
    }
    if let Some(event) = &row.event {
        menu = menu
            .item(go_to_object_item(event, context, shell))
            .item(filter_similar_item(event, shell))
            .item(copy_message_item(event))
            .separator();
    }
    if let Some(browse) = extras.browse {
        menu = menu.item(browse);
    }
    let key = ResourceKey::of_row(kind, row);
    let object = context.object(key.clone());
    // A key without an object reference (a Helm release) has no YAML tab.
    if kind == ResourceKind::HelmReleases {
        for (label, tab) in HELM_VIEW_ITEMS {
            menu = menu.item(view_tab_item(label, object.clone(), tab, shell));
        }
    }
    if object_ref(&key).is_some() {
        menu = menu.item(view_yaml_item(object, shell));
    }
    if let Some(item) = extras.open_url {
        menu = menu.item(item);
    }
    match topology_menu(
        kind,
        row.namespace.as_deref(),
        extras.scope.as_ref(),
        (!context.is_primary).then_some(context.primary_label.as_str()),
    ) {
        TopologyMenu::Hidden => {}
        TopologyMenu::Enabled => menu = menu.item(show_in_topology_item(key.clone(), shell)),
        TopologyMenu::Disabled(reason) => {
            menu = menu.item(disabled_menu_item("Show in Topology", reason.into()));
        }
    }
    if has_go_to_target(kind) {
        menu = menu.item(go_to_target_item(row, context, shell));
    }
    if has_go_to_owner(kind) {
        menu = menu.item(go_to_owner_item(row, context, shell));
    }
    match kind {
        ResourceKind::PersistentVolumeClaims => {
            menu = menu.item(go_to_pod_item(row, pods, context, shell));
        }
        ResourceKind::PersistentVolumes => {
            menu = menu.item(go_to_claim_item(row, context, shell));
        }
        ResourceKind::RoleBindings | ResourceKind::ClusterRoleBindings => {
            menu = menu.item(go_to_role_item(row, context, shell));
        }
        _ => {}
    }
    if kind.has_port_forward() {
        menu = menu.item(action_item(ResourceAction::PortForward, guard));
    }
    if let Some(is_default) =
        default_namespace_state(kind, &row.name, extras.default_namespace.as_deref())
    {
        menu = menu.item(default_namespace_item(
            context.cluster.clone(),
            row.name.clone(),
            is_default,
            shell,
        ));
    }
    let change_actions = kind.read_only_actions();
    if !change_actions.is_empty() {
        menu = menu.separator();
    }
    for item in change_actions {
        let entry = disabled_menu_item(item.label, NOT_SHIPPED_REASON.into());
        menu = menu.item(match item.action {
            Some(action) => entry.action(action.row_action().key_action()),
            None => entry,
        });
    }
    let menu = menu
        .separator()
        .item(copy_name_item(&row.name, access))
        .separator()
        .item(
            disabled_menu_item(kind.delete_label(), NOT_SHIPPED_REASON.into())
                .action(RowAction::Delete.key_action()),
        );
    with_cluster_filter(menu, context, shell)
}

/// While several clusters are viewed: `Filter by this cluster`, which keeps the rows of the
/// row's cluster (an Equals chip on the Cluster column).
fn with_cluster_filter(
    menu: PopupMenu,
    row: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenu {
    if !row.is_multi {
        return menu;
    }
    let (label, shell) = (row.label.clone(), shell.clone());
    menu.separator().item(
        PopupMenuItem::new("Filter by this cluster").on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.filter_by_cluster(&label, cx));
        }),
    )
}

/// Whether the menu of a kind offers Show in Topology.
#[derive(Debug, PartialEq, Eq)]
enum TopologyMenu {
    /// Only Services and Ingresses have the item.
    Hidden,
    Enabled,
    /// The namespace is outside the session's scope, so Topology could not draw it, or the row
    /// is not in the cluster Topology draws.
    Disabled(String),
}

/// `other_primary`: the label of the primary cluster when the row is in another one, because
/// Topology draws the primary cluster alone.
fn topology_menu(
    kind: ResourceKind,
    namespace: Option<&str>,
    scope: Option<&NamespaceScope>,
    other_primary: Option<&str>,
) -> TopologyMenu {
    if !matches!(kind, ResourceKind::Services | ResourceKind::Ingresses) {
        return TopologyMenu::Hidden;
    }
    let (Some(namespace), Some(scope)) = (namespace, scope) else {
        return TopologyMenu::Hidden;
    };
    if let Some(primary) = other_primary {
        return TopologyMenu::Disabled(format!(
            "Topology draws only the primary cluster ({primary})"
        ));
    }
    if scope_includes(scope, namespace) {
        TopologyMenu::Enabled
    } else {
        TopologyMenu::Disabled(format!("Namespace {namespace} is outside the scope"))
    }
}

fn show_in_topology_item(key: ResourceKey, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new("Show in Topology").on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.show_in_topology(&key, cx));
    })
}

/// Whether the Namespaces menu offers "Set as default namespace" for `row_name`, and if so
/// whether it already is the default (the item is then checked). `None` for every other kind.
fn default_namespace_state(
    kind: ResourceKind,
    row_name: &str,
    default_namespace: Option<&str>,
) -> Option<bool> {
    (kind == ResourceKind::Namespaces).then(|| default_namespace == Some(row_name))
}

/// Stores the namespace as the cluster's default, or clears it when it already is. Local only:
/// nothing is sent to the cluster, and the current scope does not change.
fn default_namespace_item(
    cluster: ClusterRef,
    name: String,
    is_default: bool,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new("Set as default namespace")
        .checked(is_default)
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.toggle_default_namespace(&cluster, &name, cx)
            });
        })
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

/// What the caller supplies beyond the row and the access state: items that need the window or
/// the app to be built (so the caller builds them before it borrows the session; a submenu needs
/// the app mutably), and data the menu reads, such as the default namespace.
#[derive(Default)]
pub(crate) struct MenuExtras {
    pub(crate) open_url: Option<PopupMenuItem>,
    pub(crate) secret: Option<SecretMenu>,
    /// A CRD row's Browse instances item.
    pub(crate) browse: Option<PopupMenuItem>,
    /// The row cluster's default namespace, for the Namespaces menu.
    pub(crate) default_namespace: Option<String>,
    /// The scope of the session, for Show in Topology.
    pub(crate) scope: Option<NamespaceScope>,
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
    object: ClusterObject,
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
            object.clone(),
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
                    object.clone(),
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

/// A Reveal or Copy item: it opens the drawer of `object` and runs `action` through the shell.
fn secret_action_item(
    label: SharedString,
    object: ClusterObject,
    action: SecretAction,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| {
            shell.run_secret_action(object.clone(), action.clone(), cx);
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

/// A menu item that reveals `target` in the row's cluster; disabled with `reason` when there is
/// none.
fn go_to_item(
    label: &'static str,
    target: Option<ResourceKey>,
    reason: SharedString,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let Some(key) = target else {
        return disabled_menu_item(label, reason);
    };
    let object = context.object(key);
    let shell = shell.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.reveal_object(object.clone(), cx));
    })
}

/// Reveals the owner (a Deployment, usually); disabled when there is none.
fn go_to_owner_item(
    row: &KindRow,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    go_to_item(
        "Go to owner",
        owner_target(row),
        "No owner".into(),
        context,
        shell,
    )
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

fn go_to_target_item(
    row: &KindRow,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    go_to_item(
        "Go to target",
        scale_target(row),
        no_target_screen_reason(row),
        context,
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
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    go_to_item(
        "Go to pod",
        claim_pod_target(row, pods),
        "Not mounted by any pod".into(),
        context,
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

fn go_to_claim_item(
    row: &KindRow,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    go_to_item(
        "Go to claim",
        volume_claim_target(row),
        "No claim".into(),
        context,
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

fn go_to_role_item(
    row: &KindRow,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    go_to_item(
        "Go to role",
        binding_role_target(row),
        "No screen for this role".into(),
        context,
        shell,
    )
}

/// Reveals the involved object on its own screen; disabled when there is none.
fn go_to_object_item(
    event: &EventDetail,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    const LABEL: &str = "Go to object";
    let Some(key) = event.object.clone() else {
        return disabled_menu_item(LABEL, "No screen for this kind yet".into());
    };
    let object = context.object(key);
    let shell = shell.clone();
    PopupMenuItem::new(LABEL).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.reveal_object(object.clone(), cx));
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

/// Roles and ClusterRoles offer Who can… as their first menu item.
fn has_who_can(kind: ResourceKind) -> bool {
    matches!(kind, ResourceKind::Roles | ResourceKind::ClusterRoles)
}

/// NetworkPolicies offer Test traffic… as their first menu item.
fn has_test_traffic(kind: ResourceKind) -> bool {
    kind == ResourceKind::NetworkPolicies
}

/// Opens Test traffic… with the destination the policy selects, checked at once.
fn test_traffic_item(
    row: &KindRow,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let cluster = context.cluster.clone();
    let shell = shell.clone();
    let policy = match &row.object {
        KindObject::NetworkPolicy(policy) => Some(policy.clone()),
        _ => None,
    };
    PopupMenuItem::new("Test traffic…").on_click(move |_, window, cx| {
        let policy = policy.clone();
        let _ = shell.update(cx, |shell, cx| {
            shell.open_traffic_test(&cluster, policy.as_ref(), true, window, cx);
        });
    })
}

/// Service accounts offer Check permissions as their first menu item.
fn has_check_permissions(kind: ResourceKind) -> bool {
    kind == ResourceKind::ServiceAccounts
}

/// Opens Check permissions for this account, in its own namespace, checked at once.
fn check_permissions_item(
    row: &KindRow,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let cluster = context.cluster.clone();
    let shell = shell.clone();
    let namespace = row.namespace.clone();
    let subject = namespace
        .as_ref()
        .map(|namespace| format!("sa {namespace}/{}", row.name));
    PopupMenuItem::new("Check permissions").on_click(move |_, window, cx| {
        let subject = subject.clone();
        let namespace = namespace.clone();
        let _ = shell.update(cx, |shell, cx| {
            shell.open_permissions(&cluster, subject, namespace, true, window, cx);
        });
    })
}

/// The Who can… query a role row prefills: its first resource rule, if it has one.
fn who_can_query(row: &KindRow) -> Option<String> {
    match &row.object {
        KindObject::Role(role) => who_can_prefill(&role.rules),
        _ => None,
    }
}

/// Opens Who can… on the role's own namespace (cluster-wide for a ClusterRole), prefilled and
/// asked at once when the role has a rule to ask about.
fn who_can_item(
    row: &KindRow,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let cluster = context.cluster.clone();
    let shell = shell.clone();
    let query = who_can_query(row);
    let namespace = row.namespace.clone();
    PopupMenuItem::new("Who can…").on_click(move |_, window, cx| {
        let query = query.clone();
        let namespace = namespace.clone();
        let _ = shell.update(cx, |shell, cx| {
            let check_now = query.is_some();
            shell.open_who_can(&cluster, query, namespace, check_now, window, cx);
        });
    })
}

/// Opens the drawer of `object` on its YAML tab. Always enabled: a missing right shows inline
/// there.
fn view_yaml_item(object: ClusterObject, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    view_tab_item(
        action_label(ResourceAction::ViewYaml),
        object,
        DrawerTab::Yaml,
        shell,
    )
    .action(RowAction::ViewYaml.key_action())
}

/// Opens the drawer of `object` on `tab`; the same item serves View YAML, View values, and View
/// manifest.
fn view_tab_item(
    label: &'static str,
    object: ClusterObject,
    tab: DrawerTab,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| {
            shell.open_drawer_tab(object.clone(), tab, cx)
        });
    })
}

/// Why the logs of a container cannot be opened, or `None` when they can. Without a live session
/// there is nothing to read from.
pub(crate) fn view_logs_reason(live: Option<&LiveCluster>) -> Option<SharedString> {
    let Some(live) = live else {
        return Some("Not connected".into());
    };
    match availability_before_lock(ResourceAction::ViewLogs, &live.access) {
        ActionAvailability::Disabled { reason } => Some(reason),
        ActionAvailability::Enabled => None,
    }
}

/// Whether a container can take a shell now.
fn is_running(container: &ContainerSummary) -> bool {
    matches!(container.state, ContainerState::Running { .. })
}

/// The container S opens: the first running main container, else the first running one. Init
/// containers are never offered.
pub(crate) fn default_shell_container(pod: &PodSummary) -> Option<&ContainerSummary> {
    let mut running = pod
        .containers
        .iter()
        .filter(|container| container.kind != ContainerKind::Init && is_running(container));
    let first = running.next()?;
    if first.kind == ContainerKind::Main {
        return Some(first);
    }
    Some(
        running
            .find(|container| container.kind == ContainerKind::Main)
            .unwrap_or(first),
    )
}

/// One container of the Open shell submenu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShellChoice {
    pub(crate) name: String,
    /// `MAIN` or `SIDECAR`.
    pub(crate) tag: &'static str,
    pub(crate) is_running: bool,
}

/// What the Open shell item of a pod offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShellMenuState {
    Disabled(SharedString),
    /// A pod with one container opens it directly.
    One(String),
    /// Several containers: a submenu, a container that is not running disabled.
    Pick(Vec<ShellChoice>),
}

/// The Open shell item of a pod, decided from the gate of the pod's own cluster and its containers.
/// Pure.
pub(crate) fn shell_menu_state(pod: &PodSummary, guard: &ClusterGuard<'_>) -> ShellMenuState {
    if let ActionAvailability::Disabled { reason } =
        action_availability(ResourceAction::OpenShell, guard)
    {
        return ShellMenuState::Disabled(reason);
    }
    let choices: Vec<ShellChoice> = pod
        .containers
        .iter()
        .filter(|container| container.kind != ContainerKind::Init)
        .map(|container| ShellChoice {
            name: container.name.clone(),
            tag: kind_tag_text(container.kind),
            is_running: is_running(container),
        })
        .collect();
    match choices.as_slice() {
        [] => ShellMenuState::Disabled("The pod has no containers".into()),
        [only] if only.is_running => ShellMenuState::One(only.name.clone()),
        [_] => ShellMenuState::Disabled(NOT_RUNNING_REASON.into()),
        _ if choices.iter().any(|choice| choice.is_running) => ShellMenuState::Pick(choices),
        _ => ShellMenuState::Disabled("No running container".into()),
    }
}

const NOT_RUNNING_REASON: &str = "Container is not running";

/// The Open shell item of a pod menu, with everything it needs owned: a submenu is built from the
/// app, so a caller makes this before it borrows the session (like `secret_menu`).
pub(crate) struct ShellMenu {
    state: ShellMenuState,
    namespace: String,
    pod: String,
    short_pod: String,
}

impl ShellMenu {
    pub(crate) fn of(pod: &PodSummary, guard: &ClusterGuard<'_>) -> Self {
        Self {
            state: shell_menu_state(pod, guard),
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            short_pod: short_pod_name(pod),
        }
    }

    /// Each entry opens its own container in the row's cluster, through the guarded flow.
    pub(crate) fn item(
        self,
        row: &RowContext,
        shell: &WeakEntity<AppShell>,
        window: &mut Window,
        cx: &mut App,
    ) -> PopupMenuItem {
        let label = action_label(ResourceAction::OpenShell);
        let Self {
            state,
            namespace,
            pod,
            short_pod,
        } = self;
        let open = {
            let (cluster, shell) = (row.cluster.clone(), shell.clone());
            move |container: String| -> StartShell {
                let open = ShellOpen {
                    cluster: cluster.clone(),
                    namespace: namespace.clone(),
                    pod: pod.clone(),
                    short_pod: short_pod.clone(),
                    container,
                };
                let shell = shell.clone();
                Box::new(move |window: &mut Window, cx: &mut App| {
                    let open = open.clone();
                    let _ = shell.update(cx, |shell, cx| shell.start_shell(open, window, cx));
                })
            }
        };
        match state {
            ShellMenuState::Disabled(reason) => {
                disabled_menu_item(label, reason).action(RowAction::OpenShell.key_action())
            }
            ShellMenuState::One(container) => {
                let start = open(container);
                PopupMenuItem::new(label)
                    .on_click(move |_, window, cx| start(window, cx))
                    .action(RowAction::OpenShell.key_action())
            }
            ShellMenuState::Pick(choices) => {
                let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
                    choice_items(submenu, &choices, &open)
                });
                PopupMenuItem::submenu(label, submenu)
            }
        }
    }
}

/// The call an entry of the submenu makes when it is clicked.
type StartShell = Box<dyn Fn(&mut Window, &mut App)>;

/// One entry per container of the submenu: a running one opens a shell, one that is not running is
/// shown disabled with its reason.
fn choice_items(
    menu: PopupMenu,
    choices: &[ShellChoice],
    open: &dyn Fn(String) -> StartShell,
) -> PopupMenu {
    choices.iter().fold(menu, |menu, choice| {
        let text = format!("{} · {}", choice.name, choice.tag);
        if !choice.is_running {
            return menu.item(disabled_menu_item(text, NOT_RUNNING_REASON.into()));
        }
        let start = open(choice.name.clone());
        menu.item(PopupMenuItem::new(text).on_click(move |_, window, cx| start(window, cx)))
    })
}

/// `--screen shell-picker-fixture`: the container list of the Open shell submenu, in a dialog, since a
/// submenu cannot be held open by a command line. Nothing in it opens a shell.
#[cfg(feature = "screenshot")]
pub(crate) fn open_shell_picker_fixture(window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    let choices = vec![
        ShellChoice {
            name: "api".to_owned(),
            tag: "MAIN",
            is_running: true,
        },
        ShellChoice {
            name: "worker".to_owned(),
            tag: "MAIN",
            is_running: true,
        },
        ShellChoice {
            name: "istio-proxy".to_owned(),
            tag: "SIDECAR",
            is_running: true,
        },
        ShellChoice {
            name: "migrate".to_owned(),
            tag: "SIDECAR",
            is_running: false,
        },
    ];
    let menu = PopupMenu::build(window, cx, move |menu, _, _| {
        choice_items(menu, &choices, &|_| Box::new(|_, _| {}))
    });
    window.open_dialog(cx, move |dialog, _, _| {
        // The menu draws its own border, so the dialog adds neither a title nor padding.
        dialog
            .w(gpui_kit::px(300.))
            .p_0()
            .close_button(false)
            .child(menu.clone())
    });
}

/// The tooltip of a disabled Forward button. Port-forward has not shipped, so the lock never decides.
pub(crate) fn port_forward_reason(access: &AccessState) -> SharedString {
    match availability_before_lock(ResourceAction::PortForward, access) {
        ActionAvailability::Disabled { reason } => reason,
        ActionAvailability::Enabled => NOT_SHIPPED_REASON.into(),
    }
}

/// Cordon, or Uncordon on a cordoned node. It acts on the node of the row's own cluster, never on the
/// cursor or the primary.
fn cordon_item(
    node: &NodeSummary,
    guard: &ClusterGuard<'_>,
    row: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let label = cordon_label(&node.status.scheduling);
    match action_availability(ResourceAction::Cordon, guard) {
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
        ActionAvailability::Enabled => {
            let (cluster, name, shell) = (row.cluster.clone(), node.name.clone(), shell.clone());
            let scheduling = node.status.scheduling;
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.start_cordon(&cluster, &name, Some(scheduling), window, cx);
                });
            })
        }
    }
    .action(RowAction::Cordon.key_action())
}

/// Disabled items stay visible with their reason, so users learn what exists. The item shows the
/// key of its action. Drain keeps its ellipsis because it opens a dialog.
fn action_item(action: ResourceAction, guard: &ClusterGuard<'_>) -> PopupMenuItem {
    let label = match action {
        ResourceAction::Drain => "Drain…",
        _ => action_label(action),
    };
    match action_availability(action, guard) {
        ActionAvailability::Enabled => PopupMenuItem::new(label),
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
    }
    .action(action.row_action().key_action())
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
    let label = action_label(ResourceAction::CopyName);
    match availability_before_lock(ResourceAction::CopyName, access) {
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
        ActionAvailability::Enabled => {
            let name = name.to_owned();
            PopupMenuItem::new(label).on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
            })
        }
    }
    .action(RowAction::CopyName.key_action())
}

#[cfg(test)]
#[path = "resource_actions_tests.rs"]
mod resource_actions_tests;
