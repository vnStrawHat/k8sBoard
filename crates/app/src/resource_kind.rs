//! The Kubernetes kinds that have an explorer screen, and the data that differs per kind.

use cluster::{
    AccessCheck, ClusterConnection, EventFilter, NamespaceScope, ObjectKind, ObjectRef, WatchUpdate,
};
use futures::StreamExt as _;
use futures::stream::BoxStream;
use gpui_kit::assets::IconName;

use crate::access_rows::{
    cluster_role_binding_row, cluster_role_row, role_binding_row, role_row, service_account_row,
};
use crate::batch_rows::{cron_job_row, job_row};
use crate::config_map_rows::config_map_row;
use crate::custom_kind::CustomKind;
use crate::custom_rows::custom_object_row;
use crate::event_rows::event_rows;
use crate::helm_rows::helm_release_row;
use crate::kind_row::KindRow;
use crate::namespace_rows::namespace_row;
use crate::network_policy_rows::network_policy_row;
use crate::network_rows::{ingress_row, service_row};
use crate::policy_rows::{
    horizontal_pod_autoscaler_row, pod_disruption_budget_row, resource_quota_row,
};
use crate::resource_actions::ResourceAction;
use crate::secret_rows::secret_row;
use crate::storage_rows::{persistent_volume_claim_row, persistent_volume_row, storage_class_row};
use crate::workload_rows::{daemon_set_row, deployment_row, replica_set_row, stateful_set_row};

/// Pods and nodes have their own screens, so their icons are not a `KindSpec` field.
pub(crate) const POD_ICON: IconName = IconName::Box;
pub(crate) const NODE_ICON: IconName = IconName::Server;

/// One kind with an explorer screen. Per-kind variation is data (the tables below) plus one
/// `match` in `watch_rows`; there is no trait.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ResourceKind {
    Namespaces,
    Events,
    Deployments,
    StatefulSets,
    DaemonSets,
    ReplicaSets,
    Jobs,
    CronJobs,
    Services,
    Ingresses,
    ConfigMaps,
    NetworkPolicies,
    PodDisruptionBudgets,
    HorizontalPodAutoscalers,
    ResourceQuotas,
    PersistentVolumeClaims,
    PersistentVolumes,
    StorageClasses,
    Roles,
    ClusterRoles,
    RoleBindings,
    ClusterRoleBindings,
    ServiceAccounts,
    Secrets,
    /// Helm releases, read from their `helm.sh/release.v1` Secrets. It shares `ObjectKind::Secret`
    /// with `Secrets`, so `from_object_kind` keeps finding Secrets first.
    HelmReleases,
    /// Custom resource definitions. The list is fed by the session's CRD watch, which also drives
    /// the custom kinds, so the kind has no watch of its own.
    Crds,
    /// A served custom resource kind, from an Established CRD. Never in `ALL`.
    Custom(CustomKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Right,
}

/// Whether a kind leads with the Name column, or hides it and flexes one of its own columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameColumn {
    Flexible,
    /// `flexible` indexes `ResourceKind::columns`.
    Hidden {
        flexible: usize,
    },
}

/// A column after the Name column, or all the columns when the Name column is hidden.
///
/// `width` is the width the column always has. A column with a `weight` also takes a share of the
/// table's spare width in proportion to it, up to `max_width` (0 for no limit); weight 0 is a
/// column whose values are short and fixed.
#[derive(Clone, Copy)]
pub(crate) struct KindColumn {
    pub(crate) name: &'static str,
    pub(crate) width: f32,
    pub(crate) align: Align,
    pub(crate) weight: u8,
    pub(crate) max_width: f32,
}

pub(crate) const fn column(name: &'static str, width: f32, align: Align) -> KindColumn {
    KindColumn {
        name,
        width,
        align,
        weight: 0,
        max_width: 0.,
    }
}

impl KindColumn {
    /// Takes a share of the spare table width, `weight` parts of it.
    pub(crate) const fn grows(self, weight: u8) -> Self {
        Self { weight, ..self }
    }

    /// Stops growing at `max_width`.
    pub(crate) const fn up_to(self, max_width: f32) -> Self {
        Self { max_width, ..self }
    }
}

const AGE_COLUMN: KindColumn = column("Age", 70., Align::Right);

/// One mutating menu item of a kind that is shown disabled. `action` is the key action behind
/// it, when the wireframe gives it a key; the menu shows that key as a hint.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KindAction {
    pub(crate) label: &'static str,
    pub(crate) action: Option<ResourceAction>,
}

impl KindAction {
    pub(crate) const fn named(label: &'static str) -> Self {
        Self {
            label,
            action: None,
        }
    }

    pub(crate) const fn keyed(label: &'static str, action: ResourceAction) -> Self {
        Self {
            label,
            action: Some(action),
        }
    }
}

/// Everything that differs between kinds except the watch. A new kind adds one `static` here,
/// one arm in `spec`, and one arm in `watch_rows`.
pub(crate) struct KindSpec {
    pub(crate) label: &'static str,
    pub(crate) name_column: NameColumn,
    /// Whether the drawer has a Labels section.
    pub(crate) has_labels: bool,
    pub(crate) singular: &'static str,
    pub(crate) plural: &'static str,
    pub(crate) badge: &'static str,
    pub(crate) icon: IconName,
    pub(crate) is_namespaced: bool,
    pub(crate) api: KindApi,
    pub(crate) columns: &'static [KindColumn],
    pub(crate) read_only_actions: &'static [KindAction],
    pub(crate) delete_label: &'static str,
    pub(crate) has_port_forward: bool,
}

/// How a kind reaches the cluster: a built-in kind has a typed object and a list check; a custom
/// kind has neither (its resource comes from its CRD, and its gate is a per-resource review).
#[derive(Clone, Copy)]
pub(crate) enum KindApi {
    Builtin {
        /// The Kubernetes `kind`, as an event's `involvedObject.kind` spells it.
        object: ObjectKind,
        access_check: AccessCheck,
    },
    Custom,
}

static NAMESPACES: KindSpec = KindSpec {
    label: "Namespaces",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "namespace",
    plural: "namespaces",
    badge: "Ns",
    icon: IconName::Folder,
    is_namespaced: false,
    api: KindApi::Builtin {
        object: ObjectKind::Namespace,
        access_check: AccessCheck::ListNamespaces,
    },
    columns: &[
        column("Status", 140., Align::Left),
        column("Pods", 70., Align::Right),
        column("CPU req", 110., Align::Right),
        column("Memory req", 120., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete namespace…",
    has_port_forward: false,
};

static EVENTS: KindSpec = KindSpec {
    label: "Events",
    name_column: NameColumn::Hidden { flexible: 3 },
    has_labels: false,
    singular: "event",
    plural: "events",
    badge: "Ev",
    icon: IconName::Bell,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::Event,
        access_check: AccessCheck::ListEvents,
    },
    columns: &[
        column("Type", 80., Align::Left),
        column("Reason", 260., Align::Left).grows(1).up_to(320.),
        column("Object", 220., Align::Left).grows(1).up_to(360.),
        column("Message", 160., Align::Left).grows(4),
        column("Count", 90., Align::Right),
        column("First seen", 90., Align::Right),
        column("Last seen", 80., Align::Right),
    ],
    read_only_actions: &[],
    delete_label: "Delete event…",
    has_port_forward: false,
};

static DEPLOYMENTS: KindSpec = KindSpec {
    label: "Deployments",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "deployment",
    plural: "deployments",
    badge: "De",
    icon: IconName::Layers,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::Deployment,
        access_check: AccessCheck::ListDeployments,
    },
    columns: &[
        column("Ready", 80., Align::Left),
        column("Up-to-date", 100., Align::Right),
        column("Available", 90., Align::Right),
        column("Strategy", 130., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &[
        KindAction::keyed("Scale…", ResourceAction::Scale(ObjectKind::Deployment)),
        KindAction::keyed(
            "Restart rollout",
            ResourceAction::RestartRollout(ObjectKind::Deployment),
        ),
        KindAction::keyed("Roll back…", ResourceAction::RollBack),
        KindAction::keyed("Pause rollout", ResourceAction::PauseRollout),
    ],
    delete_label: "Delete deployment…",
    has_port_forward: true,
};

static STATEFUL_SETS: KindSpec = KindSpec {
    label: "StatefulSets",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "statefulset",
    plural: "statefulsets",
    badge: "Ss",
    icon: IconName::Database,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::StatefulSet,
        access_check: AccessCheck::ListStatefulSets,
    },
    columns: &[
        column("Ready", 80., Align::Left),
        column("Service", 200., Align::Left).grows(1),
        column("Strategy", 140., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &[
        KindAction::keyed("Scale…", ResourceAction::Scale(ObjectKind::StatefulSet)),
        KindAction::keyed(
            "Restart rollout",
            ResourceAction::RestartRollout(ObjectKind::StatefulSet),
        ),
    ],
    delete_label: "Delete statefulset…",
    has_port_forward: true,
};

static DAEMON_SETS: KindSpec = KindSpec {
    label: "DaemonSets",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "daemonset",
    plural: "daemonsets",
    badge: "Ds",
    icon: IconName::Radio,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::DaemonSet,
        access_check: AccessCheck::ListDaemonSets,
    },
    columns: &[
        column("Desired", 80., Align::Right),
        column("Current", 80., Align::Right),
        column("Ready", 80., Align::Right),
        column("Up-to-date", 100., Align::Right),
        column("Available", 90., Align::Right),
        column("Node selector", 200., Align::Left).grows(2),
        AGE_COLUMN,
    ],
    read_only_actions: &[KindAction::keyed(
        "Restart rollout",
        ResourceAction::RestartRollout(ObjectKind::DaemonSet),
    )],
    delete_label: "Delete daemonset…",
    has_port_forward: false,
};

static REPLICA_SETS: KindSpec = KindSpec {
    label: "ReplicaSets",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "replicaset",
    plural: "replicasets",
    badge: "Rs",
    icon: IconName::Grid2x2,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::ReplicaSet,
        access_check: AccessCheck::ListReplicaSets,
    },
    columns: &[
        column("Desired", 80., Align::Right),
        column("Current", 80., Align::Right),
        column("Ready", 80., Align::Right),
        column("Owner", 220., Align::Left).grows(1),
        AGE_COLUMN,
    ],
    // Scale belongs to the owning Deployment.
    read_only_actions: &[],
    delete_label: "Delete replicaset…",
    has_port_forward: false,
};

static JOBS: KindSpec = KindSpec {
    label: "Jobs",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "job",
    plural: "jobs",
    badge: "Jb",
    icon: IconName::BriefcaseBusiness,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::Job,
        access_check: AccessCheck::ListJobs,
    },
    columns: &[
        column("Status", 120., Align::Left),
        column("Completions", 110., Align::Left),
        column("Duration", 90., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[KindAction::keyed("Re-run job", ResourceAction::RerunJob)],
    delete_label: "Delete job…",
    has_port_forward: false,
};

static CRON_JOBS: KindSpec = KindSpec {
    label: "CronJobs",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "cronjob",
    plural: "cronjobs",
    badge: "Cj",
    icon: IconName::CalendarClock,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::CronJob,
        access_check: AccessCheck::ListCronJobs,
    },
    columns: &[
        column("Schedule", 140., Align::Left),
        column("Suspend", 80., Align::Left),
        column("Active", 70., Align::Right),
        column("Last run", 120., Align::Right),
        column("Next run", 100., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[
        KindAction::keyed("Trigger now", ResourceAction::TriggerCronJob),
        KindAction::keyed("Suspend", ResourceAction::SuspendCronJob),
    ],
    delete_label: "Delete cronjob…",
    has_port_forward: false,
};

static SERVICES: KindSpec = KindSpec {
    label: "Services",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "service",
    plural: "services",
    badge: "Sv",
    icon: IconName::Network,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::Service,
        access_check: AccessCheck::ListServices,
    },
    columns: &[
        column("Type", 130., Align::Left),
        column("Cluster IP", 140., Align::Left),
        column("External IP", 110., Align::Left),
        column("Ports", 150., Align::Left).grows(1),
        column("Endpoints", 80., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete service…",
    has_port_forward: true,
};

static INGRESSES: KindSpec = KindSpec {
    label: "Ingresses",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "ingress",
    plural: "ingresses",
    badge: "In",
    icon: IconName::Globe,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::Ingress,
        access_check: AccessCheck::ListIngresses,
    },
    columns: &[
        column("Class", 100., Align::Left),
        column("Hosts", 260., Align::Left).grows(2),
        column("Backends", 160., Align::Left).grows(1),
        column("Address", 180., Align::Left),
        column("TLS", 140., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete ingress…",
    has_port_forward: false,
};

static CONFIG_MAPS: KindSpec = KindSpec {
    label: "ConfigMaps",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "configmap",
    plural: "configmaps",
    badge: "Cm",
    icon: IconName::FileCog,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::ConfigMap,
        access_check: AccessCheck::ListConfigMaps,
    },
    columns: &[
        column("Data", 70., Align::Right),
        column("Used by", 170., Align::Left).grows(2).up_to(300.),
        AGE_COLUMN,
    ],
    read_only_actions: &[KindAction::keyed(
        "Edit values…",
        ResourceAction::EditValues(ObjectKind::ConfigMap),
    )],
    delete_label: "Delete configmap…",
    has_port_forward: false,
};

static NETWORK_POLICIES: KindSpec = KindSpec {
    label: "NetworkPolicies",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "networkpolicy",
    plural: "networkpolicies",
    badge: "Np",
    icon: IconName::BrickWallShield,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::NetworkPolicy,
        access_check: AccessCheck::ListNetworkPolicies,
    },
    columns: &[
        column("Pod selector", 220., Align::Left).grows(2),
        column("Policy types", 130., Align::Left),
        column("Affects", 90., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete policy…",
    has_port_forward: false,
};

static POD_DISRUPTION_BUDGETS: KindSpec = KindSpec {
    label: "PDBs",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "poddisruptionbudget",
    plural: "poddisruptionbudgets",
    badge: "Pd",
    icon: IconName::ShieldCheck,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::PodDisruptionBudget,
        access_check: AccessCheck::ListPodDisruptionBudgets,
    },
    columns: &[
        column("Min available", 110., Align::Left),
        column("Max unavailable", 135., Align::Left),
        column("Allowed disruptions", 155., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete PDB…",
    has_port_forward: false,
};

static HORIZONTAL_POD_AUTOSCALERS: KindSpec = KindSpec {
    label: "HPAs",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "horizontalpodautoscaler",
    plural: "horizontalpodautoscalers",
    badge: "Hp",
    icon: IconName::Gauge,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::HorizontalPodAutoscaler,
        access_check: AccessCheck::ListHorizontalPodAutoscalers,
    },
    columns: &[
        // Capped, so the Name column (the widest weight) gets the spare width of a wide table.
        column("Target", 200., Align::Left).grows(1).up_to(320.),
        column("Min / Max", 90., Align::Left),
        column("Replicas", 80., Align::Right),
        column("Status", 135., Align::Left),
        column("Metrics", 170., Align::Left).grows(1).up_to(320.),
        AGE_COLUMN,
    ],
    read_only_actions: &[KindAction::keyed(
        "Edit min / max…",
        ResourceAction::EditHpaRange,
    )],
    delete_label: "Delete HPA…",
    has_port_forward: false,
};

static RESOURCE_QUOTAS: KindSpec = KindSpec {
    label: "ResourceQuotas",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "resourcequota",
    plural: "resourcequotas",
    badge: "Rq",
    icon: IconName::ChartPie,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::ResourceQuota,
        access_check: AccessCheck::ListResourceQuotas,
    },
    columns: &[
        column("CPU req", 130., Align::Right),
        column("Memory req", 150., Align::Right),
        column("Pods", 100., Align::Right),
        column("Fullest", 160., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete quota…",
    has_port_forward: false,
};

static PERSISTENT_VOLUME_CLAIMS: KindSpec = KindSpec {
    label: "PVCs",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "persistentvolumeclaim",
    plural: "persistentvolumeclaims",
    badge: "Pc",
    icon: IconName::Ticket,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::PersistentVolumeClaim,
        access_check: AccessCheck::ListPersistentVolumeClaims,
    },
    columns: &[
        column("Status", 110., Align::Left),
        column("Capacity", 90., Align::Right),
        column("Used", 80., Align::Right),
        column("Access", 90., Align::Left),
        column("Class", 150., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &[KindAction::keyed("Expand…", ResourceAction::ExpandClaim)],
    delete_label: "Delete PVC…",
    has_port_forward: false,
};

static PERSISTENT_VOLUMES: KindSpec = KindSpec {
    label: "PVs",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "persistentvolume",
    plural: "persistentvolumes",
    badge: "Pv",
    icon: IconName::HardDrive,
    is_namespaced: false,
    api: KindApi::Builtin {
        object: ObjectKind::PersistentVolume,
        access_check: AccessCheck::ListPersistentVolumes,
    },
    columns: &[
        // Status and Claim lead: with a drawer open the table is cut at the drawer, and they matter
        // more than the access mode and the reclaim policy. At 1320 px the drawer leaves 550 px, so
        // the select box, Name, Status, and Claim add up to less than that, and the width this takes
        // from Status and Claim goes to Class: the table has no spare width there for Name to take.
        column("Status", 90., Align::Left),
        column("Claim", 220., Align::Left).grows(2),
        column("Capacity", 90., Align::Right),
        column("Class", 190., Align::Left).grows(1),
        column("Access", 90., Align::Left),
        column("Reclaim", 90., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete PV…",
    has_port_forward: false,
};

static STORAGE_CLASSES: KindSpec = KindSpec {
    label: "StorageClasses",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "storageclass",
    plural: "storageclasses",
    badge: "Sc",
    icon: IconName::Archive,
    is_namespaced: false,
    api: KindApi::Builtin {
        object: ObjectKind::StorageClass,
        access_check: AccessCheck::ListStorageClasses,
    },
    columns: &[
        column("Provisioner", 200., Align::Left).grows(1),
        column("Reclaim", 90., Align::Left),
        column("Binding mode", 190., Align::Left),
        column("Expansion", 90., Align::Left),
        column("Default", 70., Align::Left),
        column("PVs", 70., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[KindAction::keyed(
        "Set as default",
        ResourceAction::SetDefaultStorageClass,
    )],
    delete_label: "Delete storage class…",
    has_port_forward: false,
};

static ROLES: KindSpec = KindSpec {
    label: "Roles",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "role",
    plural: "roles",
    badge: "Ro",
    icon: IconName::ScrollText,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::Role,
        access_check: AccessCheck::ListRoles,
    },
    columns: &[
        column("Rules", 70., Align::Right),
        column("Bindings", 90., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete role…",
    has_port_forward: false,
};

static CLUSTER_ROLES: KindSpec = KindSpec {
    label: "ClusterRoles",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "clusterrole",
    plural: "clusterroles",
    badge: "Cr",
    icon: IconName::BookKey,
    is_namespaced: false,
    api: KindApi::Builtin {
        object: ObjectKind::ClusterRole,
        access_check: AccessCheck::ListClusterRoles,
    },
    columns: &[
        column("Rules", 90., Align::Right),
        column("Aggregated", 100., Align::Left),
        column("Bindings", 90., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete cluster role…",
    has_port_forward: false,
};

static ROLE_BINDINGS: KindSpec = KindSpec {
    label: "RoleBindings",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "rolebinding",
    plural: "rolebindings",
    badge: "Rb",
    icon: IconName::Link,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::RoleBinding,
        access_check: AccessCheck::ListRoleBindings,
    },
    columns: &[
        column("Role", 220., Align::Left).grows(2),
        column("Subjects", 300., Align::Left).grows(2),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete binding…",
    has_port_forward: false,
};

static CLUSTER_ROLE_BINDINGS: KindSpec = KindSpec {
    label: "ClusterRoleBindings",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "clusterrolebinding",
    plural: "clusterrolebindings",
    badge: "Cb",
    icon: IconName::Cable,
    is_namespaced: false,
    api: KindApi::Builtin {
        object: ObjectKind::ClusterRoleBinding,
        access_check: AccessCheck::ListClusterRoleBindings,
    },
    columns: &[
        column("ClusterRole", 200., Align::Left).grows(2),
        column("Subjects", 300., Align::Left).grows(2),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete binding…",
    has_port_forward: false,
};

static SERVICE_ACCOUNTS: KindSpec = KindSpec {
    label: "ServiceAccounts",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "serviceaccount",
    plural: "serviceaccounts",
    badge: "Sa",
    icon: IconName::Bot,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::ServiceAccount,
        access_check: AccessCheck::ListServiceAccounts,
    },
    columns: &[
        column("Bound roles", 280., Align::Left).grows(2),
        column("Used by", 90., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete service account…",
    has_port_forward: false,
};

static SECRETS: KindSpec = KindSpec {
    label: "Secrets",
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "secret",
    plural: "secrets",
    badge: "Se",
    icon: IconName::KeyRound,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::Secret,
        access_check: AccessCheck::ListSecrets,
    },
    columns: &[
        column("Type", 190., Align::Left),
        column("Keys", 70., Align::Right),
        column("Used by", 170., Align::Left).grows(2).up_to(300.),
        AGE_COLUMN,
    ],
    read_only_actions: &[KindAction::keyed(
        "Edit values…",
        ResourceAction::EditValues(ObjectKind::Secret),
    )],
    delete_label: "Delete secret…",
    has_port_forward: false,
};

static HELM_RELEASES: KindSpec = KindSpec {
    label: "Releases",
    name_column: NameColumn::Flexible,
    // Helm's labels are bookkeeping (owner, status, version), not the user's.
    has_labels: false,
    singular: "release",
    plural: "releases",
    badge: "Hm",
    icon: IconName::ShipWheel,
    is_namespaced: true,
    api: KindApi::Builtin {
        object: ObjectKind::Secret,
        access_check: AccessCheck::ListSecrets,
    },
    columns: &[
        column("Chart", 260., Align::Left).grows(2),
        column("App version", 110., Align::Left),
        column("Revision", 80., Align::Right),
        column("Status", 130., Align::Left),
        column("Updated", 100., Align::Right),
    ],
    read_only_actions: &[KindAction::named("Roll back…")],
    delete_label: "Uninstall release…",
    has_port_forward: false,
};

static CRDS: KindSpec = KindSpec {
    label: "CRDs",
    name_column: NameColumn::Flexible,
    // The summary holds no labels.
    has_labels: false,
    singular: "customresourcedefinition",
    plural: "customresourcedefinitions",
    badge: "Cd",
    icon: IconName::Blocks,
    is_namespaced: false,
    api: KindApi::Builtin {
        object: ObjectKind::CustomResourceDefinition,
        access_check: AccessCheck::ListCustomResourceDefinitions,
    },
    columns: &[
        column("Group", 200., Align::Left).grows(1),
        column("Version", 90., Align::Left),
        column("Scope", 110., Align::Left),
        column("Instances", 90., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete CRD…",
    has_port_forward: false,
};

/// The Name column of a kind that shows it, as wide as its minimum.
pub(crate) const NAME_COLUMN: KindColumn = column("Name", 200., Align::Left).grows(3).up_to(640.);

/// Every logical column of a kind's table: Name first unless the kind hides it, then
/// `ResourceKind::columns`.
pub(crate) fn kind_columns(kind: ResourceKind) -> Vec<KindColumn> {
    let rest = kind.columns().iter().copied();
    match kind.name_column() {
        NameColumn::Flexible => std::iter::once(NAME_COLUMN).chain(rest).collect(),
        NameColumn::Hidden { .. } => rest.collect(),
    }
}

impl ResourceKind {
    pub(crate) const ALL: [Self; 26] = [
        Self::Namespaces,
        Self::Events,
        Self::Deployments,
        Self::StatefulSets,
        Self::DaemonSets,
        Self::ReplicaSets,
        Self::Jobs,
        Self::CronJobs,
        Self::Services,
        Self::Ingresses,
        Self::ConfigMaps,
        Self::NetworkPolicies,
        Self::PodDisruptionBudgets,
        Self::HorizontalPodAutoscalers,
        Self::ResourceQuotas,
        Self::PersistentVolumeClaims,
        Self::PersistentVolumes,
        Self::StorageClasses,
        Self::Roles,
        Self::ClusterRoles,
        Self::RoleBindings,
        Self::ClusterRoleBindings,
        Self::ServiceAccounts,
        Self::Secrets,
        Self::HelmReleases,
        Self::Crds,
    ];

    fn spec(self) -> &'static KindSpec {
        match self {
            Self::Namespaces => &NAMESPACES,
            Self::Events => &EVENTS,
            Self::Deployments => &DEPLOYMENTS,
            Self::StatefulSets => &STATEFUL_SETS,
            Self::DaemonSets => &DAEMON_SETS,
            Self::ReplicaSets => &REPLICA_SETS,
            Self::Jobs => &JOBS,
            Self::CronJobs => &CRON_JOBS,
            Self::Services => &SERVICES,
            Self::Ingresses => &INGRESSES,
            Self::ConfigMaps => &CONFIG_MAPS,
            Self::NetworkPolicies => &NETWORK_POLICIES,
            Self::PodDisruptionBudgets => &POD_DISRUPTION_BUDGETS,
            Self::HorizontalPodAutoscalers => &HORIZONTAL_POD_AUTOSCALERS,
            Self::ResourceQuotas => &RESOURCE_QUOTAS,
            Self::PersistentVolumeClaims => &PERSISTENT_VOLUME_CLAIMS,
            Self::PersistentVolumes => &PERSISTENT_VOLUMES,
            Self::StorageClasses => &STORAGE_CLASSES,
            Self::Roles => &ROLES,
            Self::ClusterRoles => &CLUSTER_ROLES,
            Self::RoleBindings => &ROLE_BINDINGS,
            Self::ClusterRoleBindings => &CLUSTER_ROLE_BINDINGS,
            Self::ServiceAccounts => &SERVICE_ACCOUNTS,
            Self::Secrets => &SECRETS,
            Self::HelmReleases => &HELM_RELEASES,
            Self::Crds => &CRDS,
            Self::Custom(kind) => kind.spec(),
        }
    }

    /// The sidebar item and the screen title.
    pub(crate) fn label(self) -> &'static str {
        self.spec().label
    }

    pub(crate) fn singular(self) -> &'static str {
        self.spec().singular
    }

    /// The kubectl resource name: the count label and the `--screen` slug.
    pub(crate) fn plural(self) -> &'static str {
        self.spec().plural
    }

    /// Two letters in the Topology export.
    pub(crate) fn badge(self) -> &'static str {
        self.spec().badge
    }

    pub(crate) fn icon(self) -> IconName {
        self.spec().icon
    }

    pub(crate) fn is_namespaced(self) -> bool {
        self.spec().is_namespaced
    }

    /// The list check of a built-in kind; `None` for a custom kind, which is reviewed per resource.
    pub(crate) fn access_check(self) -> Option<AccessCheck> {
        match self.spec().api {
            KindApi::Builtin { access_check, .. } => Some(access_check),
            KindApi::Custom => None,
        }
    }

    /// The columns after Name, or all of them when Name is hidden.
    pub(crate) fn columns(self) -> &'static [KindColumn] {
        self.spec().columns
    }

    /// The mutating menu items that are shown disabled.
    pub(crate) fn read_only_actions(self) -> &'static [KindAction] {
        self.spec().read_only_actions
    }

    pub(crate) fn delete_label(self) -> &'static str {
        self.spec().delete_label
    }

    pub(crate) fn has_port_forward(self) -> bool {
        self.spec().has_port_forward
    }

    /// The typed object kind of a built-in kind; `None` for a custom kind.
    pub(crate) fn builtin_object(self) -> Option<ObjectKind> {
        match self.spec().api {
            KindApi::Builtin { object, .. } => Some(object),
            KindApi::Custom => None,
        }
    }

    /// The Kubernetes `kind` name, as an event's `involvedObject.kind` spells it.
    pub(crate) fn object_kind(self) -> &'static str {
        match self {
            Self::Custom(kind) => &kind.resource().kind,
            _ => self.builtin_object().map_or("", ObjectKind::name),
        }
    }

    /// The kind as a drawer title names it: the Kubernetes `kind` (`StatefulSet`), or `Helm
    /// release` for a release, which is stored as a Secret.
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::HelmReleases => "Helm release",
            _ => self.object_kind(),
        }
    }

    /// The reference to one object of this kind; `None` when the namespace does not fit its scope.
    pub(crate) fn object_ref(self, namespace: Option<String>, name: String) -> Option<ObjectRef> {
        match self {
            Self::Custom(kind) => ObjectRef::custom(kind.resource().clone(), namespace, name),
            _ => ObjectRef::new(self.builtin_object()?, namespace, name),
        }
    }

    pub(crate) fn custom(self) -> Option<CustomKind> {
        match self {
            Self::Custom(kind) => Some(kind),
            _ => None,
        }
    }

    pub(crate) fn name_column(self) -> NameColumn {
        self.spec().name_column
    }

    pub(crate) fn has_labels(self) -> bool {
        self.spec().has_labels
    }

    /// Whether the sidebar counts the kind with a one-shot list. Releases do not: counting their
    /// Secrets would count revisions, not releases.
    pub(crate) fn has_count(self) -> bool {
        !matches!(self, Self::HelmReleases | Self::Custom(_))
    }

    /// Whether the drawer has a Monitor tab: the workloads that own pods (a CronJob has none).
    pub(crate) fn has_monitor(self) -> bool {
        matches!(
            self,
            Self::Deployments
                | Self::StatefulSets
                | Self::DaemonSets
                | Self::ReplicaSets
                | Self::Jobs
        )
    }

    /// The kubectl short names the command palette accepts after `:`, besides the plural and
    /// singular. Exhaustive, so a new kind has to name its own; a custom kind has none.
    pub(crate) fn short_names(self) -> &'static [&'static str] {
        match self {
            Self::Namespaces => &["ns"],
            Self::Events => &["ev"],
            Self::Deployments => &["deploy"],
            Self::StatefulSets => &["sts"],
            Self::DaemonSets => &["ds"],
            Self::ReplicaSets => &["rs"],
            Self::Jobs => &["job"],
            Self::CronJobs => &["cj"],
            Self::Services => &["svc"],
            Self::Ingresses => &["ing"],
            Self::ConfigMaps => &["cm"],
            Self::NetworkPolicies => &["netpol"],
            Self::PodDisruptionBudgets => &["pdb"],
            Self::HorizontalPodAutoscalers => &["hpa"],
            Self::ResourceQuotas => &["quota"],
            Self::PersistentVolumeClaims => &["pvc"],
            Self::PersistentVolumes => &["pv"],
            Self::StorageClasses => &["sc"],
            Self::Roles => &["role"],
            Self::ClusterRoles => &["cr"],
            Self::RoleBindings => &["rb"],
            Self::ClusterRoleBindings => &["crb"],
            Self::ServiceAccounts => &["sa"],
            Self::Secrets => &["secret"],
            Self::HelmReleases => &["helm"],
            Self::Crds => &["crd"],
            Self::Custom(_) => &[],
        }
    }

    pub(crate) fn from_object_kind(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.object_kind() == text)
    }

    pub(crate) fn from_label(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.label() == text)
    }

    pub(crate) fn from_plural(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.plural() == text)
    }

    /// The only per-kind `match` over cluster calls: watch, then map to rows on tokio, so the
    /// main thread only swaps a `Vec`. Cluster-scoped kinds (Namespaces, PVs, StorageClasses,
    /// ClusterRoles, ClusterRoleBindings) ignore `scope`. Only
    /// Events reads `events`.
    pub(crate) fn watch_rows(
        self,
        connection: &ClusterConnection,
        scope: NamespaceScope,
        events: EventFilter,
    ) -> Option<BoxStream<'static, WatchUpdate<KindRow>>> {
        let stream = match self {
            Self::Crds => return None,
            Self::Custom(custom) => connection
                .watch_custom_objects(custom.resource(), custom.printer_columns(), scope)
                .map(move |update| rows(update, |summary| custom_object_row(custom, summary)))
                .boxed(),
            Self::Namespaces => connection
                .watch_namespaces()
                .map(|update| rows(update, namespace_row))
                .boxed(),
            Self::Events => connection
                .watch_events(scope, events)
                .map(event_rows)
                .boxed(),
            Self::Deployments => connection
                .watch_deployments(scope)
                .map(|update| rows(update, deployment_row))
                .boxed(),
            Self::StatefulSets => connection
                .watch_stateful_sets(scope)
                .map(|update| rows(update, stateful_set_row))
                .boxed(),
            Self::DaemonSets => connection
                .watch_daemon_sets(scope)
                .map(|update| rows(update, daemon_set_row))
                .boxed(),
            Self::ReplicaSets => connection
                .watch_replica_sets(scope)
                .map(|update| rows(update, replica_set_row))
                .boxed(),
            Self::Jobs => connection
                .watch_jobs(scope)
                .map(|update| rows(update, job_row))
                .boxed(),
            Self::CronJobs => connection
                .watch_cron_jobs(scope)
                .map(|update| rows(update, cron_job_row))
                .boxed(),
            Self::Services => connection
                .watch_services(scope)
                .map(|update| rows(update, service_row))
                .boxed(),
            Self::Ingresses => connection
                .watch_ingresses(scope)
                .map(|update| rows(update, ingress_row))
                .boxed(),
            Self::ConfigMaps => connection
                .watch_config_maps(scope)
                .map(|update| rows(update, config_map_row))
                .boxed(),
            Self::NetworkPolicies => connection
                .watch_network_policies(scope)
                .map(|update| rows(update, network_policy_row))
                .boxed(),
            Self::PodDisruptionBudgets => connection
                .watch_pod_disruption_budgets(scope)
                .map(|update| rows(update, pod_disruption_budget_row))
                .boxed(),
            Self::HorizontalPodAutoscalers => connection
                .watch_horizontal_pod_autoscalers(scope)
                .map(|update| rows(update, horizontal_pod_autoscaler_row))
                .boxed(),
            Self::ResourceQuotas => connection
                .watch_resource_quotas(scope)
                .map(|update| rows(update, resource_quota_row))
                .boxed(),
            Self::PersistentVolumeClaims => connection
                .watch_persistent_volume_claims(scope)
                .map(|update| rows(update, persistent_volume_claim_row))
                .boxed(),
            Self::PersistentVolumes => connection
                .watch_persistent_volumes()
                .map(|update| rows(update, persistent_volume_row))
                .boxed(),
            Self::StorageClasses => connection
                .watch_storage_classes()
                .map(|update| rows(update, storage_class_row))
                .boxed(),
            Self::Roles => connection
                .watch_roles(scope)
                .map(|update| rows(update, role_row))
                .boxed(),
            Self::ClusterRoles => connection
                .watch_cluster_roles()
                .map(|update| rows(update, cluster_role_row))
                .boxed(),
            Self::RoleBindings => connection
                .watch_role_bindings(scope)
                .map(|update| rows(update, role_binding_row))
                .boxed(),
            Self::ClusterRoleBindings => connection
                .watch_cluster_role_bindings()
                .map(|update| rows(update, cluster_role_binding_row))
                .boxed(),
            Self::ServiceAccounts => connection
                .watch_service_accounts(scope)
                .map(|update| rows(update, service_account_row))
                .boxed(),
            Self::Secrets => connection
                .watch_secrets(scope)
                .map(|update| rows(update, secret_row))
                .boxed(),
            Self::HelmReleases => connection
                .watch_helm_releases(scope)
                .map(|update| rows(update, helm_release_row))
                .boxed(),
        };
        Some(stream)
    }
}

/// Maps a snapshot to rows and passes a failure through.
#[cfg_attr(feature = "hotpath-profiling", hotpath::measure)]
fn rows<T>(update: WatchUpdate<T>, row: impl Fn(&T) -> KindRow) -> WatchUpdate<KindRow> {
    match update {
        WatchUpdate::Snapshot(items) => WatchUpdate::Snapshot(items.iter().map(row).collect()),
        WatchUpdate::Failed(error) => WatchUpdate::Failed(error),
    }
}

#[cfg(test)]
mod tests {
    use cluster::ClusterError;

    use super::*;
    use crate::kind_row::KindObject;
    use crate::status_tone::{StatusLabel, StatusTone};

    #[test]
    fn plural_slugs_round_trip() {
        for kind in ResourceKind::ALL {
            assert_eq!(ResourceKind::from_plural(kind.plural()), Some(kind));
        }
        assert_eq!(ResourceKind::from_plural("pods"), None);
    }

    #[test]
    fn has_monitor_matches_the_wireframe_kinds() {
        let with: Vec<_> = ResourceKind::ALL
            .into_iter()
            .filter(|kind| kind.has_monitor())
            .map(ResourceKind::plural)
            .collect();
        assert_eq!(
            with,
            [
                "deployments",
                "statefulsets",
                "daemonsets",
                "replicasets",
                "jobs"
            ]
        );
        assert!(!ResourceKind::CronJobs.has_monitor());
    }

    #[test]
    fn built_in_kind_icons_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        let icons = ResourceKind::ALL
            .into_iter()
            .map(ResourceKind::icon)
            .chain([POD_ICON, NODE_ICON]);
        for icon in icons {
            assert!(seen.insert(format!("{icon:?}")), "{icon:?} repeats");
        }
    }
    #[test]
    fn labels_round_trip() {
        for kind in ResourceKind::ALL {
            assert_eq!(ResourceKind::from_label(kind.label()), Some(kind));
        }
        assert_eq!(ResourceKind::from_label("Pods"), None);
    }

    #[test]
    fn short_names_cover_every_kind() {
        let mut seen = std::collections::HashSet::new();
        for kind in ResourceKind::ALL {
            let names = kind.short_names();
            assert!(!names.is_empty(), "{kind:?} has no short name");
            for name in names {
                assert!(seen.insert(*name), "{name} repeats");
            }
        }
    }

    #[test]
    fn cluster_scoped_kinds_are_listed() {
        let cluster_scoped: Vec<ResourceKind> = ResourceKind::ALL
            .into_iter()
            .filter(|kind| !kind.is_namespaced())
            .collect();
        assert_eq!(
            cluster_scoped,
            [
                ResourceKind::Namespaces,
                ResourceKind::PersistentVolumes,
                ResourceKind::StorageClasses,
                ResourceKind::ClusterRoles,
                ResourceKind::ClusterRoleBindings,
                ResourceKind::Crds
            ]
        );
    }

    #[test]
    fn every_kind_ends_with_a_right_aligned_age_column() {
        let age_name = |kind| match kind {
            ResourceKind::Events => "Last seen",
            ResourceKind::HelmReleases => "Updated",
            _ => "Age",
        };
        for kind in ResourceKind::ALL {
            let last = kind.columns().last().expect("kinds have columns");
            assert_eq!(last.name, age_name(kind));
            assert_eq!(last.align, Align::Right);
        }
    }

    #[test]
    fn display_names_are_the_kubernetes_kinds() {
        assert_eq!(ResourceKind::Deployments.display_name(), "Deployment");
        assert_eq!(ResourceKind::StatefulSets.display_name(), "StatefulSet");
        assert_eq!(ResourceKind::DaemonSets.display_name(), "DaemonSet");
        assert_eq!(ResourceKind::ReplicaSets.display_name(), "ReplicaSet");
        assert_eq!(ResourceKind::CronJobs.display_name(), "CronJob");
        assert_eq!(ResourceKind::ConfigMaps.display_name(), "ConfigMap");
        assert_eq!(ResourceKind::ClusterRoles.display_name(), "ClusterRole");
        assert_eq!(ResourceKind::Secrets.display_name(), "Secret");
    }

    #[test]
    fn a_release_is_not_titled_as_the_secret_that_stores_it() {
        assert_eq!(ResourceKind::HelmReleases.display_name(), "Helm release");
    }

    #[test]
    fn every_kind_has_a_display_name() {
        for kind in ResourceKind::ALL {
            assert!(!kind.display_name().is_empty(), "{}", kind.label());
        }
    }

    #[test]
    fn object_kinds_round_trip() {
        for kind in ResourceKind::ALL {
            // Releases are Secrets, and Secrets come first in `ALL`.
            let expected = if kind == ResourceKind::HelmReleases {
                ResourceKind::Secrets
            } else {
                kind
            };
            assert_eq!(
                ResourceKind::from_object_kind(kind.object_kind()),
                Some(expected)
            );
        }
        assert_eq!(ResourceKind::from_object_kind("Pod"), None);
    }

    #[test]
    fn releases_follow_secrets_in_all() {
        let position = |kind| ResourceKind::ALL.iter().position(|listed| *listed == kind);
        assert_eq!(
            position(ResourceKind::HelmReleases),
            position(ResourceKind::Secrets).map(|index| index + 1)
        );
        assert_eq!(
            ResourceKind::from_label("Releases"),
            Some(ResourceKind::HelmReleases)
        );
    }

    #[test]
    fn releases_have_no_count() {
        for kind in ResourceKind::ALL {
            assert_eq!(kind.has_count(), kind != ResourceKind::HelmReleases);
        }
    }

    #[test]
    fn events_count_holds_the_largest_abbreviated_count() {
        // `999.9k` is 6 mono characters of about 9.6 px, plus the 24 px of cell padding.
        let count = ResourceKind::Events
            .columns()
            .iter()
            .find(|column| column.name == "Count")
            .map(|column| column.width);
        assert!(count >= Some(6. * 9.6 + 24.), "{count:?}");
    }

    #[test]
    fn only_events_hide_the_name_column() {
        for kind in ResourceKind::ALL {
            match (kind, kind.name_column()) {
                (ResourceKind::Events, NameColumn::Hidden { flexible }) => {
                    let column = kind
                        .columns()
                        .get(flexible)
                        .expect("flexible column exists");
                    assert_eq!(column.name, "Message");
                }
                (ResourceKind::Events, NameColumn::Flexible) => panic!("Events hide Name"),
                (_, name_column) => assert_eq!(name_column, NameColumn::Flexible),
            }
        }
    }

    #[test]
    fn only_events_releases_and_crds_have_no_labels() {
        for kind in ResourceKind::ALL {
            let has_none = matches!(
                kind,
                ResourceKind::Events | ResourceKind::HelmReleases | ResourceKind::Crds
            );
            assert_eq!(kind.has_labels(), !has_none);
        }
    }
    #[test]
    fn crds_describe_the_definition_kind() {
        let kind = ResourceKind::Crds;
        assert_eq!(kind.label(), "CRDs");
        assert_eq!(kind.object_kind(), "CustomResourceDefinition");
        assert_eq!(
            kind.access_check(),
            Some(AccessCheck::ListCustomResourceDefinitions)
        );
        assert_eq!(kind.plural(), "customresourcedefinitions");
        let names: Vec<_> = kind_columns(kind)
            .iter()
            .map(|column| column.name)
            .collect();
        assert_eq!(
            names,
            ["Name", "Group", "Version", "Scope", "Instances", "Age"]
        );
        // Last in `ALL`, so `from_object_kind` and the sidebar order are unaffected.
        assert_eq!(ResourceKind::ALL.last(), Some(&kind));
    }

    #[test]
    fn port_forward_kinds_are_deployments_stateful_sets_services() {
        let kinds: Vec<ResourceKind> = ResourceKind::ALL
            .into_iter()
            .filter(|kind| kind.has_port_forward())
            .collect();
        assert_eq!(
            kinds,
            [
                ResourceKind::Deployments,
                ResourceKind::StatefulSets,
                ResourceKind::Services
            ]
        );
    }

    #[test]
    fn deployments_offer_port_forward_and_four_read_only_actions() {
        assert!(ResourceKind::Deployments.has_port_forward());
        assert!(!ResourceKind::Namespaces.has_port_forward());
        assert_eq!(ResourceKind::Deployments.read_only_actions().len(), 4);
        assert!(ResourceKind::Namespaces.read_only_actions().is_empty());
    }

    #[test]
    fn kind_actions_name_their_key_action() {
        let action_of = |kind: ResourceKind, label: &str| {
            kind.read_only_actions()
                .iter()
                .find(|item| item.label == label)
                .map(|item| item.action)
        };
        assert_eq!(
            action_of(ResourceKind::Deployments, "Scale…"),
            Some(Some(ResourceAction::Scale(ObjectKind::Deployment)))
        );
        assert_eq!(
            action_of(ResourceKind::Deployments, "Restart rollout"),
            Some(Some(ResourceAction::RestartRollout(ObjectKind::Deployment)))
        );
        // Edit YAML is one item of every editable kind's menu, not a kind action of its own; Edit
        // values is the keyed kind action of ConfigMaps and Secrets (spec 0047).
        assert_eq!(action_of(ResourceKind::ConfigMaps, "Edit"), None);
        assert_eq!(action_of(ResourceKind::Secrets, "Edit"), None);
        assert_eq!(
            action_of(ResourceKind::ConfigMaps, "Edit values…"),
            Some(Some(ResourceAction::EditValues(ObjectKind::ConfigMap)))
        );
        assert_eq!(
            action_of(ResourceKind::Secrets, "Edit values…"),
            Some(Some(ResourceAction::EditValues(ObjectKind::Secret)))
        );
        // Roll back has a unit action of its own, with no default key.
        assert_eq!(
            action_of(ResourceKind::Deployments, "Roll back…"),
            Some(Some(ResourceAction::RollBack))
        );
        // The wireframe gives a Helm release no key and 0032 does not ship it.
        assert_eq!(
            action_of(ResourceKind::HelmReleases, "Roll back…"),
            Some(None)
        );
    }

    #[test]
    fn rows_maps_snapshot_and_keeps_failure() {
        fn row(value: &u32) -> KindRow {
            KindRow {
                namespace: None,
                name: format!("row-{value}"),
                created_at: None,
                status: StatusLabel {
                    text: "Active".into(),
                    tone: StatusTone::Ok,
                },
                cells: Vec::new(),
                sections: Vec::new(),
                event: None,
                related_pods: None,
                labels: Vec::new(),
                object: KindObject::Plain,
            }
        }
        let WatchUpdate::Snapshot(mapped) = rows(WatchUpdate::Snapshot(vec![1, 2]), row) else {
            panic!("a snapshot must stay a snapshot");
        };
        let names: Vec<&str> = mapped.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["row-1", "row-2"]);

        let failure = WatchUpdate::Failed(ClusterError::TimedOut {
            context: "ctx".to_owned(),
            action: "watching deployments",
        });
        assert!(matches!(rows(failure, row), WatchUpdate::Failed(_)));
    }
}
