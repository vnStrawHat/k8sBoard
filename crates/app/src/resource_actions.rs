use std::rc::Rc;

use cluster::{
    AccessCheck, ClusterConnection, ContainerKind, ContainerState, ContainerSummary,
    ContainerTerminal, HELM_RELEASE_SECRET_TYPE, NamespacePhase, NamespaceScope, NodeSummary,
    ObjectKind, PodStatus, PodSummary, ReplicaSetSummary, SecretDetails, SecretKey,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::{
    Action, AnyElement, App, ClipboardItem, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window, div, px,
};

use crate::access_bindings::role_key;
use crate::access_query::who_can_prefill;
use crate::app_shell::object_delete::HELM_RECORD_REASON;
use crate::app_shell::shell_open::ShellOpen;
use crate::app_shell::write_flow::cordon_label;
use crate::app_shell::{AppShell, Screen};
use crate::cluster_registry::ClusterRef;
use crate::cluster_session::{AccessState, LiveCluster, scope_includes};
use crate::custom_kind::CustomKind;
use crate::dock::{Dock, LogOrigin};
use crate::drawer::DrawerTab;
use crate::keymap::{
    Attach, CopyName, Cordon, DebugContainer, Delete, Drain, EditHpaRange, EditLabels, EditTaints,
    EditValues, EditYaml, EvictPod, ExpandClaim, OpenShell, PauseRollout, PortForward,
    RenewCertificate, RerunJob, RestartPod, RestartRollout, RollBack, Scale,
    SetDefaultStorageClass, SuspendCronJob, TriggerCronJob, ViewLogs, ViewYaml,
};
use crate::kind_access::{KindAccess, KindAccessMap};
use crate::kind_join::last_job_owner;
use crate::kind_row::{EventDetail, JOB_KIND, KindObject, KindRow, PodOwner};
use crate::live_sections::claim_pods;
use crate::log_target::{LogTarget, workload_label};
use crate::log_workload::pod_tab_name;
use crate::namespace_rows::REMAINING_RESOURCES_TITLE;
use crate::network_rows::ingress_urls;
use crate::pod_drawer::kind_tag_text;
use crate::policy_rows::SELECTED_PODS_TITLE;
use crate::resource_kind::ResourceKind;
use crate::row_context::RowContext;
use crate::secret_values::{SecretAction, ValueAccess};

use crate::table_selection::{ClusterObject, ResourceKey};
use crate::workload_actions::{row_block, state_label};
use crate::write_guard::{ActionRisk, ClusterGuard, WriteLock};
use crate::yaml_view::object_ref;

/// Why a mutating action is off while its spec has not shipped.
pub(crate) const NOT_SHIPPED_REASON: &str = "Comes in a later version";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceAction {
    ViewLogs,
    OpenShell,
    PortForward,
    /// Attaches to a running container that has a terminal (spec 0040).
    Attach,
    OpenNodeShell,
    /// Adds an ephemeral container to a running pod and attaches a shell to it (spec 0037).
    DebugContainer,
    Cordon,
    /// Only the bulk Uncordon button uses it; the single-node key and menu item are `Cordon`,
    /// whose label follows the node's state (spec 0034).
    Uncordon,
    Drain,
    /// Edit taints… and Edit labels… of one node (spec 0034).
    EditTaints,
    EditLabels,
    CopyName,
    ViewYaml,
    /// Carries the kind of the row: only the editable kinds offer it (spec 0031).
    EditYaml(ObjectKind),
    /// The key and value editor of a ConfigMap or Secret; carries the kind of the row (spec 0047).
    EditValues(ObjectKind),
    /// Carries the kind of the new object: the `New` header button of its screen (spec 0042). It
    /// has no row and no key.
    CreateObject(ObjectKind),
    /// Carries the kind of the row: every built-in kind but a Helm release is deleted (spec 0033).
    Delete(ObjectKind),
    /// Carries the kind of the row: Deployments, StatefulSets, and DaemonSets restart.
    RestartRollout(ObjectKind),
    /// Deletes a controller-owned pod so its controller recreates it (spec 0040).
    RestartPod,
    /// Asks the eviction API to remove a pod, which checks its PodDisruptionBudgets (spec 0040).
    EvictPod,
    /// Carries the kind of the row: Deployments and StatefulSets scale.
    Scale(ObjectKind),
    PauseRollout,
    RollBack,
    SuspendCronJob,
    TriggerCronJob,
    RerunJob,
    /// Sets the min and max replicas of an HPA (spec 0032b).
    EditHpaRange,
    /// Grows the storage request of a PVC; it cannot shrink again (spec 0032b).
    ExpandClaim,
    /// Makes a StorageClass the default and unsets the old default (spec 0032b).
    SetDefaultStorageClass,
    /// Adds `Issuing=True` to a cert-manager Certificate's status, which makes cert-manager issue
    /// a new certificate now (spec 0018 step 6).
    RenewCertificate,
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
    /// A on a pod: the default container with a terminal (spec 0040).
    Attach,
    /// Has an unbound key action only: the menu item carries the container it acts on.
    DebugContainer,
    Cordon,
    Drain,
    /// Have unbound unit actions only: the node menu and the palette dispatch them (spec 0034).
    EditTaints,
    EditLabels,
    EditYaml,
    /// Edit values… of a ConfigMap or Secret (spec 0047); its key is E on those two screens.
    EditValues,
    RestartRollout,
    /// Have unbound unit actions only: the pod menu and the palette dispatch them (spec 0040).
    RestartPod,
    EvictPod,
    Scale,
    Delete,
    PauseRollout,
    RollBack,
    SuspendCronJob,
    TriggerCronJob,
    RerunJob,
    EditHpaRange,
    ExpandClaim,
    SetDefaultStorageClass,
    RenewCertificate,
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

/// The object kind Edit YAML edits on a row of `kind`; `None` for a kind that is not editable. A
/// Helm release reads as a Secret, but its record is never edited here.
pub(crate) fn edit_yaml_kind(kind: ResourceKind) -> Option<ObjectKind> {
    if kind == ResourceKind::HelmReleases {
        return None;
    }
    kind.builtin_object().filter(|object| object.is_editable())
}

/// The object kind Edit values edits on a row of `kind`: ConfigMaps and Secrets. A Helm release
/// reads as a Secret by storage, but its record is never edited here.
pub(crate) fn edit_values_kind(kind: ResourceKind) -> Option<ObjectKind> {
    match kind {
        ResourceKind::ConfigMaps => Some(ObjectKind::ConfigMap),
        ResourceKind::Secrets => Some(ObjectKind::Secret),
        _ => None,
    }
}

/// The permission a restart of `kind` needs; `None` for a kind that does not restart.
fn restart_check(kind: ObjectKind) -> Option<AccessCheck> {
    match kind {
        ObjectKind::Deployment => Some(AccessCheck::PatchDeployments),
        ObjectKind::StatefulSet => Some(AccessCheck::PatchStatefulSets),
        ObjectKind::DaemonSet => Some(AccessCheck::PatchDaemonSets),
        _ => None,
    }
}

/// The permission a scale of `kind` needs (the `scale` subresource); `None` for a kind that does
/// not scale.
fn scale_check(kind: ObjectKind) -> Option<AccessCheck> {
    match kind {
        ObjectKind::Deployment => Some(AccessCheck::PatchDeploymentScale),
        ObjectKind::StatefulSet => Some(AccessCheck::PatchStatefulSetScale),
        _ => None,
    }
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
            // Like exec: a WebSocket upgrade is authorized as `get`, and from Kubernetes 1.35 as
            // `create` too (spec 0035), so a forward needs both.
            Self::PortForward => ActionGate::Mutating {
                checks: vec![
                    AccessCheck::GetPodPortForward,
                    AccessCheck::CreatePodPortForward,
                ],
                is_shipped: true,
            },
            // Like exec: an attach upgrade is `get`, and from Kubernetes 1.35 `create` too (0040).
            Self::Attach => ActionGate::Mutating {
                checks: vec![AccessCheck::GetPodAttach, AccessCheck::CreatePodAttach],
                is_shipped: true,
            },
            // The node shell creates a privileged pod, attaches to it, and deletes it (spec 0037).
            Self::OpenNodeShell => ActionGate::Mutating {
                checks: vec![
                    AccessCheck::CreatePods,
                    AccessCheck::DeletePods,
                    AccessCheck::WatchPods,
                    AccessCheck::GetPodAttach,
                    AccessCheck::CreatePodAttach,
                ],
                is_shipped: true,
            },
            // An ephemeral container is patched in, then attached to (spec 0037).
            Self::DebugContainer => ActionGate::Mutating {
                checks: vec![
                    AccessCheck::PatchPodEphemeralContainers,
                    AccessCheck::WatchPods,
                    AccessCheck::GetPodAttach,
                    AccessCheck::CreatePodAttach,
                ],
                is_shipped: true,
            },
            // Spec 0030: the first shipped mutating action.
            Self::Cordon | Self::Uncordon | Self::EditTaints | Self::EditLabels => {
                ActionGate::Mutating {
                    checks: vec![AccessCheck::PatchNodes],
                    is_shipped: true,
                }
            }
            // The check follows the carried kind; a kind with no such action has no check.
            Self::RestartRollout(kind) => match restart_check(kind) {
                Some(check) => ActionGate::Mutating {
                    checks: vec![check],
                    is_shipped: true,
                },
                None => ActionGate::Planned,
            },
            Self::PauseRollout => ActionGate::Mutating {
                checks: vec![AccessCheck::PatchDeployments],
                is_shipped: true,
            },
            Self::SuspendCronJob => ActionGate::Mutating {
                checks: vec![AccessCheck::PatchCronJobs],
                is_shipped: true,
            },
            Self::TriggerCronJob | Self::RerunJob => ActionGate::Mutating {
                checks: vec![AccessCheck::CreateJobs],
                is_shipped: true,
            },
            Self::Scale(kind) => match scale_check(kind) {
                Some(check) => ActionGate::Mutating {
                    checks: vec![check],
                    is_shipped: true,
                },
                None => ActionGate::Planned,
            },
            Self::RollBack => ActionGate::Mutating {
                checks: vec![AccessCheck::PatchDeployments],
                is_shipped: true,
            },
            Self::EditYaml(kind) => ActionGate::Mutating {
                checks: vec![AccessCheck::Update(kind)],
                is_shipped: true,
            },
            // A merge patch needs `patch`, not `update` (spec 0047 decision 7).
            Self::EditValues(kind) => ActionGate::Mutating {
                checks: vec![AccessCheck::Patch(kind)],
                is_shipped: true,
            },
            Self::CreateObject(kind) => ActionGate::Mutating {
                checks: vec![AccessCheck::Create(kind)],
                is_shipped: true,
            },
            Self::Delete(kind) => ActionGate::Mutating {
                checks: vec![AccessCheck::Delete(kind)],
                is_shipped: true,
            },
            // The lazy check Delete pod reads, so the two menu items agree (spec 0040).
            Self::RestartPod => ActionGate::Mutating {
                checks: vec![AccessCheck::Delete(ObjectKind::Pod)],
                is_shipped: true,
            },
            Self::EvictPod => ActionGate::Mutating {
                checks: vec![AccessCheck::CreatePodEviction],
                is_shipped: true,
            },
            Self::EditHpaRange => ActionGate::Mutating {
                checks: vec![AccessCheck::PatchHorizontalPodAutoscalers],
                is_shipped: true,
            },
            Self::ExpandClaim => ActionGate::Mutating {
                checks: vec![AccessCheck::PatchPersistentVolumeClaims],
                is_shipped: true,
            },
            Self::SetDefaultStorageClass => ActionGate::Mutating {
                checks: vec![AccessCheck::PatchStorageClasses],
                is_shipped: true,
            },
            // A custom kind has no lazy per-kind check: this one is in the session report (0018).
            Self::RenewCertificate => ActionGate::Mutating {
                checks: vec![AccessCheck::UpdateCertificateStatus],
                is_shipped: true,
            },
            // A drain evicts pods (spec 0034). It cordons first, so the cordon right is needed too.
            Self::Drain => ActionGate::Mutating {
                checks: vec![AccessCheck::CreatePodEviction, AccessCheck::PatchNodes],
                is_shipped: true,
            },
        }
    }

    /// The kind-less name the key layer, menus, and palette know the action by. The node shell
    /// shares the key of the pod shell. `None` for the `New` header button, which has no row.
    pub(crate) fn row_action(self) -> Option<RowAction> {
        let row = match self {
            Self::ViewLogs => RowAction::ViewLogs,
            Self::OpenShell | Self::OpenNodeShell => RowAction::OpenShell,
            Self::DebugContainer => RowAction::DebugContainer,
            Self::PortForward => RowAction::PortForward,
            Self::Attach => RowAction::Attach,
            Self::Cordon | Self::Uncordon => RowAction::Cordon,
            Self::Drain => RowAction::Drain,
            Self::EditTaints => RowAction::EditTaints,
            Self::EditLabels => RowAction::EditLabels,
            Self::CopyName => RowAction::CopyName,
            Self::ViewYaml => RowAction::ViewYaml,
            Self::EditYaml(_) => RowAction::EditYaml,
            Self::EditValues(_) => RowAction::EditValues,
            Self::Delete(_) => RowAction::Delete,
            Self::RestartRollout(_) => RowAction::RestartRollout,
            Self::RestartPod => RowAction::RestartPod,
            Self::EvictPod => RowAction::EvictPod,
            Self::Scale(_) => RowAction::Scale,
            Self::PauseRollout => RowAction::PauseRollout,
            Self::RollBack => RowAction::RollBack,
            Self::SuspendCronJob => RowAction::SuspendCronJob,
            Self::TriggerCronJob => RowAction::TriggerCronJob,
            Self::RerunJob => RowAction::RerunJob,
            Self::EditHpaRange => RowAction::EditHpaRange,
            Self::ExpandClaim => RowAction::ExpandClaim,
            Self::SetDefaultStorageClass => RowAction::SetDefaultStorageClass,
            Self::RenewCertificate => RowAction::RenewCertificate,
            Self::CreateObject(_) => return None,
        };
        Some(row)
    }
}

impl RowAction {
    /// The key action behind the row action, which menus show as a hint and the palette
    /// dispatches.
    pub(crate) fn key_action(self) -> Box<dyn Action> {
        match self {
            Self::ViewLogs => Box::new(ViewLogs),
            Self::OpenShell => Box::new(OpenShell),
            Self::DebugContainer => Box::new(DebugContainer),
            Self::PortForward => Box::new(PortForward),
            Self::Attach => Box::new(Attach),
            Self::Cordon => Box::new(Cordon),
            Self::Drain => Box::new(Drain),
            Self::EditTaints => Box::new(EditTaints),
            Self::EditLabels => Box::new(EditLabels),
            Self::CopyName => Box::new(CopyName),
            Self::ViewYaml => Box::new(ViewYaml),
            Self::EditYaml => Box::new(EditYaml),
            Self::EditValues => Box::new(EditValues),
            Self::Delete => Box::new(Delete),
            Self::RestartRollout => Box::new(RestartRollout),
            Self::RestartPod => Box::new(RestartPod),
            Self::EvictPod => Box::new(EvictPod),
            Self::Scale => Box::new(Scale),
            Self::PauseRollout => Box::new(PauseRollout),
            Self::RollBack => Box::new(RollBack),
            Self::SuspendCronJob => Box::new(SuspendCronJob),
            Self::TriggerCronJob => Box::new(TriggerCronJob),
            Self::RerunJob => Box::new(RerunJob),
            Self::EditHpaRange => Box::new(EditHpaRange),
            Self::ExpandClaim => Box::new(ExpandClaim),
            Self::SetDefaultStorageClass => Box::new(SetDefaultStorageClass),
            Self::RenewCertificate => Box::new(RenewCertificate),
        }
    }

    /// The icon of the action in menus and the palette (spec 0051).
    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::ViewLogs => IconName::FileText,
            Self::ViewYaml => IconName::FileCode,
            Self::CopyName => IconName::Copy,
            Self::OpenShell => IconName::SquareTerminal,
            Self::Attach => IconName::Plug,
            Self::DebugContainer => IconName::Bug,
            Self::PortForward => IconName::ArrowLeftRight,
            Self::Cordon => IconName::Ban,
            Self::Drain => IconName::ArrowDown,
            Self::EditTaints | Self::EditLabels => IconName::Tag,
            Self::EditYaml => IconName::FilePenLine,
            Self::EditValues => IconName::Pencil,
            Self::RestartRollout | Self::RestartPod => IconName::RotateCw,
            Self::RenewCertificate => IconName::RefreshCw,
            Self::EvictPod => IconName::LogOut,
            Self::Scale | Self::EditHpaRange => IconName::ChevronsUpDown,
            Self::Delete => IconName::Trash,
            Self::PauseRollout => IconName::Pause,
            Self::RollBack => IconName::Undo2,
            Self::SuspendCronJob => IconName::Timer,
            Self::TriggerCronJob => IconName::Play,
            Self::RerunJob => IconName::Repeat,
            Self::ExpandClaim => IconName::HardDriveUpload,
            Self::SetDefaultStorageClass => IconName::Star,
        }
    }
}

/// Whether the action reaches a 0030 confirm (dialog, popover, editor diff, or connect tier): its
/// gate is `Mutating`. The palette marks such entries `needs confirm`.
pub(crate) fn needs_confirm(action: ResourceAction) -> bool {
    matches!(action.gate(), ActionGate::Mutating { .. })
}

/// Whether the action has not shipped: its gate is `Planned`, or `Mutating` with `is_shipped:
/// false`. Such an action can never run, so the palette pairs it with no object.
pub(crate) fn is_planned(action: ResourceAction) -> bool {
    matches!(
        action.gate(),
        ActionGate::Planned
            | ActionGate::Mutating {
                is_shipped: false,
                ..
            }
    )
}

/// What an action can do to the cluster; the confirm dialog's button style follows it.
pub(crate) fn action_risk(action: ResourceAction) -> ActionRisk {
    match action {
        ResourceAction::Delete(_)
        | ResourceAction::Drain
        | ResourceAction::RestartPod
        | ResourceAction::EvictPod => ActionRisk::Destructive,
        // A root shell on the node: the strongest tier, typed in every environment.
        ResourceAction::OpenNodeShell => ActionRisk::Privileged,
        ResourceAction::ViewLogs
        | ResourceAction::OpenShell
        | ResourceAction::PortForward
        | ResourceAction::Attach
        | ResourceAction::DebugContainer
        | ResourceAction::Cordon
        | ResourceAction::Uncordon
        | ResourceAction::EditTaints
        | ResourceAction::EditLabels
        | ResourceAction::CopyName
        | ResourceAction::ViewYaml
        | ResourceAction::EditYaml(_)
        | ResourceAction::EditValues(_)
        | ResourceAction::CreateObject(_)
        | ResourceAction::RestartRollout(_)
        | ResourceAction::Scale(_)
        | ResourceAction::PauseRollout
        | ResourceAction::RollBack
        | ResourceAction::SuspendCronJob
        | ResourceAction::TriggerCronJob
        | ResourceAction::RerunJob
        | ResourceAction::EditHpaRange
        | ResourceAction::ExpandClaim
        | ResourceAction::SetDefaultStorageClass
        | ResourceAction::RenewCertificate => ActionRisk::Change,
    }
}

/// The menu and notice text of an action; one source for both.
pub(crate) fn action_label(action: ResourceAction) -> &'static str {
    match action {
        ResourceAction::ViewLogs => "View logs",
        ResourceAction::OpenShell => "Open shell",
        ResourceAction::PortForward => "Port-forward",
        ResourceAction::Attach => "Attach",
        ResourceAction::OpenNodeShell => "Open node shell",
        ResourceAction::DebugContainer => "Debug container",
        ResourceAction::Cordon => "Cordon",
        ResourceAction::Uncordon => "Uncordon",
        ResourceAction::Drain => "Drain",
        ResourceAction::EditTaints => "Edit taints",
        ResourceAction::EditLabels => "Edit labels",
        ResourceAction::CopyName => "Copy name",
        ResourceAction::ViewYaml => "View YAML",
        ResourceAction::EditYaml(_) => "Edit YAML",
        ResourceAction::EditValues(_) => "Edit values",
        ResourceAction::CreateObject(kind) => create_label(kind),
        ResourceAction::Delete(_) => "Delete",
        ResourceAction::RestartRollout(_) => "Restart rollout",
        ResourceAction::RestartPod => "Restart pod",
        ResourceAction::EvictPod => "Evict",
        ResourceAction::Scale(_) => "Scale",
        ResourceAction::PauseRollout => "Pause rollout",
        ResourceAction::RollBack => "Roll back",
        ResourceAction::SuspendCronJob => "Suspend",
        ResourceAction::TriggerCronJob => "Trigger now",
        ResourceAction::RerunJob => "Re-run job",
        ResourceAction::EditHpaRange => "Edit min / max",
        ResourceAction::ExpandClaim => "Expand",
        ResourceAction::SetDefaultStorageClass => "Set as default",
        ResourceAction::RenewCertificate => "Renew now",
    }
}

/// `New ConfigMap`: the label of a `New` button in notices (the creatable kinds only).
fn create_label(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Namespace => "New Namespace",
        ObjectKind::ConfigMap => "New ConfigMap",
        ObjectKind::ResourceQuota => "New ResourceQuota",
        ObjectKind::PodDisruptionBudget => "New PodDisruptionBudget",
        ObjectKind::RoleBinding => "New RoleBinding",
        _ => "New object",
    }
}

/// The notice of an offered key that is unavailable, such as "Edit YAML is unavailable: Read-only
/// mode".
pub(crate) fn unavailable_text(label: &str, reason: &str) -> String {
    // A lock is the one reason the user can lift on the spot, so the notice says how.
    if reason.ends_with(READ_ONLY_SUFFIX) {
        return format!("{label} is unavailable: {reason} · {UNLOCK_KEYS} to unlock");
    }
    format!("{label} is unavailable: {reason}")
}

/// The action the key `row` runs on `subject`: S opens the node shell on a node. `None` means the
/// subject does not offer it (L on a Service).
pub(crate) fn subject_action(row: RowAction, subject: &ResourceKey) -> Option<ResourceAction> {
    match row {
        RowAction::ViewLogs => match subject {
            ResourceKey::Pod { .. } => true,
            ResourceKey::Node { .. } => false,
            ResourceKey::Kind { kind, .. } => has_workload_logs(*kind),
        }
        .then_some(ResourceAction::ViewLogs),
        RowAction::OpenShell => match subject {
            ResourceKey::Pod { .. } => Some(ResourceAction::OpenShell),
            ResourceKey::Node { .. } => Some(ResourceAction::OpenNodeShell),
            ResourceKey::Kind { .. } => None,
        },
        RowAction::DebugContainer => {
            matches!(subject, ResourceKey::Pod { .. }).then_some(ResourceAction::DebugContainer)
        }
        RowAction::Attach => {
            matches!(subject, ResourceKey::Pod { .. }).then_some(ResourceAction::Attach)
        }
        RowAction::RestartPod => {
            matches!(subject, ResourceKey::Pod { .. }).then_some(ResourceAction::RestartPod)
        }
        RowAction::EvictPod => {
            matches!(subject, ResourceKey::Pod { .. }).then_some(ResourceAction::EvictPod)
        }
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
        RowAction::EditTaints => {
            matches!(subject, ResourceKey::Node { .. }).then_some(ResourceAction::EditTaints)
        }
        RowAction::EditLabels => {
            matches!(subject, ResourceKey::Node { .. }).then_some(ResourceAction::EditLabels)
        }
        RowAction::ViewYaml => object_ref(subject)
            .is_some()
            .then_some(ResourceAction::ViewYaml),
        // The editable kinds only: a Helm release has no object reference, a custom resource no
        // built-in kind.
        RowAction::EditYaml => match subject {
            ResourceKey::Pod { .. } => Some(ObjectKind::Pod),
            ResourceKey::Node { .. } => None,
            ResourceKey::Kind { kind, .. } => edit_yaml_kind(*kind),
        }
        .filter(|kind| kind.is_editable())
        .map(ResourceAction::EditYaml),
        // ConfigMaps and Secrets only (spec 0047 decision 9); a Helm release row, which reads as a
        // Secret by storage, a Pod, and a custom kind have none.
        RowAction::EditValues => match subject {
            ResourceKey::Kind { kind, .. } => edit_values_kind(*kind),
            ResourceKey::Pod { .. } | ResourceKey::Node { .. } => None,
        }
        .map(ResourceAction::EditValues),
        // The kind table is the one source of which workload kinds offer the action.
        RowAction::RestartRollout
        | RowAction::Scale
        | RowAction::PauseRollout
        | RowAction::RollBack
        | RowAction::SuspendCronJob
        | RowAction::TriggerCronJob
        | RowAction::RerunJob
        | RowAction::EditHpaRange
        | RowAction::ExpandClaim
        | RowAction::SetDefaultStorageClass
        | RowAction::RenewCertificate => match subject {
            ResourceKey::Kind { kind, .. } => kind.read_only_actions().iter().find_map(|item| {
                item.action
                    .filter(|action| action.row_action() == Some(row))
            }),
            ResourceKey::Pod { .. } | ResourceKey::Node { .. } => None,
        },
        RowAction::CopyName => Some(ResourceAction::CopyName),
        RowAction::Delete => delete_kind(subject).map(ResourceAction::Delete),
    }
}

/// The kinds whose rows open the logs of their pods: View logs, key L.
fn has_workload_logs(kind: ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Deployments
            | ResourceKind::StatefulSets
            | ResourceKind::DaemonSets
            | ResourceKind::ReplicaSets
            | ResourceKind::Jobs
            | ResourceKind::CronJobs
    )
}

/// The kind Delete removes on a row of `subject`. A Helm release row reads as a Secret by storage,
/// so naming it a Secret would delete the wrong object; custom resources have no `ObjectKind`.
/// Both stay off (spec 0033 decision 26).
pub(crate) fn delete_kind(subject: &ResourceKey) -> Option<ObjectKind> {
    match subject {
        ResourceKey::Pod { .. } => Some(ObjectKind::Pod),
        ResourceKey::Node { .. } => Some(ObjectKind::Node),
        ResourceKey::Kind { kind, .. } => delete_kind_of(*kind),
    }
}

/// `delete_kind` for the rows of an explorer kind.
pub(crate) fn delete_kind_of(kind: ResourceKind) -> Option<ObjectKind> {
    if kind == ResourceKind::HelmReleases {
        return None;
    }
    kind.builtin_object()
}

/// What a key for `row` does on `subject`; menus and keys read the same gates.
pub(crate) fn key_availability(
    row: RowAction,
    subject: &ResourceKey,
    live: &LiveCluster,
    guard: &ClusterGuard<'_>,
) -> KeyAvailability {
    if let ResourceKey::Kind { kind, .. } = subject
        && subject_action(row, subject) == Some(ResourceAction::ViewLogs)
    {
        let found = live.kind_list(*kind).and_then(|explorer| {
            explorer
                .list
                .items()
                .iter()
                .find(|candidate| subject.is_row(*kind, candidate))
        });
        return workload_logs_key(found, live.pods.items(), guard.access);
    }
    let pod = live.pods.items().iter().find(|pod| subject.is_pod(pod));
    key_availability_of(row, subject, pod, guard)
}

/// What L does on a workload row: the gate of the logs first, then the row (`None` when its list
/// no longer holds it). Pure.
fn workload_logs_key(
    row: Option<&KindRow>,
    pods: &[PodSummary],
    access: &AccessState,
) -> KeyAvailability {
    if let ActionAvailability::Disabled { reason } =
        availability_before_lock(ResourceAction::ViewLogs, access)
    {
        return KeyAvailability::Disabled { reason };
    }
    match row.and_then(|row| workload_logs_owner(row, pods)) {
        None => KeyAvailability::NotOffered,
        Some(Ok(_)) => KeyAvailability::Run(ResourceAction::ViewLogs),
        Some(Err(reason)) => KeyAvailability::Disabled { reason },
    }
}

/// The pods the logs of a workload row merge: the Job of the last run for a CronJob (it owns no
/// pods itself), else the row's own pods. `None` for a row with no workload; `Err` says why a
/// CronJob has no job to read. Pure.
pub(crate) fn workload_logs_owner(
    row: &KindRow,
    pods: &[PodSummary],
) -> Option<Result<PodOwner, SharedString>> {
    if let KindObject::CronJob(cron_job) = &row.object {
        return Some(last_job_owner(cron_job, pods));
    }
    let owner = row.related_pods.clone()?;
    workload_label(&owner)?;
    Some(Ok(owner))
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
    if matches!(
        action,
        ResourceAction::Attach | ResourceAction::RestartPod | ResourceAction::EvictPod
    ) {
        // The gate first, then the pod: the reason the user can act on first comes first.
        return match (action_availability(action, guard), pod) {
            (ActionAvailability::Disabled { reason }, _) => KeyAvailability::Disabled { reason },
            (ActionAvailability::Enabled, None) => KeyAvailability::NotOffered,
            (ActionAvailability::Enabled, Some(pod)) => match pod_state_block(action, pod) {
                Some(reason) => KeyAvailability::Disabled { reason },
                None => KeyAvailability::Run(action),
            },
        };
    }
    // Drain always opens its dialog; a gated session gets the read-only preview (spec 0034).
    if action == ResourceAction::Drain {
        return KeyAvailability::Run(action);
    }
    match action_availability(action, guard) {
        ActionAvailability::Enabled => match subject {
            ResourceKey::Kind { kind, .. } => match kind_block(action, *kind) {
                Some(reason) => KeyAvailability::Disabled { reason },
                None => KeyAvailability::Run(action),
            },
            ResourceKey::Pod { .. } | ResourceKey::Node { .. } => KeyAvailability::Run(action),
        },
        ActionAvailability::Disabled { reason } => KeyAvailability::Disabled { reason },
    }
}

/// Why `kind` cannot take `action` whatever the row: Renew now reaches `cert-manager.io/v1` only,
/// so a cluster that serves another version of the CRD shows the item off (spec 0018 step 6).
/// `None` for every other pair. Pure.
pub(crate) fn kind_block(action: ResourceAction, kind: ResourceKind) -> Option<SharedString> {
    if action != ResourceAction::RenewCertificate {
        return None;
    }
    let custom = kind.custom()?;
    (custom.is_cert_manager_certificate() && !custom.is_cert_manager_v1())
        .then(|| NEEDS_CERT_MANAGER_V1.into())
}

/// The reason Renew now is off on a Certificate kind served at another version.
pub(crate) const NEEDS_CERT_MANAGER_V1: &str = "Needs cert-manager.io/v1";

/// The gate, in order: a mutating action whose spec has not shipped, then the permission state,
/// then the denied permission, then the cluster's read-only lock. A read-only action skips the
/// first and the last, so it needs no guard: see `availability_before_lock`. The first failing reason
/// is the one shown. Menus, keys, and the palette all read this function.
pub(crate) fn action_availability(
    action: ResourceAction,
    guard: &ClusterGuard<'_>,
) -> ActionAvailability {
    if let Some(reason) = node_shell_setting_block(action, guard) {
        return disabled(reason);
    }
    gate_availability(&action.gate(), guard)
}

/// Why the node shell is off for the cluster's own setting (spec 0037 decision 15): after the
/// shipped flag and the permissions, before the lock, so only a user who could open it is told that
/// the setting is what stops them.
fn node_shell_setting_block(
    action: ResourceAction,
    guard: &ClusterGuard<'_>,
) -> Option<SharedString> {
    if action != ResourceAction::OpenNodeShell || guard.profile.allow_node_shell {
        return None;
    }
    before_lock_reason(&action.gate(), guard.access, guard.kind_access)
        .is_none()
        .then(|| {
            format!(
                "Node shell is off for {} (Settings › Clusters › Safety)",
                guard.display_name()
            )
            .into()
        })
}

/// Why this node cannot take a node shell, `None` when it can: the shell is `nsenter` and `sh` on a
/// Linux host.
pub(crate) fn node_shell_block(node: &NodeSummary) -> Option<SharedString> {
    (!node.system.operating_system.eq_ignore_ascii_case("linux"))
        .then(|| "Node shell needs a Linux node".into())
}

/// Why this pod cannot take a debug container, `None` when it can: the container shares the
/// process namespace of a running one.
pub(crate) fn debug_container_block(pod: &PodSummary) -> Option<SharedString> {
    debug_targets(pod)
        .next()
        .is_none()
        .then(|| "The pod has no running container".into())
}

/// The containers a debug container can share a process namespace with: the running ones, init
/// containers excluded.
pub(crate) fn debug_targets(pod: &PodSummary) -> impl Iterator<Item = &ContainerSummary> {
    pod.containers
        .iter()
        .filter(|container| container.kind != ContainerKind::Init && is_running(container))
}

/// A static pod (the mirror pod of the kubelet) has a Node as its controller.
const STATIC_POD_CONTROLLER_KIND: &str = "Node";
/// The refusal both Restart pod and Evict give a static pod.
pub(crate) const STATIC_POD_TEXT: &str = "Static pod: the kubelet owns it";
/// The refusal of a pod whose deletion has started.
pub(crate) const ALREADY_TERMINATING_TEXT: &str = "Already terminating";

/// Why this pod cannot take `action` (Restart pod or Evict), `None` when it can. Pure: the menus,
/// the keys, the palette, and the start of the removal all read it.
pub(crate) fn pod_block(action: ResourceAction, pod: &PodSummary) -> Option<SharedString> {
    let is_restart = match action {
        ResourceAction::RestartPod => true,
        ResourceAction::EvictPod => false,
        _ => return None,
    };
    if is_restart && pod.controller.is_none() {
        return Some("Not managed by a controller; it would not come back. Use Delete pod…".into());
    }
    let is_static = pod
        .controller
        .as_ref()
        .is_some_and(|controller| controller.kind == STATIC_POD_CONTROLLER_KIND);
    if is_static {
        return Some(STATIC_POD_TEXT.into());
    }
    if pod.status == PodStatus::Terminating {
        return Some(ALREADY_TERMINATING_TEXT.into());
    }
    // A finished pod is not restarted by its Job or ReplicaSet; the eviction API deletes it
    // without a budget check, so Evict stays allowed.
    (is_restart && pod.is_finished)
        .then(|| "The pod has finished; its controller does not restart it".into())
}

/// What the state of the pod says against `action`, for the pod actions that read it.
fn pod_state_block(action: ResourceAction, pod: &PodSummary) -> Option<SharedString> {
    match action {
        ResourceAction::Attach => default_attach_container(pod).err(),
        _ => pod_block(action, pod),
    }
}

/// Why this container cannot be attached, `None` when it can: it must run, be a main or sidecar
/// container, and have both stdin and a tty, since the attach drives the terminal.
pub(crate) fn attach_block(container: &ContainerSummary) -> Option<SharedString> {
    if !is_running(container) {
        return Some(NOT_RUNNING_REASON.into());
    }
    if container.kind == ContainerKind::Init {
        return Some("Init containers cannot be attached".into());
    }
    (container.terminal == ContainerTerminal::None)
        .then(|| "The container has no terminal (stdin and tty); use View logs".into())
}

/// The container A attaches: the first attachable running main container, else the first
/// attachable running sidecar.
pub(crate) fn default_attach_container(
    pod: &PodSummary,
) -> Result<&ContainerSummary, SharedString> {
    let mut attachable = pod
        .containers
        .iter()
        .filter(|container| attach_block(container).is_none());
    let first = attachable.next();
    let main = first
        .filter(|container| container.kind == ContainerKind::Main)
        .or_else(|| attachable.find(|container| container.kind == ContainerKind::Main));
    main.or(first)
        .ok_or_else(|| "No running container has a terminal (stdin and tty); use View logs".into())
}

fn gate_availability(gate: &ActionGate, guard: &ClusterGuard<'_>) -> ActionAvailability {
    if let Some(reason) = before_lock_reason(gate, guard.access, guard.kind_access) {
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
    match before_lock_reason(&action.gate(), access, KindAccessMap::EMPTY) {
        Some(reason) => disabled(reason),
        None => ActionAvailability::Enabled,
    }
}

/// Rows 2-4 of the gate: why the action is off before the lock is even asked.
fn before_lock_reason(
    gate: &ActionGate,
    access: &AccessState,
    kind_access: &KindAccessMap,
) -> Option<SharedString> {
    match gate {
        ActionGate::Planned
        | ActionGate::Mutating {
            is_shipped: false, ..
        } => Some(NOT_SHIPPED_REASON.into()),
        // An action with no check is never held back by the permission state.
        ActionGate::ReadOnly { check } => {
            check.and_then(|check| permission_reason(&[check], access, kind_access))
        }
        ActionGate::Mutating { checks, .. } => permission_reason(checks, access, kind_access),
    }
}

/// Why the permissions do not allow the action: the state while they are not known, else the
/// first check of `checks` that is not allowed. A denied verb with its sibling verb on the same
/// resource in the list names both (`get and create pods/exec`), because the user needs both.
/// An `Update(kind)` or `Delete(kind)` check is answered by the lazy review of its kind
/// (`kind_access`, specs 0031 and 0033); the other checks by the session's report.
fn permission_reason(
    checks: &[AccessCheck],
    access: &AccessState,
    kind_access: &KindAccessMap,
) -> Option<SharedString> {
    let mut denied = None;
    for check in checks {
        let report = match check {
            AccessCheck::Update(kind)
            | AccessCheck::Delete(kind)
            | AccessCheck::Patch(kind)
            | AccessCheck::Create(kind) => match kind_access.get(*kind) {
                None | Some(KindAccess::Checking { .. }) => {
                    return Some("Checking permissions…".into());
                }
                Some(KindAccess::Unknown) => {
                    return Some("Permissions could not be checked".into());
                }
                Some(KindAccess::Known(report)) => report,
            },
            _ => match access {
                AccessState::Checking { .. } => return Some("Checking permissions…".into()),
                AccessState::Unknown => {
                    return Some("Permissions could not be checked".into());
                }
                AccessState::Known(report) => report,
            },
        };
        if denied.is_none() && !report.is_allowed(*check) {
            denied = Some(*check);
        }
    }
    Some(denied_text(denied?, checks).into())
}

/// The verb pairs that read as one right: a server before Kubernetes 1.35 asks for `get`, one after
/// asks for both.
const VERB_PAIRS: [(AccessCheck, AccessCheck, &str); 3] = [
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
    (
        AccessCheck::GetPodAttach,
        AccessCheck::CreatePodAttach,
        "pods/attach",
    ),
];

/// The start of the reason of an action the permissions do not allow.
pub(crate) const NOT_PERMITTED: &str = "Not permitted";

fn denied_text(denied: AccessCheck, checks: &[AccessCheck]) -> String {
    let pair = VERB_PAIRS.iter().find(|(get, create, _)| {
        (denied == *get || denied == *create) && checks.contains(get) && checks.contains(create)
    });
    match pair {
        Some((_, _, resource)) => format!("{NOT_PERMITTED}: get and create {resource}"),
        None => format!("{NOT_PERMITTED}: {denied}"),
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

/// The items of a pod menu that hold a submenu, built by the caller (`ShellMenu::item`,
/// `ForwardMenu::item`) because a submenu needs the app.
pub(crate) struct PodMenuItems {
    pub(crate) view_logs: PopupMenuItem,
    pub(crate) open_shell: PopupMenuItem,
    /// `Debug container…` when the Open shell item has no submenu to hold it (spec 0037).
    pub(crate) debug_container: Option<PopupMenuItem>,
    pub(crate) port_forward: PopupMenuItem,
}

/// The entries of the pod menu, in the order they are shown (W4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PodMenuEntry {
    ViewLogs,
    OpenShell,
    /// Absent when the Open shell item holds `Debug container…` in its submenu.
    DebugContainer,
    PortForward,
    Attach,
    EditYaml,
    ViewYaml,
    RestartPod,
    EvictPod,
    CopyName,
    CopyKubectlCommand,
    DeletePod,
    Separator,
}

const POD_MENU: [PodMenuEntry; 15] = [
    PodMenuEntry::ViewLogs,
    PodMenuEntry::OpenShell,
    PodMenuEntry::DebugContainer,
    PodMenuEntry::PortForward,
    PodMenuEntry::Attach,
    PodMenuEntry::Separator,
    PodMenuEntry::EditYaml,
    PodMenuEntry::ViewYaml,
    PodMenuEntry::RestartPod,
    PodMenuEntry::EvictPod,
    PodMenuEntry::Separator,
    PodMenuEntry::CopyName,
    PodMenuEntry::CopyKubectlCommand,
    PodMenuEntry::Separator,
    PodMenuEntry::DeletePod,
];

/// Shared by the row context menu and the drawer header menu, so both always agree.
pub(crate) fn pod_menu(
    menu: PopupMenu,
    pod: &PodSummary,
    guard: &ClusterGuard<'_>,
    row: &RowContext,
    links: &PodMenuLinks<'_>,
    items: PodMenuItems,
) -> PopupMenu {
    let shell = links.shell;
    let access = guard.access;
    // Each caller-built item is used once, at its entry.
    let PodMenuItems {
        view_logs,
        open_shell,
        debug_container,
        port_forward,
    } = items;
    let (mut view_logs, mut open_shell, mut port_forward) =
        (Some(view_logs), Some(open_shell), Some(port_forward));
    let mut debug_container = debug_container;
    POD_MENU.into_iter().fold(menu, |menu, entry| {
        let item = match entry {
            PodMenuEntry::Separator => return menu.separator(),
            PodMenuEntry::ViewLogs => view_logs.take(),
            PodMenuEntry::OpenShell => open_shell.take(),
            PodMenuEntry::DebugContainer => debug_container.take(),
            PodMenuEntry::PortForward => port_forward.take().map(|item| guarded(row, item)),
            PodMenuEntry::Attach => Some(attach_item(pod, guard)),
            PodMenuEntry::EditYaml => Some(action_item(
                ResourceAction::EditYaml(ObjectKind::Pod),
                guard,
            )),
            PodMenuEntry::ViewYaml => Some(guarded(
                row,
                view_yaml_item(row.object(ResourceKey::of_pod(pod)), shell),
            )),
            PodMenuEntry::RestartPod => {
                Some(pod_removal_item(ResourceAction::RestartPod, pod, guard))
            }
            PodMenuEntry::EvictPod => Some(pod_removal_item(ResourceAction::EvictPod, pod, guard)),
            PodMenuEntry::CopyName => Some(copy_name_item(&pod.name, access)),
            PodMenuEntry::CopyKubectlCommand => Some(copy_kubectl_command_item(&row.context, pod)),
            PodMenuEntry::DeletePod => Some(delete_item(
                DeleteLabel::of("Delete pod…", "pods"),
                action_availability(ResourceAction::Delete(ObjectKind::Pod), guard),
                shell,
            )),
        };
        match item {
            Some(item) => menu.item(item),
            None => menu,
        }
    })
}

/// The entries of the container ⋯ menu, in the order they are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerMenuEntry {
    ViewLogs,
    OpenShell,
    Attach,
    CopyImage,
}

const CONTAINER_MENU: [ContainerMenuEntry; 4] = [
    ContainerMenuEntry::ViewLogs,
    ContainerMenuEntry::OpenShell,
    ContainerMenuEntry::Attach,
    ContainerMenuEntry::CopyImage,
];

/// The ⋯ menu of the container detail (W4b note 3): View logs, Open shell, Attach, Copy image, all
/// for the shown container. No key hints: L, S, and A act on the pod's default container.
pub(crate) fn container_menu(
    menu: PopupMenu,
    pod: &PodSummary,
    container: &ContainerSummary,
    live: &LiveCluster,
    guard: &ClusterGuard<'_>,
    row: &RowContext,
    links: &PodMenuLinks<'_>,
) -> PopupMenu {
    CONTAINER_MENU
        .into_iter()
        .fold(menu, |menu, entry| match entry {
            ContainerMenuEntry::ViewLogs => menu.item(plain_logs_item(
                pod,
                Some(&container.name),
                live,
                row,
                links.dock,
            )),
            ContainerMenuEntry::OpenShell => menu.item(guarded(
                row,
                container_shell_item(pod, container, guard, row, links.shell),
            )),
            ContainerMenuEntry::Attach => menu.item(container_attach_item(
                pod,
                container,
                guard,
                row,
                links.shell,
            )),
            ContainerMenuEntry::CopyImage => {
                menu.separator().item(copy_image_item(&container.image))
            }
        })
}

/// Open shell on one named container, through the guarded flow of `start_shell`.
fn container_shell_item(
    pod: &PodSummary,
    container: &ContainerSummary,
    guard: &ClusterGuard<'_>,
    row: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let label = action_label(ResourceAction::OpenShell);
    let item = match container_shell_availability(container, guard) {
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
        ActionAvailability::Enabled => {
            let open = ShellOpen {
                cluster: row.cluster.clone(),
                namespace: pod.namespace.clone(),
                pod: pod.name.clone(),
                short_pod: pod_tab_name(pod),
                container: container.name.clone(),
            };
            let shell = shell.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let open = open.clone();
                let _ = shell.update(cx, |shell, cx| shell.start_shell(open, window, cx));
            })
        }
    };
    item.menu_icon(RowAction::OpenShell.icon())
}

/// Attach to one named container, through the guarded flow of `start_attach`. The item acts only
/// while the session `row` was built on is still open (`guarded`), so a menu left open over a
/// switch does nothing.
pub(crate) fn container_attach_item(
    pod: &PodSummary,
    container: &ContainerSummary,
    guard: &ClusterGuard<'_>,
    row: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let label = action_label(ResourceAction::Attach);
    let item = match container_attach_availability(container, guard) {
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
        ActionAvailability::Enabled => {
            let open = ShellOpen {
                cluster: row.cluster.clone(),
                namespace: pod.namespace.clone(),
                pod: pod.name.clone(),
                short_pod: pod_tab_name(pod),
                container: container.name.clone(),
            };
            let (terminal, shell) = (container.terminal, shell.clone());
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let open = open.clone();
                let _ = shell.update(cx, |shell, cx| {
                    shell.start_attach(open, terminal, window, cx)
                });
            })
        }
    };
    guarded(row, item.menu_icon(RowAction::Attach.icon()))
}

/// The gate of the session, then the container: one that is not running, is an init container, or
/// has no terminal cannot be attached.
pub(crate) fn container_attach_availability(
    container: &ContainerSummary,
    guard: &ClusterGuard<'_>,
) -> ActionAvailability {
    match action_availability(ResourceAction::Attach, guard) {
        ActionAvailability::Enabled => match attach_block(container) {
            Some(reason) => ActionAvailability::Disabled { reason },
            None => ActionAvailability::Enabled,
        },
        disabled => disabled,
    }
}

/// Restart pod or Evict in a pod menu: the gate decides first, then the state of the pod. Neither
/// has a click handler: the menu dispatches the unit action, which runs on the cursor pod.
fn pod_removal_item(
    action: ResourceAction,
    pod: &PodSummary,
    guard: &ClusterGuard<'_>,
) -> PopupMenuItem {
    let label = action_label(action);
    let availability = match action_availability(action, guard) {
        ActionAvailability::Enabled => match pod_block(action, pod) {
            Some(reason) => ActionAvailability::Disabled { reason },
            None => ActionAvailability::Enabled,
        },
        disabled => disabled,
    };
    let item = match availability {
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
        ActionAvailability::Enabled => PopupMenuItem::new(label),
    };
    keyed(item, action)
}

/// The Attach item of a pod menu. It has no `on_click`: the menu dispatches the key action, which
/// runs on the cursor row.
fn attach_item(pod: &PodSummary, guard: &ClusterGuard<'_>) -> PopupMenuItem {
    let label = action_label(ResourceAction::Attach);
    let availability = match action_availability(ResourceAction::Attach, guard) {
        ActionAvailability::Enabled => match default_attach_container(pod) {
            Ok(_) => ActionAvailability::Enabled,
            Err(reason) => ActionAvailability::Disabled { reason },
        },
        disabled => disabled,
    };
    let item = match availability {
        ActionAvailability::Enabled => PopupMenuItem::new(label),
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
    };
    row_keyed(item, RowAction::Attach)
}

/// The gate of the session, then the container: one that is not running has no shell.
pub(crate) fn container_shell_availability(
    container: &ContainerSummary,
    guard: &ClusterGuard<'_>,
) -> ActionAvailability {
    match action_availability(ResourceAction::OpenShell, guard) {
        ActionAvailability::Enabled if !is_running(container) => disabled(NOT_RUNNING_REASON),
        availability => availability,
    }
}

fn copy_image_item(image: &str) -> PopupMenuItem {
    let image = image.to_owned();
    PopupMenuItem::new("Copy image")
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(image.clone()));
        })
        .menu_icon(IconName::Copy)
}

/// Copies the read-only `kubectl describe` command for the pod.
fn copy_kubectl_command_item(context: &str, pod: &PodSummary) -> PopupMenuItem {
    let command = kubectl_describe_command(context, &pod.namespace, &pod.name);
    PopupMenuItem::new("Copy kubectl command")
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
        })
        .menu_icon(IconName::Terminal)
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
    row_keyed(
        plain_logs_item(pod, container, live, row, dock),
        RowAction::ViewLogs,
    )
}

/// `view_logs_item` without the key hint, for a menu whose item names a container: L opens the
/// pod's default container, so a hint there would lie.
fn plain_logs_item(
    pod: &PodSummary,
    container: Option<&str>,
    live: &LiveCluster,
    row: &RowContext,
    dock: &WeakEntity<Dock>,
) -> PopupMenuItem {
    let label = action_label(ResourceAction::ViewLogs);
    let item = match logs_launch(pod, container, &live.access) {
        Err(reason) => disabled_menu_item(label, reason),
        Ok(target) => {
            let open = open_logs(live.connection().clone(), row.clone(), dock.clone());
            PopupMenuItem::new(label)
                .on_click(move |_, window, cx| open(target.clone(), window, cx))
        }
    };
    item.menu_icon(RowAction::ViewLogs.icon())
}

/// The call every View logs entry makes: opens `target` in the dock under the row's origin, while
/// the session the row was built on is still open (spec 0046 decision 5).
fn open_logs(
    connection: ClusterConnection,
    row: RowContext,
    dock: WeakEntity<Dock>,
) -> impl Fn(LogTarget, &mut Window, &mut App) + Clone {
    move |target, window, cx| {
        if row.session.upgrade().is_none() {
            return;
        }
        let _ = dock.update(cx, |dock, cx| {
            let origin = LogOrigin::new(&row, connection.clone());
            dock.open(origin, target, window, cx)
        });
    }
}

/// One container of the View logs submenu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogChoice {
    pub(crate) name: String,
    /// `MAIN`, `SIDECAR`, or `INIT`.
    pub(crate) tag: &'static str,
}

/// What the View logs item of a pod offers.
pub(crate) enum LogsMenuState {
    /// Logs are not permitted, or the pod has no container.
    Disabled(SharedString),
    /// A pod with one container opens it directly.
    One(LogTarget),
    /// Several containers: a submenu with every one, init included, none disabled for its state.
    Pick(Vec<LogChoice>),
}

/// The View logs item of a pod menu, owned so the caller builds it before it borrows the session
/// (a submenu is built from the app, like `ShellMenu`).
pub(crate) struct LogsMenu {
    state: LogsMenuState,
    pod: PodSummary,
}

impl LogsMenu {
    pub(crate) fn of(pod: &PodSummary, access: &AccessState) -> Self {
        let state = match logs_launch(pod, None, access) {
            Err(reason) => LogsMenuState::Disabled(reason),
            Ok(target) if pod.containers.len() < 2 => LogsMenuState::One(target),
            Ok(_) => LogsMenuState::Pick(
                pod.containers
                    .iter()
                    .map(|container| LogChoice {
                        name: container.name.clone(),
                        tag: kind_tag_text(container.kind),
                    })
                    .collect(),
            ),
        };
        Self {
            state,
            pod: pod.clone(),
        }
    }

    /// Each entry opens its own container in the dock; a single container keeps the key hint.
    pub(crate) fn item(
        self,
        connection: ClusterConnection,
        row: &RowContext,
        dock: &WeakEntity<Dock>,
        window: &mut Window,
        cx: &mut App,
    ) -> PopupMenuItem {
        let label = action_label(ResourceAction::ViewLogs);
        let open = open_logs(connection, row.clone(), dock.clone());
        match self.state {
            LogsMenuState::Disabled(reason) => {
                row_keyed(disabled_menu_item(label, reason), RowAction::ViewLogs)
            }
            LogsMenuState::One(target) => row_keyed(
                PopupMenuItem::new(label)
                    .on_click(move |_, window, cx| open(target.clone(), window, cx)),
                RowAction::ViewLogs,
            ),
            LogsMenuState::Pick(choices) => {
                let pod = self.pod;
                let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
                    choices.iter().fold(submenu, |submenu, choice| {
                        let open = open.clone();
                        let target = LogTarget::of_container(&pod, &choice.name);
                        let text = format!("{} · {}", choice.name, choice.tag);
                        submenu.item(PopupMenuItem::new(text).on_click(move |_, window, cx| {
                            if let Some(target) = target.clone() {
                                open(target, window, cx);
                            }
                        }))
                    })
                });
                row_keyed(PopupMenuItem::submenu(label, submenu), RowAction::ViewLogs)
            }
        }
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
    Some((
        label,
        availability_before_lock(ResourceAction::ViewLogs, access),
    ))
}

/// The label of the logs item of a CronJob row, which owns no pods itself.
const LAST_JOB_LOGS_LABEL: &str = "View logs of last job";

/// Merges the logs of every pod of the workload into one dock tab; a CronJob opens its last job.
/// Both carry the L hint, since the key does the same on the cursor row.
fn workload_logs_item(
    row: &KindRow,
    access: &AccessState,
    pods: &[PodSummary],
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> Option<PopupMenuItem> {
    let label = match &row.object {
        KindObject::CronJob(_) => LAST_JOB_LOGS_LABEL,
        _ => workload_logs_entry(row.related_pods.as_ref(), access)?.0,
    };
    let owner = workload_logs_owner(row, pods)?;
    let gate = availability_before_lock(ResourceAction::ViewLogs, access);
    let item = match (gate, owner) {
        (ActionAvailability::Disabled { reason }, _) | (_, Err(reason)) => {
            disabled_menu_item(label, reason)
        }
        (ActionAvailability::Enabled, Ok(owner)) => {
            let cluster = context.cluster.clone();
            let shell = shell.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.open_workload_logs(&cluster, owner.clone(), window, cx);
                });
            })
        }
    };
    Some(row_keyed(item, RowAction::ViewLogs))
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
    menu.item(open_node_shell_item(node, guard))
        .item(guarded(row, cordon_item(node, guard, row, shell)))
        .item(drain_item(guard))
        .separator()
        .item(action_item(ResourceAction::EditTaints, guard))
        .item(action_item(ResourceAction::EditLabels, guard))
        .item(guarded(
            row,
            view_pods_on_node_item(node, live.pods.items(), shell),
        ))
        .item(guarded(
            row,
            view_yaml_item(row.object(ResourceKey::of_node(node)), shell),
        ))
        .separator()
        .item(copy_name_item(&node.name, access))
        .separator()
        .item(delete_item(
            DeleteLabel::of("Delete node…", "nodes"),
            action_availability(ResourceAction::Delete(ObjectKind::Node), guard),
            shell,
        ))
}

/// Open node shell, with the reason a node cannot take it (a Windows node) after the gate's own.
/// It has no `on_click`: the menu dispatches the key action, which runs on the cursor row.
fn open_node_shell_item(node: &NodeSummary, guard: &ClusterGuard<'_>) -> PopupMenuItem {
    let label = action_label(ResourceAction::OpenNodeShell);
    let availability = match action_availability(ResourceAction::OpenNodeShell, guard) {
        ActionAvailability::Enabled => match node_shell_block(node) {
            Some(reason) => ActionAvailability::Disabled { reason },
            None => ActionAvailability::Enabled,
        },
        disabled => disabled,
    };
    let item = match availability {
        ActionAvailability::Enabled => PopupMenuItem::new(label),
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
    };
    row_keyed(item, RowAction::OpenShell)
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
    .menu_icon(IconName::List)
}

/// How many of `pods` are scheduled on `node`.
fn pods_on_node(pods: &[PodSummary], node: &str) -> usize {
    pods.iter()
        .filter(|pod| pod.node_name.as_deref() == Some(node))
        .count()
}

/// The items a release adds before its disabled ones: each opens the drawer on a Helm tab.
const HELM_VIEW_ITEMS: [(&str, DrawerTab, IconName); 2] = [
    ("View values", DrawerTab::Values, IconName::FileText),
    ("View manifest", DrawerTab::Manifest, IconName::FileCode),
];

/// The cluster a kind menu is built for: the guard, the pod list, and the row context of the row's
/// own slot.
pub(crate) struct MenuCluster<'a> {
    pub(crate) guard: &'a ClusterGuard<'a>,
    pub(crate) pods: &'a [PodSummary],
    pub(crate) context: &'a RowContext,
    /// The ReplicaSets of the row, once its drawer has loaded them (`loaded_replica_sets`): Roll
    /// back stays off without them.
    pub(crate) replica_sets: Option<&'a [ReplicaSetSummary]>,
}

/// The row context menu and the drawer ⋯ menu of an explorer kind. The change items of a kind go
/// through `row_action_item`: gated, then offered only when this row can take them; the ones whose
/// spec has not shipped stay disabled.
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
        replica_sets,
    } = *cluster;
    let access = guard.access;
    let mut menu = menu;
    if has_who_can(kind) {
        menu = menu.item(guarded(context, who_can_item(row, context, shell)));
    }
    if has_check_permissions(kind) {
        menu = menu.item(guarded(
            context,
            check_permissions_item(row, context, shell),
        ));
    }
    if has_test_traffic(kind) {
        menu = menu.item(guarded(context, test_traffic_item(row, context, shell)));
    }
    if let Some(item) = workload_logs_item(row, access, pods, context, shell) {
        menu = menu.item(guarded(context, item));
    }
    if let Some(secret) = extras.secret {
        for item in secret.into_items() {
            menu = menu.item(item);
        }
        menu = menu.separator();
    }
    if let Some(event) = &row.event {
        menu = menu
            .item(guarded(context, go_to_object_item(event, context, shell)))
            .item(guarded(context, filter_similar_item(event, shell)))
            .item(copy_message_item(event))
            .separator();
    }
    if let Some(browse) = extras.browse {
        menu = menu.item(guarded(context, browse));
    }
    let key = ResourceKey::of_row(kind, row);
    let object = context.object(key.clone());
    // A key without an object reference (a Helm release) has no YAML tab.
    if kind == ResourceKind::HelmReleases {
        for (label, tab, icon) in HELM_VIEW_ITEMS {
            menu = menu.item(guarded(
                context,
                view_tab_item(label, object.clone(), tab, shell).menu_icon(icon),
            ));
        }
    }
    if let Some(item) = show_section_item(row, object.clone(), shell) {
        menu = menu.item(guarded(context, item));
    }
    if object_ref(&key).is_some() {
        menu = menu.item(guarded(context, view_yaml_item(object, shell)));
    }
    if let Some(item) = extras.open_url {
        menu = menu.item(item);
    }
    match topology_menu(kind, row.namespace.as_deref(), extras.scope.as_ref()) {
        TopologyMenu::Hidden => {}
        TopologyMenu::Enabled => {
            menu = menu.item(guarded(context, show_in_topology_item(key.clone(), shell)));
        }
        TopologyMenu::Disabled(reason) => {
            menu = menu.item(
                disabled_menu_item("Show in Topology", reason.into())
                    .menu_icon(IconName::Waypoints),
            );
        }
    }
    if has_go_to_target(kind) {
        menu = menu.item(guarded(context, go_to_target_item(row, context, shell)));
    }
    if has_go_to_owner(kind) {
        menu = menu.item(guarded(context, go_to_owner_item(row, context, shell)));
    }
    match kind {
        ResourceKind::PersistentVolumeClaims => {
            menu = menu.item(guarded(context, go_to_pod_item(row, pods, context, shell)));
        }
        ResourceKind::PersistentVolumes => {
            menu = menu.item(guarded(context, go_to_claim_item(row, context, shell)));
        }
        ResourceKind::RoleBindings | ResourceKind::ClusterRoleBindings => {
            menu = menu.item(guarded(context, go_to_role_item(row, context, shell)));
        }
        _ => {}
    }
    if let Some(item) = extras.port_forward {
        menu = menu.item(guarded(context, item));
    }
    if let Some(is_default) =
        default_namespace_state(kind, &row.name, extras.default_namespace.as_deref())
    {
        menu = menu.item(guarded(
            context,
            default_namespace_item(context.cluster.clone(), row.name.clone(), is_default, shell),
        ));
    }
    let change_actions = kind.read_only_actions();
    let edit_yaml = edit_yaml_kind(kind);
    if !change_actions.is_empty() || edit_yaml.is_some() {
        menu = menu.separator();
    }
    for item in change_actions {
        menu = menu.item(match item.action {
            Some(action) => match (action_availability(action, guard), kind_block(action, kind)) {
                // The gate first, then the kind: the reason the user can act on first.
                (ActionAvailability::Enabled, Some(reason)) => {
                    keyed(disabled_menu_item(item.label, reason), action)
                }
                _ => row_action_item(item.label, action, guard, &row.object, replica_sets),
            },
            None => disabled_menu_item(item.label, NOT_SHIPPED_REASON.into()),
        });
    }
    if let Some(object) = edit_yaml {
        menu = menu.item(action_item(ResourceAction::EditYaml(object), guard));
    }
    menu.separator()
        .item(copy_name_item(&row.name, access))
        .separator()
        .item(kind_delete_item(kind, row, guard, shell))
}

/// The Delete item of an explorer kind's menu. Helm releases and custom kinds keep it off with the
/// not-shipped reason (decision 26), and a Secret that stores a Helm release says why it stays.
fn kind_delete_item(
    kind: ResourceKind,
    row: &KindRow,
    guard: &ClusterGuard<'_>,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let label = kind.delete_label();
    let Some(object) = delete_kind_of(kind) else {
        return row_keyed(
            disabled_menu_item(label, NOT_SHIPPED_REASON.into()),
            RowAction::Delete,
        );
    };
    let availability = match helm_record_reason(&row.object) {
        Some(reason) => ActionAvailability::Disabled { reason },
        None => action_availability(ResourceAction::Delete(object), guard),
    };
    delete_item(DeleteLabel::of(label, kind.plural()), availability, shell)
}

/// Why a Secret row cannot be deleted: it stores a Helm release.
pub(crate) fn helm_record_reason(object: &KindObject) -> Option<SharedString> {
    match object {
        KindObject::Secret(secret) if secret.secret_type == HELM_RELEASE_SECRET_TYPE => {
            Some(HELM_RECORD_REASON.into())
        }
        _ => None,
    }
}

/// The Helm storage label of a ConfigMap or Secret that holds a release record (both drivers).
const HELM_OWNER_TERM: &str = "owner=helm";

/// Why the values of this ConfigMap or Secret row cannot be edited, from its summary (spec 0047):
/// a Helm release record, a service account token, or an immutable object. The menu and the palette
/// show it; `values_base` refuses the same objects again from the server's answer.
pub(crate) fn values_edit_block(object: &KindObject) -> Option<SharedString> {
    let has_helm_label = |labels: &[String]| labels.iter().any(|label| label == HELM_OWNER_TERM);
    match object {
        KindObject::Secret(secret) => {
            if secret.secret_type == HELM_RELEASE_SECRET_TYPE || has_helm_label(&secret.labels) {
                Some("Helm release records cannot be edited".into())
            } else if matches!(secret.details, SecretDetails::ServiceAccountToken { .. }) {
                Some("Service account tokens are managed by Kubernetes".into())
            } else if secret.is_immutable {
                Some("Immutable Secret".into())
            } else {
                None
            }
        }
        KindObject::ConfigMap(config_map) => {
            if has_helm_label(&config_map.labels) {
                Some("Helm release records cannot be edited".into())
            } else if config_map.is_immutable {
                Some("Immutable ConfigMap".into())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The two labels of a Delete item: for one object, and for a count of them (`Delete 12 pods…`).
struct DeleteLabel {
    single: &'static str,
    plural: &'static str,
}

impl DeleteLabel {
    fn of(single: &'static str, plural: &'static str) -> Self {
        Self { single, plural }
    }
}

/// The red last item of a row menu. It has no `on_click`: it dispatches the Del key action, which
/// runs on the cursor row, or on the ticked set when the cursor row is one of several (spec 0033
/// decision 27). So the label counts what Del would delete, read when the menu draws.
fn delete_item(
    label: DeleteLabel,
    availability: ActionAvailability,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let shell = shell.clone();
    let is_disabled = matches!(availability, ActionAvailability::Disabled { .. });
    let item = PopupMenuItem::element(move |_, cx| {
        let count = shell
            .read_with(cx, |shell, cx| shell.delete_scope_size(cx))
            .unwrap_or(1);
        let text = if count >= 2 {
            format!("Delete {count} {}…", label.plural)
        } else {
            label.single.to_owned()
        };
        let theme = cx.theme();
        match &availability {
            ActionAvailability::Enabled => div()
                .text_color(theme.danger)
                .child(text)
                .into_any_element(),
            ActionAvailability::Disabled { reason } => disabled_label(text.into(), reason.clone()),
        }
    })
    .disabled(is_disabled);
    row_keyed(item, RowAction::Delete)
}

/// Makes `item` act only while the session `row` was built on is still open. A menu that stays
/// open over a switch must do nothing: after A to B the guard of A is gone, and after A to B to A
/// the old session is released too, so the new session of A is not the one the menu showed (spec
/// 0046 decision 5). An item without a click handler dispatches a key action to the cursor row,
/// which a switch clears.
fn guarded(row: &RowContext, mut item: PopupMenuItem) -> PopupMenuItem {
    if let PopupMenuItem::Item {
        handler: Some(handler),
        ..
    }
    | PopupMenuItem::ElementItem {
        handler: Some(handler),
        ..
    } = &mut item
    {
        let (inner, session) = (Rc::clone(handler), row.session.clone());
        *handler = Rc::new(move |event, window, cx| {
            if session.upgrade().is_some() {
                inner(event, window, cx);
            }
        });
    }
    item
}

/// Whether the menu of a kind offers Show in Topology.
#[derive(Debug, PartialEq, Eq)]
enum TopologyMenu {
    /// Only Services and Ingresses have the item.
    Hidden,
    Enabled,
    /// The namespace is outside the session's scope, so Topology could not draw it.
    Disabled(String),
}

fn topology_menu(
    kind: ResourceKind,
    namespace: Option<&str>,
    scope: Option<&NamespaceScope>,
) -> TopologyMenu {
    if !matches!(kind, ResourceKind::Services | ResourceKind::Ingresses) {
        return TopologyMenu::Hidden;
    }
    let (Some(namespace), Some(scope)) = (namespace, scope) else {
        return TopologyMenu::Hidden;
    };
    if scope_includes(scope, namespace) {
        TopologyMenu::Enabled
    } else {
        TopologyMenu::Disabled(format!("Namespace {namespace} is outside the scope"))
    }
}

fn show_in_topology_item(key: ResourceKey, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    let shell = shell.clone();
    PopupMenuItem::new("Show in Topology")
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.show_in_topology(&key, cx));
        })
        .menu_icon(IconName::Waypoints)
}

/// A menu item that opens the drawer scrolled to one of its sections: the label, the section title,
/// and why the item is off, if it is.
#[derive(Debug, PartialEq, Eq)]
struct ShowSection {
    label: &'static str,
    title: &'static str,
    block: Option<&'static str>,
}

/// The Show item of a Namespace (its remaining resources, only while it is Terminating) or of a
/// PDB (its selected pods); `None` for every other kind.
fn show_section(row: &KindRow) -> Option<ShowSection> {
    match &row.object {
        KindObject::Namespace(namespace) => Some(ShowSection {
            label: "Show remaining resources",
            title: REMAINING_RESOURCES_TITLE,
            block: (namespace.phase != NamespacePhase::Terminating)
                .then_some("The namespace is not terminating"),
        }),
        KindObject::PodDisruptionBudget(_) => Some(ShowSection {
            label: "Show selected pods",
            title: SELECTED_PODS_TITLE,
            block: None,
        }),
        _ => None,
    }
}

fn show_section_item(
    row: &KindRow,
    object: ClusterObject,
    shell: &WeakEntity<AppShell>,
) -> Option<PopupMenuItem> {
    let ShowSection {
        label,
        title,
        block,
    } = show_section(row)?;
    if let Some(reason) = block {
        return Some(disabled_menu_item(label, reason.into()).menu_icon(IconName::List));
    }
    let shell = shell.clone();
    Some(
        PopupMenuItem::new(label)
            .on_click(move |_, _, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.open_drawer_section(object.clone(), title, cx)
                });
            })
            .menu_icon(IconName::List),
    )
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
        return Some(
            disabled_menu_item(LABEL, "Not established or not served".into())
                .menu_icon(IconName::List),
        );
    };
    let shell = shell.clone();
    Some(
        PopupMenuItem::new(LABEL)
            .on_click(move |_, _, cx| {
                let screen = Screen::Kind(ResourceKind::Custom(kind));
                let _ = shell.update(cx, |shell, cx| shell.show_screen(screen, cx));
            })
            .menu_icon(IconName::List),
    )
}

/// What the caller supplies beyond the row and the access state: items that need the window or
/// the app to be built (so the caller builds them before it borrows the session; a submenu needs
/// the app mutably), and data the menu reads, such as the default namespace.
#[derive(Default)]
pub(crate) struct MenuExtras {
    pub(crate) open_url: Option<PopupMenuItem>,
    /// The Port-forward item of a Service, Deployment, or StatefulSet row.
    pub(crate) port_forward: Option<PopupMenuItem>,
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

impl SecretMenu {
    /// Reveal, then the Copy submenu, in menu order.
    pub(crate) fn into_items(self) -> [PopupMenuItem; 2] {
        [self.reveal, self.copy]
    }
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
    context: &RowContext,
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
            context,
            shell,
        ),
        MenuState::Disabled(reason) => disabled_menu_item("Reveal values (30s)", reason.into()),
    }
    .menu_icon(IconName::Eye);
    let (shell, context) = (shell.clone(), context.clone());
    let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
        model.copies.iter().fold(submenu, |submenu, entry| {
            let item = match (&entry.state, &entry.key) {
                (MenuState::Enabled, Some(name)) => secret_action_item(
                    entry.label.clone().into(),
                    object.clone(),
                    SecretAction::Copy(name.clone()),
                    &context,
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
        copy: PopupMenuItem::submenu("Copy value", submenu).menu_icon(IconName::Copy),
    })
}

/// A Reveal or Copy item: it opens the drawer of `object` and runs `action` through the shell,
/// while the session `context` was built on is still open.
fn secret_action_item(
    label: SharedString,
    object: ClusterObject,
    action: SecretAction,
    context: &RowContext,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    let shell = shell.clone();
    guarded(
        context,
        PopupMenuItem::new(label).on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.run_secret_action(object.clone(), action.clone(), cx);
            });
        }),
    )
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
    row: &RowContext,
    window: &mut Window,
    cx: &mut App,
) -> PopupMenuItem {
    // The kit wires the parent of `PopupMenuItem::submenu` in `PopupMenu::render`, so `build` is the
    // sanctioned path from the table context menu.
    const LABEL: &str = "Open URL";
    let item = match choice {
        OpenUrl::Unavailable => disabled_menu_item(LABEL, "No host to open".into()),
        OpenUrl::One(url) => open_url_item(LABEL, url, row),
        OpenUrl::Several(urls) => {
            let row = row.clone();
            let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
                urls.iter().fold(submenu, |submenu, url| {
                    submenu.item(open_url_item(url.clone(), url.clone(), &row))
                })
            });
            PopupMenuItem::submenu(LABEL, submenu)
        }
    };
    item.menu_icon(IconName::ExternalLink)
}

fn open_url_item(label: impl Into<SharedString>, url: String, row: &RowContext) -> PopupMenuItem {
    guarded(
        row,
        PopupMenuItem::new(label).on_click(move |_, _, cx| cx.open_url(&url)),
    )
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
        return disabled_menu_item(label, reason).menu_icon(IconName::CornerDownRight);
    };
    let object = context.object(key);
    let shell = shell.clone();
    PopupMenuItem::new(label)
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.reveal_object(object.clone(), cx));
        })
        .menu_icon(IconName::CornerDownRight)
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
        return disabled_menu_item(LABEL, "No screen for this kind yet".into())
            .menu_icon(IconName::CornerDownRight);
    };
    let object = context.object(key);
    let shell = shell.clone();
    PopupMenuItem::new(LABEL)
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.reveal_object(object.clone(), cx));
        })
        .menu_icon(IconName::CornerDownRight)
}

/// Filters the Events list to the reason of this event; disabled when it has none.
fn filter_similar_item(event: &EventDetail, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    const LABEL: &str = "Filter similar";
    let Some(reason) = similar_reason(event) else {
        return disabled_menu_item(LABEL, "This event has no reason".into())
            .menu_icon(IconName::ListFilter);
    };
    let reason = reason.clone();
    let shell = shell.clone();
    PopupMenuItem::new(LABEL)
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.filter_similar(&reason, cx));
        })
        .menu_icon(IconName::ListFilter)
}

/// What Filter similar matches on: the reason, when the event has one.
fn similar_reason(event: &EventDetail) -> Option<&SharedString> {
    event.reason.as_ref().filter(|reason| !reason.is_empty())
}

fn copy_message_item(event: &EventDetail) -> PopupMenuItem {
    let message = event.message.clone();
    PopupMenuItem::new("Copy message")
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(message.to_string()));
        })
        .menu_icon(IconName::Copy)
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
    PopupMenuItem::new("Test traffic…")
        .on_click(move |_, window, cx| {
            let policy = policy.clone();
            let _ = shell.update(cx, |shell, cx| {
                shell.open_traffic_test(&cluster, policy.as_ref(), true, window, cx);
            });
        })
        .menu_icon(IconName::Activity)
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
    PopupMenuItem::new("Check permissions")
        .on_click(move |_, window, cx| {
            let subject = subject.clone();
            let namespace = namespace.clone();
            let _ = shell.update(cx, |shell, cx| {
                shell.open_permissions(&cluster, subject, namespace, true, window, cx);
            });
        })
        .menu_icon(IconName::ShieldQuestionMark)
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
    PopupMenuItem::new("Who can…")
        .on_click(move |_, window, cx| {
            let query = query.clone();
            let namespace = namespace.clone();
            let _ = shell.update(cx, |shell, cx| {
                let check_now = query.is_some();
                shell.open_who_can(&cluster, query, namespace, check_now, window, cx);
            });
        })
        .menu_icon(IconName::UserSearch)
}

/// Opens the drawer of `object` on its YAML tab. Always enabled: a missing right shows inline
/// there.
fn view_yaml_item(object: ClusterObject, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    row_keyed(
        view_tab_item(
            action_label(ResourceAction::ViewYaml),
            object,
            DrawerTab::Yaml,
            shell,
        ),
        RowAction::ViewYaml,
    )
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
    let choices = shell_choices(pod);
    match choices.as_slice() {
        [] => ShellMenuState::Disabled("The pod has no containers".into()),
        [only] if only.is_running => ShellMenuState::One(only.name.clone()),
        [_] => ShellMenuState::Disabled(NOT_RUNNING_REASON.into()),
        _ if choices.iter().any(|choice| choice.is_running) => ShellMenuState::Pick(choices),
        _ => ShellMenuState::Disabled("No running container".into()),
    }
}

/// The containers a shell can be asked for: every one but the init containers.
fn shell_choices(pod: &PodSummary) -> Vec<ShellChoice> {
    pod.containers
        .iter()
        .filter(|container| container.kind != ContainerKind::Init)
        .map(|container| ShellChoice {
            name: container.name.clone(),
            tag: kind_tag_text(container.kind),
            is_running: is_running(container),
        })
        .collect()
}

/// What S (and the palette's Open shell) does on a pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DefaultShell {
    /// A pod with one container, or no choice to make: this container.
    Open(String),
    /// Several containers: the picker first, as the Open shell submenu offers.
    Pick(Vec<ShellChoice>),
    Unavailable,
}

/// The same split as the Open shell submenu, so S and the menu never disagree. Pure.
pub(crate) fn default_shell(pod: &PodSummary) -> DefaultShell {
    let choices = shell_choices(pod);
    if choices.len() > 1 && choices.iter().any(|choice| choice.is_running) {
        return DefaultShell::Pick(choices);
    }
    match default_shell_container(pod) {
        Some(container) => DefaultShell::Open(container.name.clone()),
        None => DefaultShell::Unavailable,
    }
}

const NOT_RUNNING_REASON: &str = "Container is not running; see Previous logs or Debug container";

/// What the Debug container… item of a pod offers (spec 0037).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DebugMenuState {
    Disabled(SharedString),
    Ready,
}

/// The gate of the pod's own cluster, then the state of this pod. Pure.
pub(crate) fn debug_menu_state(pod: &PodSummary, guard: &ClusterGuard<'_>) -> DebugMenuState {
    if let ActionAvailability::Disabled { reason } =
        action_availability(ResourceAction::DebugContainer, guard)
    {
        return DebugMenuState::Disabled(reason);
    }
    match debug_container_block(pod) {
        Some(reason) => DebugMenuState::Disabled(reason),
        None => DebugMenuState::Ready,
    }
}

/// The Open shell item of a pod menu, with everything it needs owned: a submenu is built from the
/// app, so a caller makes this before it borrows the session (like `secret_menu`).
pub(crate) struct ShellMenu {
    state: ShellMenuState,
    debug: DebugMenuState,
    namespace: String,
    pod: String,
    short_pod: String,
}

/// The shell items of a pod menu: Open shell, and `Debug container…` when Open shell has no
/// submenu to hold it (a pod with one container, or no shell available).
pub(crate) struct ShellItems {
    pub(crate) open_shell: PopupMenuItem,
    pub(crate) debug_container: Option<PopupMenuItem>,
}

impl ShellMenu {
    pub(crate) fn of(pod: &PodSummary, guard: &ClusterGuard<'_>) -> Self {
        Self {
            state: shell_menu_state(pod, guard),
            debug: debug_menu_state(pod, guard),
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            short_pod: pod_tab_name(pod),
        }
    }

    /// Each entry opens its own container in the row's cluster, through the guarded flow. The
    /// submenu of a pod with several containers ends with `Debug container…` after a separator.
    pub(crate) fn items(
        self,
        row: &RowContext,
        shell: &WeakEntity<AppShell>,
        window: &mut Window,
        cx: &mut App,
    ) -> ShellItems {
        let label = action_label(ResourceAction::OpenShell);
        let Self {
            state,
            debug,
            namespace,
            pod,
            short_pod,
        } = self;
        let debug_item = guarded(
            row,
            debug_container_item(
                debug,
                DebugPod {
                    cluster: row.cluster.clone(),
                    namespace: namespace.clone(),
                    pod: pod.clone(),
                },
                shell,
            ),
        );
        let open = {
            let (cluster, shell, session) =
                (row.cluster.clone(), shell.clone(), row.session.clone());
            move |container: String| -> StartShell {
                let open = ShellOpen {
                    cluster: cluster.clone(),
                    namespace: namespace.clone(),
                    pod: pod.clone(),
                    short_pod: short_pod.clone(),
                    container,
                };
                let (shell, session) = (shell.clone(), session.clone());
                Box::new(move |window: &mut Window, cx: &mut App| {
                    if session.upgrade().is_none() {
                        return;
                    }
                    let open = open.clone();
                    let _ = shell.update(cx, |shell, cx| shell.start_shell(open, window, cx));
                })
            }
        };
        match state {
            ShellMenuState::Disabled(reason) => ShellItems {
                open_shell: row_keyed(disabled_menu_item(label, reason), RowAction::OpenShell),
                debug_container: Some(debug_item),
            },
            ShellMenuState::One(container) => {
                let start = open(container);
                ShellItems {
                    open_shell: row_keyed(
                        PopupMenuItem::new(label).on_click(move |_, window, cx| start(window, cx)),
                        RowAction::OpenShell,
                    ),
                    debug_container: Some(debug_item),
                }
            }
            ShellMenuState::Pick(choices) => {
                let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
                    choice_items(submenu, &choices, &open)
                        .separator()
                        .item(debug_item)
                });
                ShellItems {
                    open_shell: row_keyed(
                        PopupMenuItem::submenu(label, submenu),
                        RowAction::OpenShell,
                    ),
                    debug_container: None,
                }
            }
        }
    }
}

/// The pod a `Debug container…` item acts on, in the pod's own cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DebugPod {
    pub(crate) cluster: ClusterRef,
    pub(crate) namespace: String,
    pub(crate) pod: String,
}

/// The last item of the container submenu (W4 note 2): it opens the options dialog, where the
/// container is picked.
fn debug_container_item(
    state: DebugMenuState,
    pod: DebugPod,
    shell: &WeakEntity<AppShell>,
) -> PopupMenuItem {
    const LABEL: &str = "Debug container…";
    let item = match state {
        DebugMenuState::Disabled(reason) => disabled_menu_item(LABEL, reason),
        DebugMenuState::Ready => {
            let shell = shell.clone();
            PopupMenuItem::new(LABEL).on_click(move |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.open_debug_options(pod.clone(), None, window, cx);
                });
            })
        }
    };
    item.menu_icon(RowAction::DebugContainer.icon())
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

/// The container list of the Open shell submenu in a dialog: S on a pod with several containers shows
/// it before the confirm, and `--screen shell-picker-fixture` shows it since a submenu cannot be held
/// open by a command line.
pub(crate) fn open_shell_picker(
    choices: Vec<ShellChoice>,
    open: impl Fn(String) -> StartShell + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui_kit::Focusable as _;
    use gpui_kit::component::WindowExt as _;
    let menu = PopupMenu::build(window, cx, move |menu, _, _| {
        choice_items(menu, &choices, &open)
    });
    let focus = menu.focus_handle(cx);
    window.open_dialog(cx, move |dialog, _, _| {
        // The menu draws its own border, so the dialog adds neither a title nor padding, and its
        // minimum height would leave an empty band under a short list.
        dialog
            .w(gpui_kit::px(300.))
            .min_h(gpui_kit::px(0.))
            .p_0()
            .close_button(false)
            .child(menu.clone())
    });
    // The arrow keys and Enter pick a container, as in the submenu.
    window.focus(&focus, cx);
}

/// `--screen shell-picker-fixture`: nothing in it opens a shell.
#[cfg(feature = "screenshot")]
pub(crate) fn open_shell_picker_fixture(window: &mut Window, cx: &mut App) {
    let choice = |name: &str, tag, is_running| ShellChoice {
        name: name.to_owned(),
        tag,
        is_running,
    };
    let choices = vec![
        choice("api", "MAIN", true),
        choice("worker", "MAIN", true),
        choice("istio-proxy", "SIDECAR", true),
        choice("migrate", "SIDECAR", false),
    ];
    open_shell_picker(choices, |_| Box::new(|_, _| {}), window, cx);
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
    let item = match action_availability(ResourceAction::Cordon, guard) {
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
    };
    row_keyed(item, RowAction::Cordon)
}

/// Disabled items stay visible with their reason, so users learn what exists. The item shows the
/// key of its action. Drain and the node editors keep their ellipsis because they open a dialog.
fn action_item(action: ResourceAction, guard: &ClusterGuard<'_>) -> PopupMenuItem {
    let label = match action {
        ResourceAction::Drain => "Drain…",
        ResourceAction::EditTaints => "Edit taints…",
        ResourceAction::EditLabels => "Edit labels…",
        _ => action_label(action),
    };
    let item = match action_availability(action, guard) {
        ActionAvailability::Enabled => PopupMenuItem::new(label),
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
    };
    keyed(item, action)
}

/// Drain… of a node: always clickable. A gated session (read-only, locked, a missing permission)
/// gets the read-only preview of the plan, and the item keeps the gate's reason as its hint and
/// tooltip.
fn drain_item(guard: &ClusterGuard<'_>) -> PopupMenuItem {
    match action_availability(ResourceAction::Drain, guard) {
        ActionAvailability::Enabled => action_item(ResourceAction::Drain, guard),
        ActionAvailability::Disabled { reason } => keyed(
            PopupMenuItem::element(move |_, _| {
                hinted_label("Drain…".into(), PREVIEW_HINT.into(), reason.clone())
            }),
            ResourceAction::Drain,
        ),
    }
}

/// The gate first, then the state of this row: the lock and the permissions win over a paused
/// rollout, because the gate reason is the one the user can act on first.
fn row_availability(
    action: ResourceAction,
    guard: &ClusterGuard<'_>,
    object: &KindObject,
    replica_sets: Option<&[ReplicaSetSummary]>,
) -> ActionAvailability {
    match action_availability(action, guard) {
        ActionAvailability::Enabled => match row_block(action, object, replica_sets) {
            Some(reason) => ActionAvailability::Disabled { reason },
            None => ActionAvailability::Enabled,
        },
        disabled => disabled,
    }
}

/// The item of an action of a kind row: the gate decides first, then the state of this row. It has no
/// `on_click`: the menu dispatches the key action, which runs on the cursor row (a right click moved
/// it), so the menu, the key, and the palette share one arm.
fn row_action_item(
    label: &'static str,
    action: ResourceAction,
    guard: &ClusterGuard<'_>,
    object: &KindObject,
    replica_sets: Option<&[ReplicaSetSummary]>,
) -> PopupMenuItem {
    let label = state_label(action, label, object);
    let item = match row_availability(action, guard, object, replica_sets) {
        ActionAvailability::Enabled => PopupMenuItem::new(label),
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
    };
    let item = keyed(item, action);
    match state_icon(action, object) {
        Some(icon) => item.menu_icon(icon),
        None => item,
    }
}

/// The icon a row state flips, as `state_label` flips the text: Resume of a paused Deployment plays.
fn state_icon(action: ResourceAction, object: &KindObject) -> Option<IconName> {
    match (action, object) {
        (ResourceAction::PauseRollout, KindObject::Deployment(deployment))
            if deployment.is_paused =>
        {
            Some(IconName::Play)
        }
        _ => None,
    }
}

/// `item` with the key hint and the icon of `row`. `.action()` is a no-op on a submenu, and the
/// icon is what it shows.
pub(crate) fn row_keyed(item: PopupMenuItem, row: RowAction) -> PopupMenuItem {
    item.action(row.key_action()).menu_icon(row.icon())
}

/// `item` with the key hint of `action`: a menu item of a row action shows its key.
fn keyed(item: PopupMenuItem, action: ResourceAction) -> PopupMenuItem {
    match action.row_action() {
        Some(row) => row_keyed(item, row),
        None => item,
    }
}

/// How far a disabled menu item fades, label and icon alike, so that it reads as unavailable next
/// to the full-colour items that can be clicked. Opacity is not a colour: both stay theme tokens.
const DISABLED_ITEM_OPACITY: f32 = 0.55;

/// `PopupMenuItem::icon` that fades the icon of a disabled item with its label. Set `disabled`
/// before the icon.
pub(crate) trait MenuItemIcon {
    fn menu_icon(self, icon: impl Into<Icon>) -> Self;
}

impl MenuItemIcon for PopupMenuItem {
    fn menu_icon(self, icon: impl Into<Icon>) -> Self {
        let is_disabled = matches!(
            &self,
            PopupMenuItem::Item { disabled: true, .. }
                | PopupMenuItem::ElementItem { disabled: true, .. }
                | PopupMenuItem::Submenu { disabled: true, .. }
        );
        let icon = icon.into();
        self.icon(if is_disabled {
            icon.opacity(DISABLED_ITEM_OPACITY)
        } else {
            icon
        })
    }
}

/// The widest the reason of a disabled item grows; a longer one is cut with an ellipsis, so that a
/// reason never stretches the menu.
const REASON_WIDTH: Pixels = px(240.);

/// The chord of the read-only toggle (`ToggleReadOnly`, bound with the platform modifier).
const UNLOCK_KEYS: &str = if cfg!(target_os = "macos") {
    "Cmd+Shift+R"
} else {
    "Ctrl+Shift+R"
};

/// `reason`, followed by the next step: how to unlock the cluster when the reason says it is
/// read-only or was locked while a dialog was open, or where to read the user's rules when a
/// permission is missing. Any other reason comes back as it is.
pub(crate) fn with_next_step(reason: &SharedString) -> SharedString {
    let name = reason
        .strip_suffix(READ_ONLY_SUFFIX)
        .or_else(|| reason.strip_suffix(LOCKED_SUFFIX));
    match name {
        Some(name) => {
            format!("{reason}. Unlock {name} with {UNLOCK_KEYS} or the title-bar badge").into()
        }
        None if reason.starts_with(NOT_PERMITTED) => {
            format!("{reason}. Open Check permissions to see your rules").into()
        }
        None => reason.clone(),
    }
}

const READ_ONLY_SUFFIX: &str = " is read-only";
const LOCKED_SUFFIX: &str = " was locked; nothing was changed";

/// A disabled item: its label, then a short reason on the right where an enabled item shows its
/// key. Both are muted like the rest of the row; the full reason is the tooltip.
pub(crate) fn disabled_menu_item(
    label: impl Into<SharedString>,
    reason: SharedString,
) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |_, _| disabled_label(label.clone(), reason.clone())).disabled(true)
}

fn disabled_label(label: SharedString, reason: SharedString) -> AnyElement {
    let short = short_reason(&reason);
    hinted_label(label, short, reason)
}

/// What an enabled item that only opens a read-only preview shows where a key hint would be; the
/// permission detail stays in the tooltip.
pub(crate) const PREVIEW_HINT: &str = "Preview only";

/// A menu label with `hint` on the right and the full `reason` as the tooltip.
fn hinted_label(label: SharedString, hint: SharedString, reason: SharedString) -> AnyElement {
    h_flex()
        .id(label.clone())
        .w_full()
        .gap_4()
        .justify_between()
        .child(div().flex_shrink_0().child(label))
        .child(
            div()
                .min_w_0()
                .max_w(REASON_WIDTH)
                .truncate()
                .text_sm()
                .child(hint),
        )
        .tooltip(move |window, cx| Tooltip::new(with_next_step(&reason)).build(window, cx))
        .into_any_element()
}

/// The few words a disabled menu row shows for `reason`. Reasons travel as full sentences (they
/// also fill notices and tooltips), so the short form is derived here, in the one place the menus
/// read; a reason with no entry shows as it is and is cut at `REASON_WIDTH`.
pub(crate) fn short_reason(reason: &str) -> SharedString {
    let short = if reason.starts_with("Not permitted: ") {
        // The missing verb and resource stay in the tooltip; the row itself keeps to two words.
        "No permission"
    } else if reason.ends_with(READ_ONLY_SUFFIX) {
        return format!("Read-only · {UNLOCK_KEYS}").into();
    } else if reason == NOT_SHIPPED_REASON {
        "Later version"
    } else if reason == "Permissions could not be checked" {
        "Not checked"
    } else if reason.starts_with("Node shell is off") {
        "Off in Settings"
    } else if reason == "Node shell needs a Linux node" {
        "Linux only"
    } else if reason.starts_with("Not managed by a controller") {
        "No controller"
    } else if reason.starts_with("The pod has finished") {
        "Finished"
    } else if reason == STATIC_POD_TEXT {
        "Static pod"
    } else if reason == NOT_RUNNING_REASON || reason == "The pod has no running container" {
        "Not running"
    } else if reason == "Init containers cannot be attached" {
        "Init container"
    } else if reason.contains("terminal (stdin and tty)") {
        "No terminal"
    } else if reason.contains("cannot be edited") || reason.contains("are managed by") {
        "Managed"
    } else {
        return reason.to_owned().into();
    };
    short.into()
}

fn copy_name_item(name: &str, access: &AccessState) -> PopupMenuItem {
    let label = action_label(ResourceAction::CopyName);
    let item = match availability_before_lock(ResourceAction::CopyName, access) {
        ActionAvailability::Disabled { reason } => disabled_menu_item(label, reason),
        ActionAvailability::Enabled => {
            let name = name.to_owned();
            PopupMenuItem::new(label).on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
            })
        }
    };
    row_keyed(item, RowAction::CopyName)
}

#[cfg(test)]
#[path = "resource_actions_tests.rs"]
mod resource_actions_tests;
