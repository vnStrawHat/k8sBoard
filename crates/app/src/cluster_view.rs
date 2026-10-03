//! The clusters the window views at once: an ordered list of slots, one live `ClusterSession` each,
//! and the pure plan that turns one set of clusters into the next without reconnecting the ones
//! that stay (spec 0027).

use std::fmt;

use cluster::ContextSummary;
use gpui_kit::{Entity, Subscription};

use crate::cluster_registry::{ClusterProfile, ClusterRef};
use crate::cluster_rows::SlotSession;
use crate::cluster_session::ClusterSession;
use crate::environment::Environment;

/// Bounds the connections, access reviews, and watches of the viewed set (budget.md).
pub(crate) const MAX_VIEWED_CLUSTERS: usize = 5;

/// A sixth cluster was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TooManyClusters;

impl fmt::Display for TooManyClusters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "View at most {MAX_VIEWED_CLUSTERS} clusters at once."
        )
    }
}

/// One viewed cluster and the session that serves it.
pub(crate) struct ViewSlot {
    pub(crate) cluster: ClusterRef,
    pub(crate) summary: ContextSummary,
    pub(crate) profile: ClusterProfile,
    /// The switcher text, which the Cluster column and the banners show.
    pub(crate) label: String,
    pub(crate) session: Entity<ClusterSession>,
    /// Whether this session was already seen Live, so its first Live edge acts once.
    pub(crate) has_reported_live: bool,
    /// Keeps the shell informed of the session; dropped with the slot.
    pub(crate) _observer: Subscription,
}

/// The viewed clusters in switcher display order, and which of them is the primary.
#[derive(Default)]
pub(crate) struct ClusterView {
    slots: Vec<ViewSlot>,
    primary: Option<ClusterRef>,
}

impl ClusterView {
    /// The slot the filters, `last_used`, and the trigger label follow.
    pub(crate) fn primary(&self) -> Option<&ViewSlot> {
        let primary = self.primary.as_ref()?;
        self.slots.iter().find(|slot| slot.cluster == *primary)
    }

    pub(crate) fn slots(&self) -> &[ViewSlot] {
        &self.slots
    }

    pub(crate) fn slots_mut(&mut self) -> &mut [ViewSlot] {
        &mut self.slots
    }

    pub(crate) fn slot_of(&self, cluster: &ClusterRef) -> Option<usize> {
        self.slots.iter().position(|slot| slot.cluster == *cluster)
    }

    /// Two or more clusters: the tables get their Cluster column.
    pub(crate) fn is_multi(&self) -> bool {
        self.slots.len() >= 2
    }

    pub(crate) fn clusters(&self) -> Vec<ClusterRef> {
        self.slots.iter().map(|slot| slot.cluster.clone()).collect()
    }

    /// What the table delegates keep of the slots, in slot order.
    pub(crate) fn sessions(&self) -> Vec<SlotSession> {
        let primary_label = self.primary().map(|slot| slot.label.clone());
        self.slots
            .iter()
            .map(|slot| SlotSession {
                cluster: slot.cluster.clone(),
                label: slot.label.clone(),
                environment: slot.profile.environment,
                is_primary: self.primary.as_ref() == Some(&slot.cluster),
                primary_label: primary_label.clone().unwrap_or_default(),
                is_multi: self.is_multi(),
                session: slot.session.clone(),
            })
            .collect()
    }

    /// The riskiest environment of the viewed clusters; the title-bar border only (decision 6).
    pub(crate) fn riskiest(&self) -> Option<Environment> {
        riskiest(self.slots.iter().map(|slot| slot.profile.environment))
    }

    /// The primary cluster, also before its slot exists (its session is still to connect).
    pub(crate) fn primary_cluster(&self) -> Option<&ClusterRef> {
        self.primary.as_ref()
    }

    pub(crate) fn set_primary(&mut self, primary: Option<ClusterRef>) {
        self.primary = primary;
    }

    /// Puts the slots in `display_order`, which a settings edit can change.
    pub(crate) fn reorder(&mut self, display_order: &[ClusterRef]) {
        self.slots.sort_by_key(|slot| {
            display_order
                .iter()
                .position(|cluster| *cluster == slot.cluster)
                .unwrap_or(usize::MAX)
        });
    }

    /// Adds `slot` at the position `display_order` gives it.
    pub(crate) fn insert(&mut self, slot: ViewSlot, display_order: &[ClusterRef]) {
        let position_of = |cluster: &ClusterRef| {
            display_order
                .iter()
                .position(|other| other == cluster)
                .unwrap_or(usize::MAX)
        };
        let at = self
            .slots
            .iter()
            .position(|other| position_of(&other.cluster) > position_of(&slot.cluster))
            .unwrap_or(self.slots.len());
        self.slots.insert(at, slot);
    }

    /// Removes and returns the slot of `cluster`, whose session is released with it.
    pub(crate) fn remove(&mut self, cluster: &ClusterRef) -> Option<ViewSlot> {
        let index = self.slot_of(cluster)?;
        Some(self.slots.remove(index))
    }

    /// Removes every slot, in order.
    pub(crate) fn take_all(&mut self) -> Vec<ViewSlot> {
        self.primary = None;
        std::mem::take(&mut self.slots)
    }
}

/// The highest of `environments` by risk.
pub(crate) fn riskiest(environments: impl Iterator<Item = Environment>) -> Option<Environment> {
    environments.max()
}

/// What turning the viewed set into another one changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewPlan {
    /// Sessions that stay, in display order.
    pub(crate) keep: Vec<ClusterRef>,
    /// Sessions to release first, in the order they are viewed now.
    pub(crate) release: Vec<ClusterRef>,
    /// Sessions to connect once the released ones are gone, in display order.
    pub(crate) connect: Vec<ClusterRef>,
    pub(crate) primary: ClusterRef,
}

impl ViewPlan {
    /// The same set: nothing is released or connected.
    pub(crate) fn is_empty(&self) -> bool {
        self.release.is_empty() && self.connect.is_empty()
    }
}

/// Pure. `current_primary` stays primary when it is in `wanted`; else the first in display
/// order. A wanted cluster missing from `display_order` is not viewable and is dropped (the
/// caller names it in a notice). `None` when nothing viewable is wanted.
pub(crate) fn plan_view(
    current: &[ClusterRef],
    current_primary: Option<&ClusterRef>,
    wanted: &[ClusterRef],
    display_order: &[ClusterRef],
) -> Result<Option<ViewPlan>, TooManyClusters> {
    let ordered: Vec<ClusterRef> = display_order
        .iter()
        .filter(|cluster| wanted.contains(cluster))
        .cloned()
        .collect();
    if ordered.len() > MAX_VIEWED_CLUSTERS {
        return Err(TooManyClusters);
    }
    let Some(first) = ordered.first() else {
        return Ok(None);
    };
    let primary = current_primary
        .filter(|primary| ordered.contains(primary))
        .unwrap_or(first)
        .clone();
    Ok(Some(ViewPlan {
        keep: ordered
            .iter()
            .filter(|cluster| current.contains(cluster))
            .cloned()
            .collect(),
        release: current
            .iter()
            .filter(|cluster| !ordered.contains(cluster))
            .cloned()
            .collect(),
        connect: ordered
            .iter()
            .filter(|cluster| !current.contains(cluster))
            .cloned()
            .collect(),
        primary,
    }))
}

#[cfg(test)]
#[path = "cluster_view_tests.rs"]
mod cluster_view_tests;
