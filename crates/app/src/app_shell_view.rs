//! The lifecycle of the viewed clusters (spec 0027): applying a set of clusters, the sessions that
//! stay, leave, or connect, the scope every viewed cluster shares, and what the table delegates,
//! the graph, and the dock hold of the sessions.

use std::sync::Arc;

use cluster::{Kubeconfig, NamespaceScope};
use gpui_kit::{App, AppContext as _, Context};

#[cfg(test)]
use super::ViewConnectCheck;
use super::{AppShell, Screen, find_cluster, record_leaving};
use crate::cluster_registry::{ClusterRef, remember_scope, start_scope};
use crate::cluster_session::ClusterSession;
use crate::cluster_view::{ViewSlot, plan_view};
use crate::namespace_picker::NamespacePickerState;
use crate::settings::AppSettings;

impl AppShell {
    /// Views `wanted`: one cluster is a 0026 switch; two or more keep the sessions that stay,
    /// release the removed ones, and connect the added ones. At most `MAX_VIEWED_CLUSTERS`.
    pub(crate) fn view_clusters(&mut self, wanted: &[ClusterRef], cx: &mut Context<Self>) {
        // One cluster that is not part of a multi view, or is the whole of it: a plain switch.
        if let [only] = wanted
            && !self.view.is_multi()
        {
            self.switch_cluster(only, cx);
            return;
        }
        let leaving: Vec<ClusterRef> = self
            .view
            .clusters()
            .into_iter()
            .filter(|cluster| !wanted.contains(cluster))
            .collect();
        let work = self.leaving_work(&leaving, cx);
        if work.is_empty() {
            self.apply_view(wanted, None, cx);
            return;
        }
        let wanted = wanted.to_vec();
        self.confirm_leaving(
            work,
            move |shell, cx| shell.apply_view(&wanted, None, cx),
            cx,
        );
    }

    pub(super) fn apply_view(
        &mut self,
        wanted: &[ClusterRef],
        requested: Option<NamespaceScope>,
        cx: &mut Context<Self>,
    ) {
        let kubeconfigs: Vec<Arc<Kubeconfig>> =
            self.catalog.read(cx).kubeconfigs().cloned().collect();
        let (known, missing): (Vec<ClusterRef>, Vec<ClusterRef>) = wanted
            .iter()
            .cloned()
            .partition(|cluster| find_cluster(&kubeconfigs, cluster).is_some());
        if let Some(gone) = missing.first() {
            self.switch_notice = Some(format!("'{}' is no longer in its kubeconfig", gone.context));
        }
        let display_order = self.display_order(cx);
        let current = self.view.clusters();
        let current_primary = self.view.primary().map(|slot| slot.cluster.clone());
        let plan = match plan_view(&current, current_primary.as_ref(), &known, &display_order) {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                cx.notify();
                return;
            }
            Err(error) => {
                self.switch_notice = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        if plan.is_empty() {
            cx.notify();
            return;
        }
        self.context_error = None;
        self.view_request += 1;
        let request = self.view_request;
        for slot in self.view.slots() {
            if let Some(live) = slot.session.read(cx).live() {
                remember_scope(
                    &mut self.scope_memory,
                    slot.cluster.clone(),
                    live.scope.clone(),
                );
            }
        }
        self.view_scope = self.scope_for_view(&plan.primary, requested, cx);
        // Release first: the deferred connect below runs after the entities are gone.
        #[cfg(test)]
        {
            self.released_sessions = plan
                .release
                .iter()
                .filter_map(|cluster| self.view.slot_of(cluster))
                .map(|index| self.view.slots()[index].session.downgrade())
                .collect();
        }
        let mut released = Vec::new();
        for cluster in &plan.release {
            if let Some(slot) = self.release_slot(cluster, cx) {
                released.push(slot);
            }
        }
        self.view.set_primary(Some(plan.primary.clone()));
        self.view.reorder(&display_order);
        let primary_summary = find_cluster(&kubeconfigs, &plan.primary).map(|(_, summary)| summary);
        if let Some(summary) = primary_summary {
            self.active = Some(summary);
        }
        let is_new_primary = current_primary.as_ref() != Some(&plan.primary);
        // A new primary is a new context (decision 11); the first apply keeps the launch request.
        if is_new_primary && current_primary.is_some() {
            self.clear_all_filters(cx);
            self.namespace_picker = NamespacePickerState::default();
            self.pending_reveal = None;
        }
        if is_new_primary && self.slot_live(&plan.primary, cx).is_some() {
            let cluster = plan.primary.clone();
            AppSettings::update(cx, |settings| settings.registry.last_used = Some(cluster));
        }
        self.sync_view_sessions(cx);
        drop(released);
        if plan.connect.is_empty() {
            return;
        }
        let (shell, connect) = (cx.weak_entity(), plan.connect.clone());
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.connect_slots(request, &connect, &display_order, cx);
            });
        });
    }

    /// The scope every viewed cluster gets: the request, else the primary's live scope when it is
    /// viewed and live, else what the primary starts with, else the scope already in force.
    fn scope_for_view(
        &self,
        primary: &ClusterRef,
        requested: Option<NamespaceScope>,
        cx: &App,
    ) -> Option<NamespaceScope> {
        if requested.is_some() {
            return requested;
        }
        match self.view.slot_of(primary) {
            Some(_) => match self.slot_live(primary, cx) {
                Some(live) => Some(live.scope.clone()),
                None => self.view_scope.clone(),
            },
            None => {
                let kubeconfigs: Vec<Arc<Kubeconfig>> =
                    self.catalog.read(cx).kubeconfigs().cloned().collect();
                let (_, summary) = find_cluster(&kubeconfigs, primary)?;
                let profile = AppSettings::get(cx).registry.profile(&summary);
                start_scope(&self.scope_memory, primary, &profile)
            }
        }
    }

    /// Takes `cluster` out of the view with what depends on it: its log tabs, the drawer when the
    /// subject is there. The returned slot is dropped by the caller once the delegates let go.
    fn release_slot(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) -> Option<ViewSlot> {
        self.close_edit_of(cluster, cx);
        if self
            .selected
            .as_ref()
            .is_some_and(|object| object.cluster == *cluster)
        {
            self.clear_selection(cx);
        }
        self.dock
            .update(cx, |dock, cx| dock.close_tabs_of(cluster, cx));
        if self.subject_cluster.as_ref() == Some(cluster) {
            self.subject_cluster = None;
        }
        let slot = self.view.remove(cluster)?;
        record_leaving(&slot, &mut self.scope_memory, &mut self.switcher, cx);
        Some(slot)
    }

    /// The deferred half of an apply: connects the added clusters, unless a newer apply replaced
    /// this one.
    fn connect_slots(
        &mut self,
        request: u64,
        clusters: &[ClusterRef],
        display_order: &[ClusterRef],
        cx: &mut Context<Self>,
    ) {
        if request != self.view_request {
            return;
        }
        #[cfg(test)]
        self.view_connects.push(ViewConnectCheck {
            released_gone: self
                .released_sessions
                .iter()
                .all(|released| released.upgrade().is_none()),
            slots: self.view.slots().len(),
            connects: clusters.len(),
        });
        for cluster in clusters {
            #[cfg(test)]
            self.connected_scopes.push(self.view_scope.clone());
            let namespace = self.view_scope.clone();
            match self.new_slot(cluster, namespace, cx) {
                Some(slot) => self.view.insert(slot, display_order),
                None => {
                    self.switch_notice = Some(format!(
                        "'{}' is no longer in its kubeconfig",
                        cluster.context
                    ));
                }
            }
        }
        self.sync_view_sessions(cx);
    }

    /// A session for `cluster`, connecting; `None` once the cluster left every loaded kubeconfig.
    pub(super) fn new_slot(
        &mut self,
        cluster: &ClusterRef,
        namespace: Option<NamespaceScope>,
        cx: &mut Context<Self>,
    ) -> Option<ViewSlot> {
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
        let is_overview = self.screen == Screen::Overview && self.is_primary(cluster);
        let access_kind = self.screen.access_kind();
        session.update(cx, |session, cx| {
            session.set_overview_visible(is_overview, cx);
            session.request_kind_access(access_kind, cx);
        });
        let observed = cluster.clone();
        let observer = cx.observe(&session, move |shell, _, cx| {
            shell.on_slot_changed(&observed, cx);
        });
        Some(ViewSlot {
            cluster: cluster.clone(),
            summary,
            profile,
            label,
            session,
            has_reported_live: false,
            _observer: observer,
        })
    }

    /// Hands the sessions of the slots to everything that reads rows or a session of its own: the
    /// table delegates, the Issues table, the graph, and the log dock.
    pub(super) fn sync_view_sessions(&mut self, cx: &mut Context<Self>) {
        let sessions = self.view.sessions();
        let primary = sessions.iter().find(|slot| slot.is_primary).cloned();
        let is_multi = self.view.is_multi();
        self.pod_table.update(cx, |table, cx| {
            table.delegate_mut().set_sessions(sessions.clone());
            table.refresh(cx);
        });
        self.node_table.update(cx, |table, cx| {
            table.delegate_mut().set_sessions(sessions.clone());
            table.refresh(cx);
        });
        self.kind_table.update(cx, |table, cx| {
            table.delegate_mut().set_sessions(sessions);
            table.refresh(cx);
        });
        self.issue_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(primary.clone());
            cx.notify();
        });
        // The graph draws the primary cluster only.
        self.topology.update(cx, |view, cx| {
            view.set_session(primary.map(|slot| slot.session), cx)
        });
        self.dock
            .update(cx, |dock, cx| dock.set_multi(is_multi, cx));
        self.rebuild_visible_view(cx, |_| {});
        cx.notify();
    }

    /// Refreshes the label and the environment of every slot from the catalog and the settings;
    /// returns whether anything changed.
    pub(super) fn refresh_slot_labels(&mut self, cx: &App) -> bool {
        if self.view.slots().is_empty() {
            return false;
        }
        let rows: Vec<_> = self
            .catalog
            .read(cx)
            .groups(cx)
            .into_iter()
            .flat_map(|group| group.rows)
            .collect();
        let mut is_changed = false;
        for slot in self.view.slots_mut() {
            let Some(row) = rows.iter().find(|row| row.cluster == slot.cluster) else {
                continue;
            };
            if slot.label != row.label || slot.profile != row.profile {
                slot.label.clone_from(&row.label);
                slot.profile = row.profile.clone();
                is_changed = true;
            }
        }
        is_changed
    }

    /// Every loaded cluster in the order the switcher lists them.
    fn display_order(&self, cx: &App) -> Vec<ClusterRef> {
        self.catalog
            .read(cx)
            .groups(cx)
            .into_iter()
            .flat_map(|group| group.rows)
            .map(|row| row.cluster)
            .collect()
    }

    /// The first Live of a slot: the primary becomes the `last_used` cluster and decides the scope
    /// of the whole view; any other slot takes the view scope when it differs (decision 12).
    pub(super) fn on_first_live(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) {
        let Some(live_scope) = self.slot_live(cluster, cx).map(|live| live.scope.clone()) else {
            return;
        };
        if self.is_primary(cluster) {
            let last_used = cluster.clone();
            AppSettings::update(cx, |settings| settings.registry.last_used = Some(last_used));
            self.view_scope = Some(live_scope);
            self.reapply_view_scope(cx);
            return;
        }
        let primary_has_reported_live = self
            .view
            .primary()
            .is_some_and(|slot| slot.has_reported_live);
        let Some(scope) = self.view_scope.clone() else {
            return;
        };
        if primary_has_reported_live && scope != live_scope {
            self.set_slot_scope(cluster, scope, cx);
        }
    }

    /// Gives every live slot the view scope.
    fn reapply_view_scope(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.view_scope.clone() else {
            return;
        };
        let stale: Vec<ClusterRef> = self
            .view
            .slots()
            .iter()
            .filter(|slot| {
                slot.session
                    .read(cx)
                    .live()
                    .is_some_and(|live| live.scope != scope)
            })
            .map(|slot| slot.cluster.clone())
            .collect();
        for cluster in stale {
            self.set_slot_scope(&cluster, scope.clone(), cx);
        }
    }

    fn set_slot_scope(
        &mut self,
        cluster: &ClusterRef,
        scope: NamespaceScope,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.slot_session(cluster).cloned() {
            session.update(cx, |session, cx| session.set_scope(scope, cx));
        }
    }

    /// The Retry of one failed cluster's banner.
    pub(crate) fn retry_cluster(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) {
        if let Some(session) = self.slot_session(cluster).cloned() {
            session.update(cx, |session, cx| session.retry(cx));
        }
    }

    /// "Remove from view": the other viewed clusters stay, with their sessions.
    pub(crate) fn remove_from_view(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) {
        let remaining: Vec<ClusterRef> = self
            .view
            .clusters()
            .into_iter()
            .filter(|other| other != cluster)
            .collect();
        if remaining.is_empty() {
            return;
        }
        let work = self.leaving_work(std::slice::from_ref(cluster), cx);
        if work.is_empty() {
            self.apply_view(&remaining, None, cx);
            return;
        }
        self.confirm_leaving(
            work,
            move |shell, cx| shell.apply_view(&remaining, None, cx),
            cx,
        );
    }
}
