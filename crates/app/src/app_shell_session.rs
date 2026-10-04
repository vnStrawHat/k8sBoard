//! The lifecycle of the one open cluster (specs 0026 and 0046): the session a switch connects, the
//! labels it follows from the catalog and the settings, what the table delegates and the graph
//! hold of it, and the first Live of the session.

use std::sync::Arc;

use cluster::{Kubeconfig, NamespaceScope};
use gpui_kit::{App, AppContext as _, Context};

use super::{AppShell, Screen, find_cluster};
use crate::active_session::ActiveSession;
use crate::cluster_registry::ClusterRef;
use crate::cluster_session::ClusterSession;
use crate::settings::AppSettings;

impl AppShell {
    /// A session for `cluster`, connecting; `None` once the cluster left every loaded kubeconfig.
    pub(super) fn new_session(
        &mut self,
        cluster: &ClusterRef,
        namespace: Option<NamespaceScope>,
        cx: &mut Context<Self>,
    ) -> Option<ActiveSession> {
        let kubeconfigs: Vec<Arc<Kubeconfig>> =
            self.catalog.read(cx).kubeconfigs().cloned().collect();
        let (kubeconfig, summary) = find_cluster(&kubeconfigs, cluster)?;
        let profile = AppSettings::get(cx).registry.profile(&summary);
        let label = self
            .cluster_label(cluster, cx)
            .unwrap_or_else(|| profile.display_name.clone());
        let kind = self.screen.kind();
        let cache = std::mem::take(&mut self.kind_cache);
        let session =
            cx.new(|cx| ClusterSession::new(kubeconfig, &summary, namespace, kind, cache, cx));
        // The new session is still connecting; it keeps the choice for `LiveCluster::start`.
        let is_overview = self.screen == Screen::Overview;
        let access_kind = self.screen.access_kind();
        #[cfg(feature = "screenshot")]
        let is_fixture = self.is_monitor_source_fixture;
        session.update(cx, |session, cx| {
            session.set_overview_visible(is_overview, cx);
            session.request_kind_access(access_kind, cx);
            #[cfg(feature = "screenshot")]
            if is_fixture {
                session.forget_metrics_source_for_fixture();
            }
        });
        let observed = cluster.clone();
        let observer = cx.observe(&session, move |shell, _, cx| {
            shell.on_session_changed(&observed, cx);
        });
        Some(ActiveSession {
            cluster: cluster.clone(),
            summary,
            profile,
            label,
            session,
            has_reported_live: false,
            _observer: observer,
        })
    }

    /// Hands the session to everything that reads rows or a session of its own: the table
    /// delegates, the Issues table, and the graph.
    pub(super) fn sync_view_sessions(&mut self, cx: &mut Context<Self>) {
        let session = self
            .active_session
            .as_ref()
            .map(ActiveSession::table_session);
        self.pod_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(session.clone());
            table.refresh(cx);
        });
        self.node_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(session.clone());
            table.refresh(cx);
        });
        self.kind_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(session.clone());
            table.refresh(cx);
        });
        self.issue_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(session.clone());
            cx.notify();
        });
        self.topology.update(cx, |view, cx| {
            view.set_session(session.map(|open| open.session), cx)
        });
        self.rebuild_visible_view(cx, |_| {});
        cx.notify();
    }

    /// Refreshes the label and the environment of the open cluster from the catalog and the
    /// settings; returns whether anything changed.
    pub(super) fn refresh_slot_labels(&mut self, cx: &App) -> bool {
        let Some(open) = self.active_session.as_mut() else {
            return false;
        };
        let rows: Vec<_> = self
            .catalog
            .read(cx)
            .groups(cx)
            .into_iter()
            .flat_map(|group| group.rows)
            .collect();
        let Some(row) = rows.iter().find(|row| row.cluster == open.cluster) else {
            return false;
        };
        if open.label == row.label && open.profile == row.profile {
            return false;
        }
        open.label.clone_from(&row.label);
        open.profile = row.profile.clone();
        true
    }

    /// The first Live of the session: it looks for node shell pods of other runs, and its cluster
    /// becomes the `last_used` one.
    pub(super) fn on_first_live(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) {
        self.sweep_leftovers(cluster, cx);
        if self.live_of(cluster, cx).is_none() {
            return;
        }
        let last_used = cluster.clone();
        // A file of a watched folder is remembered as it is now: if it changes, the next start
        // asks instead of running what someone wrote into the folder since.
        let stamp = self.catalog.read(cx).folder_stamp_of(&cluster.kubeconfig);
        AppSettings::update(cx, |settings| {
            settings.registry.last_used = Some(last_used);
            settings.registry.last_used_stamp = stamp;
        });
    }
}
