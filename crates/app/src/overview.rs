//! The Overview screen (W3): what is broken, how much room is left, what just changed. This module
//! holds the pure header text and counts, and the panels' view code over the live snapshots.

use std::collections::BTreeSet;

use cluster::{
    NamespaceScope, NamespaceSummary, NodeReadiness, NodeSummary, PodStatus, PodSummary,
    ServerVersion, StatusReason,
};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::cluster_capacity::{
    CapacityInputs, CapacityRow, FromPods, VolumeFeed, cluster_capacity, volume_totals,
};
use crate::cluster_metrics::FeedStatus;
use crate::cluster_session::{LiveCluster, LiveList};
use crate::issue_feeds::{FeedState, volume_usage_state};
use crate::node_heatmap::{heat_cells, node_heatmap};
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusTone, tone_color};
use crate::usage_bar::{CapacityBar, capacity_bar};
use crate::usage_format::group_digits;

const REGION_LABEL: &str = "topology.kubernetes.io/region=";
/// Flex basis of the wide and the narrow panel of a row; below their sum the row wraps.
const WIDE_PANEL: f32 = 560.;
const NARROW_PANEL: f32 = 360.;
const LEGEND_SWATCH: f32 = 9.;

/// `{context} · Kubernetes {git_version}`, plus ` · {region}` when known.
pub(crate) fn headline_text(
    context: &str,
    version: &ServerVersion,
    nodes: Option<&[NodeSummary]>,
) -> String {
    let mut text = format!("{context} · Kubernetes {}", version.git_version);
    if let Some(region) = nodes.and_then(cluster_region) {
        text.push_str(" · ");
        text.push_str(&region);
    }
    text
}

/// The `topology.kubernetes.io/region` of the labelled nodes: one value is shown as it is, several
/// as `{n} regions`, none as `None`.
fn cluster_region(nodes: &[NodeSummary]) -> Option<String> {
    let regions: BTreeSet<&str> = nodes
        .iter()
        .flat_map(|node| &node.labels)
        .filter_map(|label| label.strip_prefix(REGION_LABEL))
        .collect();
    match regions.len() {
        0 => None,
        1 => regions.first().map(|region| (*region).to_owned()),
        count => Some(format!("{count} regions")),
    }
}

/// `ready` of `total`, for nodes that are Ready and pods that run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Counted {
    ready: usize,
    total: usize,
}

/// The numbers of the stats line; a part is `None` while its list is not Ready.
#[derive(Debug, PartialEq, Eq)]
struct ClusterStats {
    nodes: Option<Counted>,
    pods: Option<Counted>,
    namespaces: Option<usize>,
}

/// A running pod is in phase Running: ready or not.
fn is_running(pod: &PodSummary) -> bool {
    matches!(
        pod.status,
        PodStatus::Reason(StatusReason::Running) | PodStatus::NotReady
    )
}

fn cluster_stats(
    nodes: Option<&[NodeSummary]>,
    pods: Option<&[PodSummary]>,
    namespaces: Option<&[NamespaceSummary]>,
) -> ClusterStats {
    ClusterStats {
        nodes: nodes.map(|nodes| Counted {
            ready: nodes
                .iter()
                .filter(|node| node.status.readiness == NodeReadiness::Ready)
                .count(),
            total: nodes.len(),
        }),
        pods: pods.map(|pods| Counted {
            ready: pods.iter().filter(|pod| is_running(pod)).count(),
            total: pods.len(),
        }),
        namespaces: namespaces.map(<[_]>::len),
    }
}

/// The muted line under the header (user-requested, not in W3): `41 / 42 nodes ready · 1,284 /
/// 1,310 pods running · 37 namespaces`. A node count below the total is toned Bad.
pub(crate) fn stats_line(live: &LiveCluster, cx: &App) -> impl IntoElement {
    let stats = cluster_stats(
        live.nodes.ready_items(),
        live.pods.ready_items(),
        live.namespaces.ready_items(),
    );
    let figure = |counted: Option<Counted>| {
        counted.map_or_else(
            || "—".to_owned(),
            |counted| {
                format!(
                    "{} / {}",
                    group_digits(counted.ready),
                    group_digits(counted.total)
                )
            },
        )
    };
    let nodes_tone = stats
        .nodes
        .filter(|counted| counted.ready < counted.total)
        .map(|_| tone_color(StatusTone::Bad, cx));
    let pods_scope = match &live.scope {
        NamespaceScope::All => String::new(),
        _ => format!(" in {}", live.scope_label()),
    };
    h_flex()
        .flex_shrink_0()
        .flex_wrap()
        .gap_x_1()
        .px_4()
        .py_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(
            div()
                .when_some(nodes_tone, |this, color| this.text_color(color))
                .child(format!("{} nodes ready", figure(stats.nodes))),
        )
        .child("·")
        .child(format!("{} pods running{pods_scope}", figure(stats.pods)))
        .child("·")
        .child(format!(
            "{} namespaces",
            stats
                .namespaces
                .map_or_else(|| "—".to_owned(), group_digits)
        ))
}

/// The scrolling body: Capacity in row 1 and Nodes in row 2 (the other panels follow in later
/// steps).
pub(crate) fn overview_body(live: &LiveCluster, cx: &Context<AppShell>) -> AnyElement {
    v_flex()
        .id("overview")
        .size_full()
        .overflow_y_scroll()
        .p_4()
        .gap_3()
        .child(panel_row([capacity_panel(live, cx)
            .flex_basis(px(NARROW_PANEL))
            .into_any_element()]))
        .child(panel_row([nodes_panel(live, cx)
            .flex_basis(px(WIDE_PANEL))
            .into_any_element()]))
        .into_any_element()
}

/// A wrapping row of panels; the panels keep their own height.
fn panel_row(panels: impl IntoIterator<Item = AnyElement>) -> Div {
    h_flex().flex_wrap().items_start().gap_3().children(panels)
}

/// A bordered panel with a header: the title, optional extras after it, and a right group.
fn panel(
    id: &'static str,
    title: &str,
    after_title: Option<AnyElement>,
    right: Option<AnyElement>,
    body: impl IntoElement,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let header = h_flex()
        .px_3()
        .py_2()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .child(div().text_sm().font_semibold().child(title.to_owned()))
        .children(after_title)
        .children(right.map(|right| div().ml_auto().child(right)));
    v_flex()
        .id(id)
        .flex_grow(1.)
        .min_w_0()
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .bg(theme.background)
        .child(header)
        .child(body)
}

fn muted_text(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// The body of a panel whose data has not arrived: a spinner while loading, or the failure.
fn pending_body(list: &LiveList<NodeSummary>, cx: &App) -> Option<AnyElement> {
    match list {
        LiveList::Ready { .. } => None,
        LiveList::Loading => Some(
            h_flex()
                .p_3()
                .gap_2()
                .child(Spinner::new())
                .child(muted_text("Loading nodes…", cx))
                .into_any_element(),
        ),
        LiveList::Failed { message } => Some(
            div()
                .p_3()
                .child(muted_text(format!("Nodes unavailable · {message}"), cx))
                .into_any_element(),
        ),
    }
}

fn is_polling(status: &FeedStatus) -> bool {
    matches!(status, FeedStatus::Live | FeedStatus::Interrupted(_))
}

// ---- Capacity ----

fn capacity_panel(live: &LiveCluster, cx: &App) -> Stateful<Div> {
    if let Some(pending) = pending_body(&live.nodes, cx) {
        let legend = capacity_legend(false, cx);
        return panel("capacity", "Capacity", None, Some(legend), pending, cx);
    }
    let rows = capacity_model(live);
    let legend = capacity_legend(rows.iter().any(CapacityRow::has_requested), cx);
    let body = capacity_rows(live, &rows, cx);
    panel("capacity", "Capacity", None, Some(legend), body, cx)
}

fn capacity_legend(has_requested: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let entry = |label: &'static str, color: Hsla| {
        h_flex()
            .gap_1()
            .items_center()
            .child(
                div()
                    .size(px(LEGEND_SWATCH))
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .bg(color),
            )
            .child(label)
    };
    h_flex()
        .gap_2()
        .items_center()
        .text_xs()
        .font_family(theme.mono_font_family.clone())
        .text_color(theme.muted_foreground)
        .child(entry("used", theme.foreground))
        .children(has_requested.then(|| entry("requested", theme.foreground.opacity(0.28))))
        .child(entry("allocatable", theme.muted))
        .into_any_element()
}

/// The rows from the live snapshots. Requests and pod counts need the pods list of every
/// namespace, so a narrower scope drops them, and a list that has not loaded shows `—`.
fn capacity_model(live: &LiveCluster) -> Vec<CapacityRow> {
    let nodes = live.nodes.items();
    let node_feed = &live.metrics.nodes;
    let kubelet = &live.metrics.kubelet;
    let pods = match (&live.scope, live.pods.ready_items()) {
        (NamespaceScope::All, Some(pods)) => FromPods::Known(pods),
        (NamespaceScope::All, None) => FromPods::Pending,
        _ => FromPods::NeedsAllNamespaces,
    };
    let volumes = if is_polling(&kubelet.status) {
        // The claim list tells a claim's own volume from a node disk the kubelet reports for it.
        let claims = live
            .issue_feeds
            .conditions
            .iter()
            .find(|feed| feed.kind == ResourceKind::PersistentVolumeClaims)
            .filter(|feed| feed.state() == FeedState::Live)
            .and_then(|feed| feed.list.ready_items());
        let mut totals = volume_totals(kubelet.history.pvc_usages(), claims);
        let ready_nodes = nodes
            .iter()
            .filter(|node| node.status.readiness == NodeReadiness::Ready)
            .count();
        let polled = kubelet.targets().summary_nodes.len();
        if let FeedState::Limited(text) = volume_usage_state(&kubelet.status, polled, ready_nodes) {
            totals.limited = Some(text);
        }
        VolumeFeed::Live(totals)
    } else {
        VolumeFeed::Unavailable {
            reason: kubelet.status.reason().map(str::to_owned),
        }
    };
    cluster_capacity(&CapacityInputs {
        nodes,
        pods,
        node_usage: is_polling(&node_feed.status).then_some(&node_feed.history),
        volumes,
    })
}

fn capacity_rows(live: &LiveCluster, rows: &[CapacityRow], cx: &App) -> AnyElement {
    let scope_suffix = if live.scope == NamespaceScope::All {
        String::new()
    } else {
        format!(" in {}", live.scope_label())
    };
    let metrics_reason = live.metrics.nodes.status.reason().map(str::to_owned);
    v_flex()
        .p_3()
        .gap_3()
        .children(rows.iter().enumerate().map(|(index, row)| {
            capacity_row(index, row, &scope_suffix, metrics_reason.as_deref(), cx)
        }))
        .into_any_element()
}

fn capacity_row(
    index: usize,
    row: &CapacityRow,
    scope_suffix: &str,
    metrics_reason: Option<&str>,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let mut label: Vec<AnyElement> = row
        .label_parts()
        .into_iter()
        .map(|part| {
            let color = part.tone.map(|tone| tone_color(tone, cx));
            div()
                .when_some(color, |this, color| this.text_color(color))
                .child(part.text)
                .into_any_element()
        })
        .collect();
    if matches!(row, CapacityRow::Volumes(_)) && !scope_suffix.is_empty() {
        label.push(div().child(scope_suffix.to_owned()).into_any_element());
    }
    let is_compute = matches!(row, CapacityRow::Cpu(_) | CapacityRow::Memory(_));
    let tooltip = [
        metrics_reason
            .filter(|_| is_compute)
            .map(|reason| format!("Node metrics unavailable: {reason}.")),
        row.unavailable_reason()
            .map(|reason| format!("Volume usage unavailable: {reason}.")),
        row.ceiling().map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let element = v_flex().id(("capacity-row", index)).gap_1().child(
        h_flex()
            .gap_2()
            .items_baseline()
            .child(div().text_sm().font_semibold().child(row.name()))
            .child(
                h_flex()
                    .ml_auto()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .children(label),
            ),
    );
    let element = element
        .child(capacity_bar(CapacityBar::of_row(row), cx))
        .children(row.note().map(|note| muted_text(note, cx)));
    if tooltip.is_empty() {
        return element;
    }
    element.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
}

// ---- Nodes ----

fn nodes_panel(live: &LiveCluster, cx: &Context<AppShell>) -> Stateful<Div> {
    let node_feed = &live.metrics.nodes;
    let Some(nodes) = live.nodes.ready_items() else {
        let pending = pending_body(&live.nodes, cx).unwrap_or_else(|| div().into_any_element());
        return panel("nodes", "Nodes", None, None, pending, cx);
    };
    let is_live = is_polling(&node_feed.status);
    let cells = heat_cells(nodes, is_live.then_some(&node_feed.history));
    let mut subtitle = format!("{} · colored by CPU", group_digits(cells.len()));
    if !is_live {
        subtitle.push_str(" · metrics unavailable");
    }
    let not_ready = cells.iter().filter(|cell| cell.is_not_ready()).count();
    let warning = (not_ready > 0).then(|| {
        div()
            .text_xs()
            .text_color(tone_color(StatusTone::Bad, cx))
            .child(format!("{not_ready} NotReady"))
            .into_any_element()
    });
    panel(
        "nodes",
        "Nodes",
        Some(muted_text(subtitle, cx).into_any_element()),
        warning,
        node_heatmap(&cells, cx),
        cx,
    )
}

#[cfg(test)]
mod tests {
    use cluster::{
        NodeResource, NodeScheduling, NodeStatus, NodeSystemInfo, PodStatus, ReadyCount,
    };

    use super::*;

    fn node(readiness: NodeReadiness, labels: &[&str]) -> NodeSummary {
        NodeSummary {
            name: "n".to_owned(),
            status: NodeStatus {
                readiness,
                scheduling: NodeScheduling::Enabled,
            },
            roles: Vec::new(),
            taints: Vec::new(),
            kubelet_version: "v1.29.5".to_owned(),
            internal_ip: None,
            created_at: None,
            conditions: Vec::new(),
            addresses: Vec::new(),
            system: NodeSystemInfo::default(),
            resources: Vec::<NodeResource>::new(),
            labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        }
    }

    fn pod(status: PodStatus) -> PodSummary {
        PodSummary {
            namespace: "ns".to_owned(),
            name: "p".to_owned(),
            status,
            ready: ReadyCount { ready: 0, total: 0 },
            restarts: 0,
            node_name: None,
            created_at: None,
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: None,
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            containers: Vec::new(),
        }
    }

    fn version() -> ServerVersion {
        ServerVersion {
            git_version: "v1.29.5".to_owned(),
            platform: "linux/amd64".to_owned(),
        }
    }

    fn region(value: &str) -> String {
        format!("{REGION_LABEL}{value}")
    }

    #[test]
    fn headline_has_context_version_region() {
        let nodes = [node(NodeReadiness::Ready, &[&region("ap-southeast-1")])];
        assert_eq!(
            headline_text("readonly@Monitor", &version(), Some(&nodes)),
            "readonly@Monitor · Kubernetes v1.29.5 · ap-southeast-1"
        );
        assert_eq!(
            headline_text("readonly@Monitor", &version(), None),
            "readonly@Monitor · Kubernetes v1.29.5"
        );
    }

    #[test]
    fn region_single_value() {
        let label = region("eu-west-1");
        let nodes = [
            node(NodeReadiness::Ready, &[&label]),
            node(NodeReadiness::Ready, &[&label]),
        ];
        assert_eq!(cluster_region(&nodes).as_deref(), Some("eu-west-1"));
    }

    #[test]
    fn region_counts_several() {
        let nodes = [
            node(NodeReadiness::Ready, &[&region("eu-west-1")]),
            node(NodeReadiness::Ready, &[&region("us-east-1")]),
        ];
        assert_eq!(cluster_region(&nodes).as_deref(), Some("2 regions"));
    }

    #[test]
    fn region_absent_without_labels() {
        let nodes = [node(NodeReadiness::Ready, &["zone=a"])];
        assert_eq!(cluster_region(&nodes), None);
    }

    #[test]
    fn stats_count_ready_nodes() {
        let nodes = [
            node(NodeReadiness::Ready, &[]),
            node(NodeReadiness::NotReady, &[]),
            node(NodeReadiness::Unknown, &[]),
        ];
        let stats = cluster_stats(Some(&nodes), None, None);
        assert_eq!(stats.nodes, Some(Counted { ready: 1, total: 3 }));
    }

    #[test]
    fn stats_count_running_and_not_ready_pods() {
        let pods = [
            pod(PodStatus::Reason(StatusReason::Running)),
            pod(PodStatus::NotReady),
            pod(PodStatus::Reason(StatusReason::Pending)),
            pod(PodStatus::Reason(StatusReason::Succeeded)),
        ];
        let stats = cluster_stats(None, Some(&pods), None);
        assert_eq!(stats.pods, Some(Counted { ready: 2, total: 4 }));
    }

    #[test]
    fn stats_parts_are_none_while_loading() {
        let stats = cluster_stats(None, None, None);
        assert_eq!(
            stats,
            ClusterStats {
                nodes: None,
                pods: None,
                namespaces: None
            }
        );
    }
}
