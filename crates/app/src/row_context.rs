//! What the table delegates and the row menus keep of the open cluster (spec 0046): the session the
//! rows come from, and the weak handle a menu holds so that it never keeps a released session
//! alive.

use gpui_kit::{App, Entity, WeakEntity};

use crate::cluster_registry::ClusterRef;
use crate::cluster_session::ClusterSession;
use crate::table_selection::{ClusterObject, ResourceKey};

/// What a table delegate keeps of the open cluster: the session its rows come from.
#[derive(Clone)]
pub(crate) struct TableSession {
    pub(crate) cluster: ClusterRef,
    pub(crate) session: Entity<ClusterSession>,
}

impl TableSession {
    /// What a row menu keeps of the cluster, captured when the menu is built.
    pub(crate) fn row_context(&self, cx: &App) -> RowContext {
        RowContext {
            cluster: self.cluster.clone(),
            context: self.session.read(cx).context().to_owned(),
            session: self.session.downgrade(),
        }
    }
}

/// The cluster a row menu acts on. Weak: a menu that stays open must not keep a released session
/// alive, and a menu whose session is gone acts on nothing (`resource_actions::guarded`).
#[derive(Clone)]
pub(crate) struct RowContext {
    pub(crate) cluster: ClusterRef,
    /// The kubeconfig context name, for `kubectl --context`.
    pub(crate) context: String,
    pub(crate) session: WeakEntity<ClusterSession>,
}

impl RowContext {
    /// `key` in this row's cluster.
    pub(crate) fn object(&self, key: ResourceKey) -> ClusterObject {
        ClusterObject::new(self.cluster.clone(), key)
    }
}
