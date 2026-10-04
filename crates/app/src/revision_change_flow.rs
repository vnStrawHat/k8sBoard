//! A click on a Deployment row of the Overview timeline (spec 0041, wireframe W3 note 4): one
//! ReplicaSet LIST on tokio, then the 0039 revision diff dialog of the revision the row reports.
//! A child of `app_shell` because it reads the open session and owns `AppShell.revision_lookup`.
//! Read-only; nothing here logs a template or an error body.

use cluster::{AccessCheck, ClusterConnection, ClusterError, ReplicaSetSummary};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, Context, ParentElement as _, Window, px};

use super::write_flow::notify;
use super::{AppShell, REVISION_DIFF_WIDTH};
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::{AccessState, error_text};
use crate::revision_diff::{
    RevisionDiffView, RevisionSide, change_pair, dialog_body, diff_request, revision_list,
};
use crate::table_selection::ResourceKey;
use crate::yaml_view::object_ref;

impl AppShell {
    /// Lists the Deployment's revisions on tokio, then opens the 0039 dialog: `named` (the ReplicaSet
    /// the event names) against its predecessor when both are listed, else the newest and the one
    /// before. Fewer than two numbered revisions is a notice. A second click while the LIST runs
    /// replaces the first.
    pub(crate) fn open_change_diff(
        &mut self,
        deployment: ResourceKey,
        named: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(session), Some(cluster), Some(object)) = (
            self.session().cloned(),
            self.active_cluster(),
            object_ref(&deployment),
        ) else {
            return;
        };
        let Some(live) = self.live_of(&cluster, cx) else {
            return;
        };
        if let AccessState::Known(report) = &live.access
            && !report.is_allowed(AccessCheck::ListReplicaSets)
        {
            notify(
                window,
                cx,
                "Revision diff is unavailable: Not permitted: list replicasets".to_owned(),
            );
            return;
        }
        let selector = object
            .namespace()
            .and_then(|namespace| live.deployment_selector(namespace, object.name()))
            .filter(|selector| !selector.is_empty());
        let Some(selector) = selector else {
            // Without a selector there is nothing to list: the row does what the others do.
            self.reveal(deployment, cx);
            return;
        };
        let connection = live.connection().clone();
        let session_id = session.entity_id();
        let runtime = cx.global::<ClusterRuntime>().clone();
        self.revision_lookup = Some(cx.spawn_in(window, async move |this, cx| {
            let listed = runtime
                .spawn({
                    let connection = connection.clone();
                    async move { connection.deployment_revisions(&object, &selector).await }
                })
                .await;
            let _ = this.update_in(cx, |shell, window, cx| {
                shell.revision_lookup = None;
                // The listing belongs to the cluster it was asked of.
                if shell.session().map(|open| open.entity_id()) != Some(session_id) {
                    return;
                }
                let listed =
                    listed.map_err(|_| "The request stopped before it finished".to_owned());
                shell.finish_change_diff(deployment, named, connection, listed, window, cx);
            });
        }));
    }

    /// The listing came back: opens the dialog, or says why not.
    fn finish_change_diff(
        &mut self,
        deployment: ResourceKey,
        named: Option<String>,
        connection: ClusterConnection,
        listed: Result<Result<Vec<ReplicaSetSummary>, ClusterError>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let replica_sets = match listed {
            Ok(Ok(replica_sets)) => replica_sets,
            Ok(Err(error)) => {
                notify(
                    window,
                    cx,
                    format!("Could not load revisions: {}", error_text(&error)),
                );
                return;
            }
            Err(reason) => {
                notify(window, cx, format!("Could not load revisions: {reason}"));
                return;
            }
        };
        let sides: Vec<RevisionSide> = revision_list(&replica_sets);
        let Some((newer, older)) = change_pair(&sides, named.as_deref()) else {
            let name = match &deployment {
                ResourceKey::Kind { name, .. } => name.as_str(),
                ResourceKey::Pod { name, .. } | ResourceKey::Node { name } => name.as_str(),
            };
            notify(
                window,
                cx,
                format!("No earlier revision kept for deployment/{name}"),
            );
            return;
        };
        let request = diff_request(deployment.clone(), older, newer);
        let title = request.title();
        let shell = cx.weak_entity();
        let view = cx
            .new(|cx| RevisionDiffView::new(request, connection, cx).with_go_to(deployment, shell));
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .title(title.clone())
                .w(px(REVISION_DIFF_WIDTH))
                .child(dialog_body(&view, window))
        });
    }
}
