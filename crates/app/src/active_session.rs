//! The one open cluster of the window (spec 0046): its context, profile, label, and the live
//! `ClusterSession` that serves it. A switch releases it and connects the next (spec 0026).

use cluster::{ClusterConnection, ContextSummary};
use gpui_kit::{Entity, Global, Subscription, WeakEntity};

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

/// The live connection of the open cluster, for the Settings window: it is a window of its own and
/// cannot reach the shell. Set when the session goes Live (again after a reconnect) and removed on
/// a switch and when the shell is released; the Metrics page reads it and never opens a connection
/// of its own.
pub(crate) struct ActiveConnection {
    pub(crate) cluster: ClusterRef,
    pub(crate) label: String,
    pub(crate) connection: ClusterConnection,
    /// The session, for the state of its metrics source; weak, so the window never keeps it alive.
    pub(crate) session: WeakEntity<ClusterSession>,
    /// Names the connection, so a reconnect is told from the same one.
    pub(crate) generation: u64,
}

impl Global for ActiveConnection {}
