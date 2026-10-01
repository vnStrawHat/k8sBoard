//! The pods section of a drawer: the pods a workload or a node runs.

use cluster::{NamespaceScope, PodSummary};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::cluster_session::LiveCluster;
use crate::drawer::section_title;
use crate::kind_row::{DAEMON_SET_KIND, PodOwner, STATEFUL_SET_KIND, owns_pod};
use crate::status_tone::{pod_status_label, toned_text};
use crate::table_selection::ResourceKey;
use crate::workload_rows::sort_by_ordinal;

/// Bounds the render cost of a workload with very many pods.
const MAX_RELATED_PODS: usize = 50;

/// The pods of `owner`, read from the live pods list at render time so they stay current.
/// A click opens the pod on the Pods screen. A node lists the pods of the current scope that run on it.
pub(crate) fn pods_section(
    owner: &PodOwner,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> AnyElement {
    let mut pods: Vec<&PodSummary> = live
        .pods
        .items()
        .iter()
        .filter(|pod| owns_pod(owner, pod))
        .collect();
    // StatefulSet pods read best in ordinal order; the others keep the snapshot order.
    if let PodOwner::Controller { kind, name, .. } = owner
        && *kind == STATEFUL_SET_KIND
    {
        sort_by_ordinal(&mut pods, name);
    }
    // A DaemonSet runs one pod per node, so the node is what tells its pods apart.
    let detail = match owner {
        PodOwner::Controller { kind, .. } if *kind == DAEMON_SET_KIND => {
            PodRowDetail::StatusAndNode
        }
        PodOwner::Node { .. } => PodRowDetail::NamespaceAndStatus,
        _ => PodRowDetail::StatusOnly,
    };
    let (title, note) = if live.pods.is_loading() {
        ("Pods".to_owned(), Some("Loading pods…"))
    } else if live.pods.failure().is_some() {
        ("Pods".to_owned(), Some("Pods are unavailable"))
    } else {
        (
            format!("Pods {}", pods.len()),
            pods.is_empty().then_some("No pods"),
        )
    };
    let hidden = pods.len().saturating_sub(MAX_RELATED_PODS);
    let theme = cx.theme();
    v_flex()
        .child(section_title(title, cx))
        .children(scope_note(owner, &live.scope).map(|text| {
            div()
                .pb_1()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
        }))
        .children(note.map(|note| {
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(note)
        }))
        .children(
            pods.iter()
                .take(MAX_RELATED_PODS)
                .enumerate()
                .map(|(index, pod)| related_pod_row(index, pod, detail, cx)),
        )
        .children((hidden > 0).then(|| {
            div()
                .px_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(format!("+{hidden} more"))
        }))
        .into_any_element()
}

/// The pods watch follows the namespace picker, so a node lists only the pods of that namespace.
fn scope_note(owner: &PodOwner, scope: &NamespaceScope) -> Option<String> {
    match (owner, scope) {
        (PodOwner::Node { .. }, NamespaceScope::Named(namespace)) => {
            Some(format!("Only pods in {namespace} are listed"))
        }
        _ => None,
    }
}

/// What a related-pod row shows after the pod name.
#[derive(Clone, Copy)]
enum PodRowDetail {
    StatusOnly,
    StatusAndNode,
    /// A node runs pods of any namespace, so the namespace is shown.
    NamespaceAndStatus,
}

fn related_pod_row(
    index: usize,
    pod: &PodSummary,
    detail: PodRowDetail,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let key = ResourceKey::of_pod(pod);
    let hover_bg = theme.muted;
    h_flex()
        .id(("related-pod", index))
        .gap_2()
        .items_center()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(key.clone(), cx)))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .overflow_hidden()
                .font_family(theme.mono_font_family.clone())
                .when(matches!(detail, PodRowDetail::NamespaceAndStatus), |this| {
                    this.child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(160.))
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(format!("{}/", pod.namespace)),
                    )
                })
                .child(div().min_w_0().truncate().child(pod.name.clone())),
        )
        .child(toned_text(pod_status_label(pod), cx))
        .children(matches!(detail, PodRowDetail::StatusAndNode).then(|| {
            div()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(pod.node_name.clone().unwrap_or_default())
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> PodOwner {
        PodOwner::Node { name: "n1".into() }
    }

    #[test]
    fn scope_note_only_for_a_node_in_a_named_namespace() {
        let named = NamespaceScope::Named("ns".into());
        assert_eq!(
            scope_note(&node(), &named).as_deref(),
            Some("Only pods in ns are listed")
        );
        assert_eq!(scope_note(&node(), &NamespaceScope::All), None);
        let controller = PodOwner::Controller {
            kind: DAEMON_SET_KIND,
            name: "d".into(),
            namespace: "ns".into(),
        };
        assert_eq!(scope_note(&controller, &named), None);
    }
}
