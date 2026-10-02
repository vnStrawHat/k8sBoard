//! The Kubernetes kinds that have an explorer screen, and the data that differs per kind.

use cluster::{
    AccessCheck, ClusterConnection, EventFilter, NamespaceScope, ObjectKind, WatchUpdate,
};
use futures::StreamExt as _;
use futures::stream::BoxStream;

use crate::batch_rows::{cron_job_row, job_row};
use crate::config_map_rows::config_map_row;
use crate::event_rows::event_rows;
use crate::kind_row::KindRow;
use crate::namespace_rows::namespace_row;
use crate::network_rows::{ingress_row, service_row};
use crate::workload_rows::{daemon_set_row, deployment_row, replica_set_row, stateful_set_row};

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
#[derive(Clone, Copy)]
pub(crate) struct KindColumn {
    pub(crate) name: &'static str,
    pub(crate) width: f32,
    pub(crate) align: Align,
}

pub(crate) const fn column(name: &'static str, width: f32, align: Align) -> KindColumn {
    KindColumn { name, width, align }
}

const AGE_COLUMN: KindColumn = column("Age", 70., Align::Right);

/// Everything that differs between kinds except the watch. A new kind adds one `static` here,
/// one arm in `spec`, and one arm in `watch_rows`.
struct KindSpec {
    label: &'static str,
    /// The Kubernetes `kind`, as an event's `involvedObject.kind` spells it.
    object: ObjectKind,
    name_column: NameColumn,
    /// Whether the drawer has a Labels section.
    has_labels: bool,
    singular: &'static str,
    plural: &'static str,
    badge: &'static str,
    is_namespaced: bool,
    access_check: AccessCheck,
    columns: &'static [KindColumn],
    read_only_actions: &'static [&'static str],
    delete_label: &'static str,
    has_port_forward: bool,
}

static NAMESPACES: KindSpec = KindSpec {
    label: "Namespaces",
    object: ObjectKind::Namespace,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "namespace",
    plural: "namespaces",
    badge: "Ns",
    is_namespaced: false,
    access_check: AccessCheck::ListNamespaces,
    columns: &[column("Status", 140., Align::Left), AGE_COLUMN],
    read_only_actions: &[],
    delete_label: "Delete namespace…",
    has_port_forward: false,
};

static EVENTS: KindSpec = KindSpec {
    label: "Events",
    object: ObjectKind::Event,
    name_column: NameColumn::Hidden { flexible: 3 },
    has_labels: false,
    singular: "event",
    plural: "events",
    badge: "Ev",
    is_namespaced: true,
    access_check: AccessCheck::ListEvents,
    columns: &[
        column("Type", 90., Align::Left),
        column("Reason", 170., Align::Left),
        column("Object", 260., Align::Left),
        column("Message", 280., Align::Left),
        column("Count", 80., Align::Right),
        column("Last seen", 80., Align::Right),
    ],
    read_only_actions: &[],
    delete_label: "Delete event…",
    has_port_forward: false,
};

static DEPLOYMENTS: KindSpec = KindSpec {
    label: "Deployments",
    object: ObjectKind::Deployment,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "deployment",
    plural: "deployments",
    badge: "De",
    is_namespaced: true,
    access_check: AccessCheck::ListDeployments,
    columns: &[
        column("Ready", 80., Align::Left),
        column("Up-to-date", 100., Align::Right),
        column("Available", 90., Align::Right),
        column("Strategy", 130., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &["Scale…", "Restart rollout", "Roll back…", "Pause rollout"],
    delete_label: "Delete deployment…",
    has_port_forward: true,
};

static STATEFUL_SETS: KindSpec = KindSpec {
    label: "StatefulSets",
    object: ObjectKind::StatefulSet,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "statefulset",
    plural: "statefulsets",
    badge: "Ss",
    is_namespaced: true,
    access_check: AccessCheck::ListStatefulSets,
    columns: &[
        column("Ready", 80., Align::Left),
        column("Service", 200., Align::Left),
        column("Update strategy", 140., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &["Scale…", "Restart rollout"],
    delete_label: "Delete statefulset…",
    has_port_forward: true,
};

static DAEMON_SETS: KindSpec = KindSpec {
    label: "DaemonSets",
    object: ObjectKind::DaemonSet,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "daemonset",
    plural: "daemonsets",
    badge: "Ds",
    is_namespaced: true,
    access_check: AccessCheck::ListDaemonSets,
    columns: &[
        column("Desired", 80., Align::Right),
        column("Current", 80., Align::Right),
        column("Ready", 80., Align::Right),
        column("Up-to-date", 100., Align::Right),
        column("Available", 90., Align::Right),
        column("Node selector", 200., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &["Restart rollout"],
    delete_label: "Delete daemonset…",
    has_port_forward: false,
};

static REPLICA_SETS: KindSpec = KindSpec {
    label: "ReplicaSets",
    object: ObjectKind::ReplicaSet,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "replicaset",
    plural: "replicasets",
    badge: "Rs",
    is_namespaced: true,
    access_check: AccessCheck::ListReplicaSets,
    columns: &[
        column("Desired", 80., Align::Right),
        column("Current", 80., Align::Right),
        column("Ready", 80., Align::Right),
        column("Owner", 220., Align::Left),
        AGE_COLUMN,
    ],
    // Scale belongs to the owning Deployment.
    read_only_actions: &[],
    delete_label: "Delete replicaset…",
    has_port_forward: false,
};

static JOBS: KindSpec = KindSpec {
    label: "Jobs",
    object: ObjectKind::Job,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "job",
    plural: "jobs",
    badge: "Jb",
    is_namespaced: true,
    access_check: AccessCheck::ListJobs,
    columns: &[
        column("Status", 120., Align::Left),
        column("Completions", 110., Align::Left),
        column("Duration", 90., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &["Re-run job"],
    delete_label: "Delete job…",
    has_port_forward: false,
};

static CRON_JOBS: KindSpec = KindSpec {
    label: "CronJobs",
    object: ObjectKind::CronJob,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "cronjob",
    plural: "cronjobs",
    badge: "Cj",
    is_namespaced: true,
    access_check: AccessCheck::ListCronJobs,
    columns: &[
        column("Schedule", 140., Align::Left),
        column("Suspend", 80., Align::Left),
        column("Active", 70., Align::Right),
        column("Last schedule", 120., Align::Right),
        column("Next run", 100., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &["Trigger now", "Suspend"],
    delete_label: "Delete cronjob…",
    has_port_forward: false,
};

static SERVICES: KindSpec = KindSpec {
    label: "Services",
    object: ObjectKind::Service,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "service",
    plural: "services",
    badge: "Sv",
    is_namespaced: true,
    access_check: AccessCheck::ListServices,
    columns: &[
        column("Type", 130., Align::Left),
        column("Cluster IP", 140., Align::Left),
        column("External IP", 200., Align::Left),
        column("Ports", 180., Align::Left),
        column("Endpoints", 100., Align::Right),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete service…",
    has_port_forward: true,
};

static INGRESSES: KindSpec = KindSpec {
    label: "Ingresses",
    object: ObjectKind::Ingress,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "ingress",
    plural: "ingresses",
    badge: "In",
    is_namespaced: true,
    access_check: AccessCheck::ListIngresses,
    columns: &[
        column("Class", 100., Align::Left),
        column("Hosts", 260., Align::Left),
        column("Address", 180., Align::Left),
        column("Ports", 80., Align::Left),
        AGE_COLUMN,
    ],
    read_only_actions: &[],
    delete_label: "Delete ingress…",
    has_port_forward: false,
};

static CONFIG_MAPS: KindSpec = KindSpec {
    label: "ConfigMaps",
    object: ObjectKind::ConfigMap,
    name_column: NameColumn::Flexible,
    has_labels: true,
    singular: "configmap",
    plural: "configmaps",
    badge: "Cm",
    is_namespaced: true,
    access_check: AccessCheck::ListConfigMaps,
    columns: &[column("Data", 70., Align::Right), AGE_COLUMN],
    read_only_actions: &["Edit"],
    delete_label: "Delete configmap…",
    has_port_forward: false,
};

/// The Name column of a kind that shows it, as wide as its minimum.
pub(crate) const NAME_COLUMN: KindColumn = column("Name", 200., Align::Left);

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
    pub(crate) const ALL: [Self; 11] = [
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

    /// Two letters in the drawer header.
    pub(crate) fn badge(self) -> &'static str {
        self.spec().badge
    }

    pub(crate) fn is_namespaced(self) -> bool {
        self.spec().is_namespaced
    }

    pub(crate) fn access_check(self) -> AccessCheck {
        self.spec().access_check
    }

    /// The columns after Name, or all of them when Name is hidden.
    pub(crate) fn columns(self) -> &'static [KindColumn] {
        self.spec().columns
    }

    /// The mutating menu items that are shown disabled.
    pub(crate) fn read_only_actions(self) -> &'static [&'static str] {
        self.spec().read_only_actions
    }

    pub(crate) fn delete_label(self) -> &'static str {
        self.spec().delete_label
    }

    pub(crate) fn has_port_forward(self) -> bool {
        self.spec().has_port_forward
    }

    /// The Kubernetes `kind`, such as `Deployment`.
    pub(crate) fn object(self) -> ObjectKind {
        self.spec().object
    }

    /// The Kubernetes `kind` name, as an event's `involvedObject.kind` spells it.
    pub(crate) fn object_kind(self) -> &'static str {
        self.object().name()
    }

    pub(crate) fn name_column(self) -> NameColumn {
        self.spec().name_column
    }

    pub(crate) fn has_labels(self) -> bool {
        self.spec().has_labels
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
    /// main thread only swaps a `Vec`. Namespaces are cluster-scoped and ignore `scope`. Only
    /// Events reads `events`.
    pub(crate) fn watch_rows(
        self,
        connection: &ClusterConnection,
        scope: NamespaceScope,
        events: EventFilter,
    ) -> BoxStream<'static, WatchUpdate<KindRow>> {
        match self {
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
        }
    }
}

/// Maps a snapshot to rows and passes a failure through.
fn rows<T>(update: WatchUpdate<T>, row: fn(&T) -> KindRow) -> WatchUpdate<KindRow> {
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
    fn labels_round_trip() {
        for kind in ResourceKind::ALL {
            assert_eq!(ResourceKind::from_label(kind.label()), Some(kind));
        }
        assert_eq!(ResourceKind::from_label("Pods"), None);
    }

    #[test]
    fn only_namespaces_is_cluster_scoped() {
        for kind in ResourceKind::ALL {
            assert_eq!(kind.is_namespaced(), kind != ResourceKind::Namespaces);
        }
    }

    #[test]
    fn every_kind_ends_with_a_right_aligned_age_column() {
        let age_name = |kind| {
            if kind == ResourceKind::Events {
                "Last seen"
            } else {
                "Age"
            }
        };
        for kind in ResourceKind::ALL {
            let last = kind.columns().last().expect("kinds have columns");
            assert_eq!(last.name, age_name(kind));
            assert_eq!(last.align, Align::Right);
        }
    }

    #[test]
    fn object_kinds_round_trip() {
        for kind in ResourceKind::ALL {
            assert_eq!(
                ResourceKind::from_object_kind(kind.object_kind()),
                Some(kind)
            );
        }
        assert_eq!(ResourceKind::from_object_kind("Pod"), None);
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
    fn only_events_have_no_labels() {
        for kind in ResourceKind::ALL {
            assert_eq!(kind.has_labels(), kind != ResourceKind::Events);
        }
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
