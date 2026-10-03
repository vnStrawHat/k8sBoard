use std::path::PathBuf;

use super::*;
use crate::settings::{SavedSort, TablePrefs};
use crate::table_filter::FilterChip;
use crate::table_layout::ColumnPlan;
use crate::table_sort::{SortDirection, TableSort};
use crate::table_view::TableView;

struct Row {
    namespace: &'static str,
    name: &'static str,
}

impl TableRow for Row {
    fn namespace(&self) -> Option<&str> {
        Some(self.namespace)
    }

    fn name(&self) -> &str {
        self.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        std::iter::empty()
    }

    fn tone(&self) -> StatusTone {
        StatusTone::Ok
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        match column {
            0 => CellValue::Text(Cow::Borrowed(self.name)),
            _ => CellValue::Absent,
        }
    }

    fn in_preset(&self, _: &FilterPreset) -> bool {
        true
    }
}

fn cluster(context: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("kube.yaml"),
        context: context.to_owned(),
    }
}

fn row(namespace: &'static str, name: &'static str) -> Row {
    Row { namespace, name }
}

/// The Cluster column of a two-column table.
const CLUSTER: usize = 2;

fn plan() -> ColumnPlan {
    ColumnPlan {
        specs: vec![
            column("Name", 100., Align::Left),
            column("Status", 80., Align::Left),
        ],
        flexible: 0,
        flexible_min: gpui_kit::px(100.),
        session_column: None,
    }
}

#[test]
fn merge_keeps_slot_order_and_addresses() {
    let (a, b) = (cluster("a"), cluster("b"));
    let first = [row("web", "api"), row("web", "db")];
    let second = [row("web", "cache")];
    let slots = [
        SlotRows {
            cluster: &a,
            label: "prod-eu",
            items: &first[..],
        },
        SlotRows {
            cluster: &b,
            label: "stg-b",
            items: &second[..],
        },
    ];
    let (merged, addresses) = merge_rows(&slots, CLUSTER);
    let names: Vec<_> = merged.iter().map(|row| row.name()).collect();
    assert_eq!(names, ["api", "db", "cache"]);
    assert_eq!(
        addresses,
        [
            RowAddress { slot: 0, item: 0 },
            RowAddress { slot: 0, item: 1 },
            RowAddress { slot: 1, item: 0 },
        ]
    );
    assert_eq!(merged_index(&addresses, 1, 0), Some(2));
    assert_eq!(merged_index(&addresses, 1, 1), None);
}

#[test]
fn same_pod_in_two_clusters_is_two_rows() {
    let (a, b) = (cluster("a"), cluster("b"));
    let one = [row("shop", "api-7")];
    let slots = [
        SlotRows {
            cluster: &a,
            label: "prod-eu",
            items: &one[..],
        },
        SlotRows {
            cluster: &b,
            label: "stg-b",
            items: &one[..],
        },
    ];
    let (merged, _) = merge_rows(&slots, CLUSTER);
    let mut view = TableView::default();
    view.rebuild(&merged, 3, jiff::Timestamp::UNIX_EPOCH);
    assert_eq!(view.rows(), [0, 1]);
    // Ticking one leaves the other: the identity carries the cluster.
    view.apply_check(&merged, crate::table_view::RowCheck::Toggle(0));
    assert!(view.is_checked(&merged[0]));
    assert!(!view.is_checked(&merged[1]));
}

#[test]
fn clustered_label_is_switcher_label() {
    let a = cluster("a");
    let items = [row("shop", "api-7")];
    let slots = [SlotRows {
        cluster: &a,
        label: "prod-eu · kube.yaml",
        items: &items[..],
    }];
    let (merged, _) = merge_rows(&slots, CLUSTER);
    assert!(matches!(
        merged[0].value(CLUSTER),
        CellValue::Text(text) if text == "prod-eu · kube.yaml"
    ));
    // The other columns are the row\'s own.
    assert!(matches!(merged[0].value(0), CellValue::Text(text) if text == "api-7"));
    assert_eq!(merged[0].cluster(), Some(&a));
}

#[test]
fn cluster_column_is_last_and_only_in_multi() {
    let single = plan();
    assert_eq!(single.session_column, None);
    assert_eq!(single.specs.len(), 2);
    let multi = plan().with_cluster_column();
    assert_eq!(multi.session_column, Some(2));
    assert_eq!(multi.specs.last().map(|spec| spec.name), Some("Cluster"));
    assert_eq!(multi.specs.len(), 3);
}

/// Two clusters whose rows interleave when sorted by name, to tell the columns apart.
fn two_clusters<'a>(
    a: &'a ClusterRef,
    b: &'a ClusterRef,
    first: &'a [Row],
    second: &'a [Row],
) -> Vec<Clustered<'a, Row>> {
    let slots = [
        SlotRows {
            cluster: a,
            label: "stg-b",
            items: first,
        },
        SlotRows {
            cluster: b,
            label: "prod-eu",
            items: second,
        },
    ];
    merge_rows(&slots, CLUSTER).0
}

#[test]
fn cluster_column_sorts_by_label() {
    let (a, b) = (cluster("a"), cluster("b"));
    let (first, second) = ([row("x", "one")], [row("x", "two")]);
    let merged = two_clusters(&a, &b, &first, &second);
    let mut view = TableView::default();
    view.sort = Some(TableSort {
        column: CLUSTER,
        direction: SortDirection::Ascending,
    });
    view.rebuild(&merged, 3, jiff::Timestamp::UNIX_EPOCH);
    // `prod-eu` sorts before `stg-b`, though its cluster comes second.
    assert_eq!(view.rows(), [1, 0]);
}

#[test]
fn equals_chip_filters_one_cluster() {
    let (a, b) = (cluster("a"), cluster("b"));
    let (first, second) = ([row("x", "one"), row("x", "three")], [row("x", "two")]);
    let merged = two_clusters(&a, &b, &first, &second);
    let mut view = TableView::default();
    view.filter.set_equals(CLUSTER, "Cluster", "prod-eu");
    assert!(matches!(
        view.filter.chips.first(),
        Some(FilterChip::Equals {
            column: CLUSTER,
            ..
        })
    ));
    view.rebuild(&merged, 3, jiff::Timestamp::UNIX_EPOCH);
    assert_eq!(view.rows(), [2]);
}

#[test]
fn quick_filter_matches_the_cluster_label() {
    let (a, b) = (cluster("a"), cluster("b"));
    let (first, second) = ([row("x", "one")], [row("x", "two")]);
    let merged = two_clusters(&a, &b, &first, &second);
    let mut view = TableView::default();
    view.filter.text = "prod".to_owned();
    view.rebuild(&merged, 3, jiff::Timestamp::UNIX_EPOCH);
    assert_eq!(view.rows(), [1]);
    // In single mode the column does not exist, so its text is not searched.
    view.rebuild(&merged, 2, jiff::Timestamp::UNIX_EPOCH);
    assert!(view.rows().is_empty());
}

#[test]
fn prefs_skip_the_cluster_column() {
    let plan = plan().with_cluster_column();
    let mut view = TableView::default();
    view.sort = Some(TableSort {
        column: CLUSTER,
        direction: SortDirection::Descending,
    });
    view.hidden.insert(CLUSTER);
    let prefs = view.prefs(&plan);
    assert_eq!(prefs, TablePrefs::default());
    // A sort on a real column is saved as before.
    view.sort = Some(TableSort {
        column: 0,
        direction: SortDirection::Ascending,
    });
    assert_eq!(
        view.prefs(&plan).sort,
        Some(SavedSort {
            column: "Name".to_owned(),
            direction: SortDirection::Ascending,
        })
    );
}

#[test]
fn leaving_multi_drops_cluster_hidden_and_sort() {
    let mut view = TableView::default();
    view.sort = Some(TableSort {
        column: CLUSTER,
        direction: SortDirection::Ascending,
    });
    view.hidden.insert(CLUSTER);
    view.drop_column(CLUSTER);
    assert!(view.hidden.is_empty());
    assert_eq!(view.sort, None);
    // A sort on another column stays.
    view.sort = Some(TableSort {
        column: 0,
        direction: SortDirection::Ascending,
    });
    view.drop_column(CLUSTER);
    assert!(view.sort.is_some());
}

#[test]
fn hidden_prefs_by_name_survive_multi() {
    let saved = TablePrefs {
        sort: Some(SavedSort {
            column: "Name".to_owned(),
            direction: SortDirection::Descending,
        }),
        hidden: vec!["Status".to_owned()],
    };
    // The Cluster column is appended last, so every saved name keeps its logical index.
    let mut single = TableView::default();
    single.apply_prefs(&saved, &plan());
    let mut multi = TableView::default();
    multi.apply_prefs(&saved, &plan().with_cluster_column());
    assert_eq!(single.sort, multi.sort);
    assert_eq!(single.hidden, multi.hidden);
    assert_eq!(multi.hidden.iter().copied().collect::<Vec<_>>(), [1]);
}
