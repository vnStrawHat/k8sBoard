//! The Kubernetes kinds that have an explorer screen, and the data that differs per kind.

use cluster::{AccessCheck, ClusterConnection, NamespaceScope, WatchUpdate};
use futures::StreamExt as _;
use futures::stream::BoxStream;

use crate::kind_row::KindRow;
use crate::namespace_rows::namespace_row;
use crate::workload_rows::deployment_row;

/// One kind with an explorer screen. Per-kind variation is data (the tables below) plus one
/// `match` in `watch_rows`; there is no trait.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ResourceKind {
    Namespaces,
    Deployments,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Right,
}

/// A column after the Name column.
pub(crate) struct KindColumn {
    pub(crate) name: &'static str,
    pub(crate) width: f32,
    pub(crate) align: Align,
}

const fn column(name: &'static str, width: f32, align: Align) -> KindColumn {
    KindColumn { name, width, align }
}

const AGE_COLUMN: KindColumn = column("Age", 70., Align::Right);

/// Everything that differs between kinds except the watch. A new kind adds one `static` here,
/// one arm in `spec`, and one arm in `watch_rows`.
struct KindSpec {
    label: &'static str,
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

static DEPLOYMENTS: KindSpec = KindSpec {
    label: "Deployments",
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

impl ResourceKind {
    pub(crate) const ALL: [Self; 2] = [Self::Namespaces, Self::Deployments];

    fn spec(self) -> &'static KindSpec {
        match self {
            Self::Namespaces => &NAMESPACES,
            Self::Deployments => &DEPLOYMENTS,
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

    /// The columns after Name.
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

    pub(crate) fn from_label(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.label() == text)
    }

    pub(crate) fn from_plural(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.plural() == text)
    }

    /// The only per-kind `match` over cluster calls: watch, then map to rows on tokio, so the
    /// main thread only swaps a `Vec`. Namespaces are cluster-scoped and ignore `scope`.
    pub(crate) fn watch_rows(
        self,
        connection: &ClusterConnection,
        scope: NamespaceScope,
    ) -> BoxStream<'static, WatchUpdate<KindRow>> {
        match self {
            Self::Namespaces => connection
                .watch_namespaces()
                .map(|update| rows(update, namespace_row))
                .boxed(),
            Self::Deployments => connection
                .watch_deployments(scope)
                .map(|update| rows(update, deployment_row))
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
    use crate::status_tone::{StatusLabel, StatusTone};

    #[test]
    fn plural_slugs_round_trip() {
        for kind in ResourceKind::ALL {
            assert_eq!(ResourceKind::from_plural(kind.plural()), Some(kind));
        }
        assert_eq!(ResourceKind::from_plural("pods"), None);
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
        for kind in ResourceKind::ALL {
            let last = kind.columns().last().expect("kinds have columns");
            assert_eq!(last.name, "Age");
            assert_eq!(last.align, Align::Right);
        }
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
                related_pods: None,
                labels: Vec::new(),
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
