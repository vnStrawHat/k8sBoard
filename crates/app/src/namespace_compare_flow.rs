//! Opens the namespace comparison dialog (spec 0057) from the Namespaces row, the key, and the
//! palette command. The dialog reads through the connection of the open cluster and never writes.

use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, Context, ParentElement as _, Window, px};

use super::AppShell;
use crate::cluster_session::LiveList;
use crate::namespace_compare_view::{NamespaceCompareView, dialog_body};

/// The room two namespaces of field changes and diff rows need.
const COMPARE_WIDTH: f32 = 960.;

impl AppShell {
    /// Compare with… on the namespace `left`: the dialog opens on the choice of the other side.
    /// Nothing opens while the session is not live.
    pub(crate) fn open_namespace_compare(
        &mut self,
        left: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live(cx) else {
            return;
        };
        let connection = live.connection().clone();
        let candidates = match &live.namespaces {
            LiveList::Ready { items, .. } => items.iter().map(|item| item.name.clone()).collect(),
            LiveList::Loading | LiveList::Failed { .. } => Vec::new(),
        };
        let view = cx.new(|cx| NamespaceCompareView::new(left, candidates, connection, window, cx));
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .title("Compare namespaces")
                .w(px(COMPARE_WIDTH))
                .child(dialog_body(&view, window))
        });
    }

    /// The palette command and its chord: the left side is the scope's only namespace, else the
    /// namespace of the kubeconfig context.
    pub(crate) fn open_namespace_compare_from_scope(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live(cx) else {
            return;
        };
        let left = match live.scope.namespaces() {
            [only] => only.clone(),
            _ => live.default_namespace().to_owned(),
        };
        self.open_namespace_compare(left, window, cx);
    }
}
