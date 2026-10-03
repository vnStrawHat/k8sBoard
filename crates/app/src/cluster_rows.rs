//! The rows of several viewed clusters in one table (spec 0027): each row remembers its cluster, and
//! an address maps a merged row back to the slot and the item of that cluster's own list. Single
//! mode is the same code with one slot.

use std::borrow::Cow;

use gpui_kit::{App, Entity, WeakEntity};

use crate::cluster_registry::ClusterRef;
use crate::cluster_session::ClusterSession;
use crate::environment::Environment;
use crate::resource_kind::{Align, KindColumn, column};
use crate::status_tone::StatusTone;
use crate::table_filter::FilterPreset;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::table_view::{CellValue, TableRow};

/// Appended after the logical columns of every table in multi mode, so the other indices stay.
pub(crate) const CLUSTER_COLUMN: KindColumn = column("Cluster", 170., Align::Left);

/// A row with the cluster it came from. The Cluster column reads `label`.
pub(crate) struct Clustered<'a, T> {
    pub(crate) cluster: &'a ClusterRef,
    /// The switcher text of the cluster.
    pub(crate) label: &'a str,
    /// The logical index of the Cluster column.
    pub(crate) column: usize,
    pub(crate) item: &'a T,
}

impl<T: TableRow> TableRow for Clustered<'_, T> {
    fn namespace(&self) -> Option<&str> {
        self.item.namespace()
    }

    fn name(&self) -> &str {
        self.item.name()
    }

    fn cluster(&self) -> Option<&ClusterRef> {
        Some(self.cluster)
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        self.item.labels()
    }

    fn tone(&self) -> StatusTone {
        self.item.tone()
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        if column == self.column {
            return CellValue::Text(Cow::Borrowed(self.label));
        }
        self.item.value(column)
    }

    fn in_preset(&self, preset: &FilterPreset) -> bool {
        self.item.in_preset(preset)
    }
}

/// What a table delegate keeps of one viewed slot: the session its rows come from, and what the
/// Cluster column shows for them.
#[derive(Clone)]
pub(crate) struct SlotSession {
    pub(crate) cluster: ClusterRef,
    /// The switcher text of the cluster.
    pub(crate) label: String,
    pub(crate) environment: Environment,
    /// The cluster Topology, Overview, and Issues draw while several are viewed.
    pub(crate) is_primary: bool,
    /// The switcher text of the primary cluster, which Topology draws.
    pub(crate) primary_label: String,
    /// Several clusters are viewed, so the tables have a Cluster column.
    pub(crate) is_multi: bool,
    pub(crate) session: Entity<ClusterSession>,
}

impl SlotSession {
    /// What a row menu keeps of the row's cluster, captured when the menu is built: a slot index
    /// would shift when the viewed set changes.
    pub(crate) fn row_context(&self, cx: &App) -> RowContext {
        RowContext {
            cluster: self.cluster.clone(),
            label: self.label.clone(),
            context: self.session.read(cx).context().to_owned(),
            is_primary: self.is_primary,
            primary_label: self.primary_label.clone(),
            is_multi: self.is_multi,
            session: self.session.downgrade(),
        }
    }
}

/// The cluster a row menu acts on. Weak: a menu that stays open must not keep a released session
/// alive.
#[derive(Clone)]
pub(crate) struct RowContext {
    pub(crate) cluster: ClusterRef,
    /// The switcher text of the cluster.
    pub(crate) label: String,
    /// The kubeconfig context name, for `kubectl --context`.
    pub(crate) context: String,
    /// The cluster Topology draws.
    pub(crate) is_primary: bool,
    /// The switcher text of the primary cluster, which Topology draws.
    pub(crate) primary_label: String,
    /// Several clusters are viewed, so the rows can be filtered by cluster.
    pub(crate) is_multi: bool,
    pub(crate) session: WeakEntity<ClusterSession>,
}

impl RowContext {
    /// `key` in this row's cluster.
    pub(crate) fn object(&self, key: ResourceKey) -> ClusterObject {
        ClusterObject::new(self.cluster.clone(), key)
    }
}

/// Where a merged row came from: the slot, and the item index in that slot's own list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowAddress {
    pub(crate) slot: u16,
    pub(crate) item: u32,
}

/// One slot's rows and what the Cluster column shows for them.
pub(crate) struct SlotRows<'a, T> {
    pub(crate) cluster: &'a ClusterRef,
    pub(crate) label: &'a str,
    pub(crate) items: &'a [T],
}

/// The rows of every slot, slot after slot, and the address of each. `column` is the logical index
/// of the Cluster column. The merged index of a row is its index in both vectors.
pub(crate) fn merge_rows<'a, T>(
    slots: &[SlotRows<'a, T>],
    column: usize,
) -> (Vec<Clustered<'a, T>>, Vec<RowAddress>) {
    let total = slots.iter().map(|slot| slot.items.len()).sum();
    let mut merged = Vec::with_capacity(total);
    let mut addresses = Vec::with_capacity(total);
    for (slot_index, slot) in slots.iter().enumerate() {
        for (item_index, item) in slot.items.iter().enumerate() {
            merged.push(Clustered {
                cluster: slot.cluster,
                label: slot.label,
                column,
                item,
            });
            addresses.push(RowAddress {
                slot: u16::try_from(slot_index).unwrap_or(u16::MAX),
                item: u32::try_from(item_index).unwrap_or(u32::MAX),
            });
        }
    }
    (merged, addresses)
}

/// `merge_rows` over the sessions of a delegate, with `rows[i]` the rows of `sessions[i]`.
pub(crate) fn merge_slot_rows<'a, T>(
    sessions: &'a [SlotSession],
    rows: &'a [Vec<T>],
    column: usize,
) -> (Vec<Clustered<'a, T>>, Vec<RowAddress>) {
    let slots: Vec<SlotRows<'a, T>> = sessions
        .iter()
        .zip(rows)
        .map(|(slot, items)| SlotRows {
            cluster: &slot.cluster,
            label: &slot.label,
            items,
        })
        .collect();
    merge_rows(&slots, column)
}

/// The merged index of item `item` of slot `slot`; `None` when the slot has no such item.
pub(crate) fn merged_index(addresses: &[RowAddress], slot: usize, item: usize) -> Option<usize> {
    addresses
        .iter()
        .position(|address| usize::from(address.slot) == slot && address.item as usize == item)
}

#[cfg(test)]
#[path = "cluster_rows_tests.rs"]
mod cluster_rows_tests;
