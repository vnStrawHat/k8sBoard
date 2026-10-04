//! The one open cluster of the window (spec 0046): its context, profile, label, and the live
//! `ClusterSession` that serves it. A switch releases it and connects the next (spec 0026).

use cluster::ContextSummary;
use gpui_kit::{Entity, Subscription};

use crate::cluster_registry::{ClusterProfile, ClusterRef};
use crate::cluster_session::ClusterSession;
use crate::row_context::TableSession;

/// The open cluster and the session that serves it.
pub(crate) struct ActiveSession {
    pub(crate) cluster: ClusterRef,
    pub(crate) summary: ContextSummary,
    pub(crate) profile: ClusterProfile,
    /// The switcher text, which the notices and the shell tabs name the cluster by.
    pub(crate) label: String,
    pub(crate) session: Entity<ClusterSession>,
    /// Whether this session was already seen Live, so its first Live edge acts once.
    pub(crate) has_reported_live: bool,
    /// Keeps the shell informed of the session; dropped with it.
    pub(crate) _observer: Subscription,
}

impl ActiveSession {
    /// What the table delegates, the Issues table, and the graph keep of it.
    pub(crate) fn table_session(&self) -> TableSession {
        TableSession {
            cluster: self.cluster.clone(),
            session: self.session.clone(),
        }
    }
}
