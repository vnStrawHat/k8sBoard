//! The Overview screen (W3): what is broken, how much room is left, what just changed. This module
//! holds the pure header text and counts, and the panels' view code over the live snapshots.

use std::collections::{BTreeSet, HashSet};

use jiff::tz::TimeZone;

use cluster::{
    EventSummary, NamespaceScope, NamespaceSummary, NodeReadiness, NodeSummary, PodStatus,
    PodSummary, ServerVersion, StatusReason,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_capacity::{
    CapacityInputs, CapacityRow, FromPods, VolumeFeed, cluster_capacity, volume_totals,
};
use crate::cluster_metrics::FeedStatus;
use crate::cluster_session::{ChangeEvents, LiveCluster, LiveList};
use crate::dock::{Dock, LogOrigin};
use crate::event_rows::message_line;
use crate::file_export::ExportState;
use crate::issue::{Issue, IssueAction};
use crate::issue_board::IssueBoard;
use crate::issue_feeds::{FeedState, volume_usage_state};
use crate::issue_table::{coverage_status, logs_pod, short_kind};
use crate::kind_row::KindObject;
use crate::node_heatmap::{heat_cells, node_heatmap};
use crate::recent_changes::{
    CHANGE_ROWS, ChangeEntry, ChangeInputs, ChangeKind, ChangeWindow, recent_changes,
};
use crate::resource_actions::logs_launch;
use crate::resource_kind::ResourceKind;
use crate::row_context::RowContext;
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ResourceKey;
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

/// The three parts of the stats line, as text: `41 / 42 nodes ready`, `1,284 / 1,310 pods
/// running`, and `37 namespaces`. A part whose list is not Ready shows `—`.
struct StatsText {
    nodes: String,
    pods: String,
    namespaces: String,
    /// Some node is not Ready, so the nodes part is toned.
    has_unready_nodes: bool,
}

fn stats_parts(live: &LiveCluster) -> StatsText {
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
    let pods_scope = match &live.scope {
        NamespaceScope::All => String::new(),
        _ => format!(" in {}", live.scope_label()),
    };
    StatsText {
        nodes: format!("{} nodes ready", figure(stats.nodes)),
        pods: format!("{} pods running{pods_scope}", figure(stats.pods)),
        namespaces: format!(
            "{} namespaces",
            stats
                .namespaces
                .map_or_else(|| "—".to_owned(), group_digits)
        ),
        has_unready_nodes: stats
            .nodes
            .is_some_and(|counted| counted.ready < counted.total),
    }
}

/// The stats line as one string, for the report.
pub(crate) fn stats_text(live: &LiveCluster) -> String {
    let parts = stats_parts(live);
    format!("{} · {} · {}", parts.nodes, parts.pods, parts.namespaces)
}

/// The muted line under the header (user-requested, not in W3). A node count below the total is
/// toned Bad.
pub(crate) fn stats_line(live: &LiveCluster, cx: &App) -> impl IntoElement {
    let parts = stats_parts(live);
    let nodes_tone = parts
        .has_unready_nodes
        .then(|| tone_color(StatusTone::Bad, cx));
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
                .child(parts.nodes),
        )
        .child("·")
        .child(parts.pods)
        .child("·")
        .child(parts.namespaces)
}

/// What the panels read besides the clock.
pub(crate) struct OverviewData<'a> {
    pub(crate) live: &'a LiveCluster,
    pub(crate) board: &'a IssueBoard,
    pub(crate) window: ChangeWindow,
    pub(crate) dock: &'a WeakEntity<Dock>,
    /// The cluster Overview draws: the primary one while several are viewed.
    pub(crate) row: &'a RowContext,
}

/// The scrolling body: Needs attention and Capacity in row 1; Nodes and Recent changes in row 2.
pub(crate) fn overview_body(data: &OverviewData, cx: &Context<AppShell>) -> AnyElement {
    let (live, window) = (data.live, data.window);
    v_flex()
        .id("overview")
        .size_full()
        .overflow_y_scroll()
        .p_4()
        .gap_3()
        .child(panel_row([
            needs_attention_panel(data, cx)
                .flex_basis(px(WIDE_PANEL))
                .into_any_element(),
            capacity_panel(live, cx)
                .flex_basis(px(NARROW_PANEL))
                .into_any_element(),
        ]))
        .child(panel_row([
            nodes_panel(live, cx)
                .flex_basis(px(WIDE_PANEL))
                .into_any_element(),
            recent_changes_panel(live, window, cx)
                .flex_basis(px(NARROW_PANEL))
                .into_any_element(),
        ]))
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

pub(crate) fn is_polling(status: &FeedStatus) -> bool {
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
pub(crate) fn capacity_model(live: &LiveCluster) -> Vec<CapacityRow> {
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

// ---- Recent changes ----

/// What the Overview screen keeps between renders.
#[derive(Default)]
pub(crate) struct OverviewState {
    pub(crate) window: ChangeWindow,
    pub(crate) export: ExportState,
    /// The running export; dropping it abandons the export.
    pub(crate) _export: Option<Task<()>>,
}

/// `Last 15 min ▾`: the range of Recent changes only; capacity and issues are "now".
pub(crate) fn change_window_button(window: ChangeWindow, cx: &Context<AppShell>) -> AnyElement {
    let shell = cx.weak_entity();
    Button::new("change-window")
        .ghost()
        .small()
        .label(window.label())
        .dropdown_caret(true)
        .tooltip("Time window of Recent changes")
        .dropdown_menu(move |menu, _, _| {
            ChangeWindow::ALL.into_iter().fold(menu, |menu, option| {
                let shell = shell.clone();
                menu.item(
                    PopupMenuItem::new(option.label())
                        .checked(option == window)
                        .on_click(move |_, _, cx| {
                            let _ =
                                shell.update(cx, |shell, cx| shell.set_change_window(option, cx));
                        }),
                )
            })
        })
        .into_any_element()
}

fn recent_changes_panel(
    live: &LiveCluster,
    window: ChangeWindow,
    cx: &Context<AppShell>,
) -> Stateful<Div> {
    let view_all = div()
        .id("changes-view-all")
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .cursor_pointer()
        .child("View all →")
        .on_click(cx.listener(|shell, _, _, cx| shell.view_all_events(cx)))
        .into_any_element();
    let body = changes_body(live, window, cx);
    panel("changes", "Recent changes", None, Some(view_all), body, cx)
}

/// What the change feeds have to show; the panel and the report read it alike.
pub(crate) enum ChangeFeed<'a> {
    Ready {
        rollouts: &'a [EventSummary],
        rescales: &'a [EventSummary],
    },
    /// A feed has not delivered its first snapshot, or has not started yet.
    Loading,
    /// A feed failed or is denied: the text that stands in for the changes.
    Unavailable(String),
}

pub(crate) fn change_feed(live: &LiveCluster) -> ChangeFeed<'_> {
    let (rollouts, rescales) = match &live.change_events {
        Some(ChangeEvents::Live {
            rollouts, rescales, ..
        }) => (rollouts, rescales),
        Some(ChangeEvents::Denied) => {
            return ChangeFeed::Unavailable("Not permitted: list events".to_owned());
        }
        // The feeds start the moment Overview is shown.
        None => return ChangeFeed::Loading,
    };
    if let Some(message) = rollouts.failure().or_else(|| rescales.failure()) {
        return ChangeFeed::Unavailable(format!("Changes unavailable · {message}"));
    }
    match (rollouts.ready_items(), rescales.ready_items()) {
        (Some(rollouts), Some(rescales)) => ChangeFeed::Ready { rollouts, rescales },
        _ => ChangeFeed::Loading,
    }
}

fn changes_body(live: &LiveCluster, window: ChangeWindow, cx: &Context<AppShell>) -> AnyElement {
    let (rollouts, rescales) = match change_feed(live) {
        ChangeFeed::Ready { rollouts, rescales } => (rollouts, rescales),
        ChangeFeed::Loading => return loading_text("Loading changes…", cx),
        ChangeFeed::Unavailable(text) => return state_text(text, cx),
    };
    let entries = recent_changes(&ChangeInputs {
        rollouts: Some(rollouts),
        rescales: Some(rescales),
        nodes: live.nodes.ready_items(),
        namespaces: live.namespaces.ready_items(),
        deployments: live.issue_feeds.deployments(),
        window,
        now: jiff::Timestamp::now(),
    });
    let theme = cx.theme();
    let zone = TimeZone::system();
    let shown = entries.len().min(CHANGE_ROWS);
    let diffable = diffable_deployments(live);
    let rows = entries
        .iter()
        .take(CHANGE_ROWS)
        .enumerate()
        .map(|(index, entry)| {
            let opens_diff = opens_diff(entry, &diffable);
            change_row(index, entry, index + 1 == shown, opens_diff, &zone, cx)
        });
    let empty = entries.is_empty().then(|| {
        let span = window.label().trim_start_matches("Last ");
        state_text(format!("No tracked changes seen in the last {span}."), cx)
    });
    v_flex()
        .children(rows)
        .children(empty)
        .child(
            div()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(CHANGES_FOOTNOTE),
        )
        .into_any_element()
}

pub(crate) const CHANGES_FOOTNOTE: &str =
    "Deployment rollouts, HPA rescales, nodes, namespaces · events kept ~1 h by the API server";

fn change_row(
    index: usize,
    entry: &ChangeEntry,
    is_last: bool,
    opens_diff: bool,
    zone: &TimeZone,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let mono = theme.mono_font_family.clone();
    let time = entry
        .at
        .to_zoned(zone.clone())
        .strftime("%H:%M")
        .to_string();
    let count = (entry.count > 1).then(|| format!(" ×{}", entry.count));
    let tooltip = SharedString::from(entry.tooltip());
    let row = h_flex()
        .id(SharedString::from(format!("change-{index}")))
        .gap_2()
        .px_3()
        .py(px(6.))
        .text_xs()
        .when(!is_last, |this| {
            this.border_b_1().border_color(theme.border)
        })
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(
            div()
                .w(px(44.))
                .flex_none()
                .font_family(mono.clone())
                .text_color(theme.muted_foreground)
                .child(time),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(entry.kind.label())
                .child(div().font_family(mono.clone()).child(entry.object.clone()))
                .child(format!("{}{}", entry.text, count.unwrap_or_default())),
        )
        .children(entry.actor.clone().map(|actor| {
            div()
                .flex_none()
                .font_family(mono)
                .text_color(theme.muted_foreground)
                .child(actor)
        }));
    match entry.target.clone() {
        Some(key) if opens_diff => {
            let named = entry.replica_set.clone();
            row.cursor_pointer()
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.open_change_diff(key.clone(), named.clone(), window, cx)
                }))
                .into_any_element()
        }
        Some(key) => row
            .cursor_pointer()
            .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(key.clone(), cx)))
            .into_any_element(),
        None => row.into_any_element(),
    }
}

/// The Deployments of the feed that have a selector, by (namespace, name), built once per render.
/// A click on their rows opens the revision diff; without a selector there is nothing to list.
fn diffable_deployments(live: &LiveCluster) -> HashSet<(&str, &str)> {
    live.issue_feeds
        .deployments()
        .into_iter()
        .flatten()
        .filter_map(|object| match object {
            KindObject::Deployment(deployment) if !deployment.selector.is_empty() => {
                Some((deployment.namespace.as_str(), deployment.name.as_str()))
            }
            _ => None,
        })
        .collect()
}

/// Whether a click on the row opens the revision diff: a Deployment row of `diffable`. Every other
/// row reveals its object.
fn opens_diff(entry: &ChangeEntry, diffable: &HashSet<(&str, &str)>) -> bool {
    let Some(ResourceKey::Kind {
        namespace: Some(namespace),
        name,
        ..
    }) = &entry.target
    else {
        return false;
    };
    entry.kind == ChangeKind::Deployment && diffable.contains(&(namespace.as_str(), name.as_str()))
}

fn state_text(text: String, cx: &App) -> AnyElement {
    div().p_3().child(muted_text(text, cx)).into_any_element()
}

fn loading_text(text: &'static str, cx: &App) -> AnyElement {
    h_flex()
        .p_3()
        .gap_2()
        .child(Spinner::new())
        .child(muted_text(text, cx))
        .into_any_element()
}

// ---- Needs attention ----

/// The issues the panel lists; the Issues screen has the rest.
const ATTENTION_ROWS: usize = 6;
/// The issue pill column, as in W3.
const PILL_WIDTH: f32 = 118.;
/// The least width of the object and cause column of an attention row.
const WHAT_MIN_WIDTH: f32 = 200.;

/// `payments / api-7d9f8c-x2k4q · container api`, plus ` · {count} pods` for a group; a cluster
/// object has no `ns / ` part.
pub(crate) fn object_line(issue: &Issue) -> String {
    let mut line = match &issue.shown.namespace {
        Some(namespace) => format!("{namespace} / {}", issue.shown.name),
        None => issue.shown.name.clone(),
    };
    if let Some(container) = &issue.container {
        line.push_str(&format!(" · container {container}"));
    }
    if issue.count > 1 {
        line.push_str(&format!(" · {} pods", issue.count));
    }
    line
}

/// The first issues of the board, in board order.
fn attention_issues(issues: &[Issue]) -> &[Issue] {
    &issues[..issues.len().min(ATTENTION_ROWS)]
}

/// What a row's button does.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AttentionAction {
    ViewLogs,
    Open { label: String },
}

/// View logs for a logs issue; `See why` for a pod (its drawer shows WHY); `Open {Kind}` for any
/// other object that has a screen, with the long policy kinds shortened as the Issues table does
/// (`Open HPA`); nothing without a target. All are read-only.
fn attention_action(issue: &Issue) -> Option<AttentionAction> {
    if matches!(issue.action, IssueAction::ViewLogs { .. }) {
        return Some(AttentionAction::ViewLogs);
    }
    let label = match issue.target.as_ref()? {
        ResourceKey::Pod { .. } => "See why".to_owned(),
        ResourceKey::Node { .. } | ResourceKey::Kind { .. } => {
            format!("Open {}", short_kind(&issue.shown.kind))
        }
    };
    Some(AttentionAction::Open { label })
}

fn needs_attention_panel(data: &OverviewData, cx: &Context<AppShell>) -> Stateful<Div> {
    let board = data.board;
    let Some(summary) = board.summary() else {
        return panel(
            "attention",
            "Needs attention",
            None,
            None,
            loading_text("Checking the cluster…", cx),
            cx,
        );
    };
    let count = div()
        .text_xs()
        .px_1()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(tone_color(summary.worst().tone(), cx))
        .text_color(tone_color(summary.worst().tone(), cx))
        .child(group_digits(summary.total))
        .into_any_element();
    let status = div()
        .text_xs()
        .child(coverage_status(board.coverage(), cx))
        .into_any_element();
    let body = if summary.total == 0 {
        let text = if summary.is_partial {
            "No issues found in what k8sBoard watches."
        } else {
            "No issues found."
        };
        v_flex()
            .p_3()
            .gap_1()
            .child(muted_text(text, cx))
            .children(board.coverage().note().map(|note| muted_text(note, cx)))
            .into_any_element()
    } else {
        attention_rows(data, summary.total, cx)
    };
    panel(
        "attention",
        "Needs attention",
        Some(count),
        Some(status),
        body,
        cx,
    )
}

fn attention_rows(data: &OverviewData, total: usize, cx: &Context<AppShell>) -> AnyElement {
    let issues = attention_issues(data.board.issues());
    let rows = issues
        .iter()
        .enumerate()
        .map(|(index, issue)| attention_row(data, index, issue, index + 1 == issues.len(), cx));
    let view_all = (total > ATTENTION_ROWS).then(|| {
        div()
            .id("attention-view-all")
            .px_3()
            .py_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .cursor_pointer()
            .child(format!("View all {} issues →", group_digits(total)))
            .on_click(cx.listener(|shell, _, _, cx| shell.show_screen(Screen::Issues, cx)))
    });
    v_flex()
        .children(rows)
        .children(view_all)
        .into_any_element()
}

fn attention_row(
    data: &OverviewData,
    index: usize,
    issue: &Issue,
    is_last: bool,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let tone = tone_color(issue.severity.tone(), cx);
    // The pill is as wide as its label, at least the W3 column, so a long reason is never cut.
    let pill = div().min_w(px(PILL_WIDTH)).flex_none().child(
        div()
            .text_xs()
            .px_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(tone)
            .text_color(tone)
            .whitespace_nowrap()
            .child(issue.reason.clone()),
    );
    let cause = SharedString::from(message_line(&issue.cause));
    let tooltip_cause = cause.clone();
    // The object and the cause keep a minimum width, so a long button label never squeezes them.
    let what = v_flex()
        .flex_1()
        .min_w(px(WHAT_MIN_WIDTH))
        .child(
            div()
                .text_xs()
                .font_family(theme.mono_font_family.clone())
                .truncate()
                .child(object_line(issue)),
        )
        .child(
            div()
                .id(SharedString::from(format!("issue-cause-{index}")))
                .text_xs()
                .text_color(theme.muted_foreground)
                .line_clamp(2)
                .child(cause)
                .tooltip(move |window, cx| Tooltip::new(tooltip_cause.clone()).build(window, cx)),
        );
    let target_text = match &issue.target {
        Some(_) => format!(
            "{} {}",
            issue.shown.kind,
            issue.shown.namespace.as_ref().map_or_else(
                || issue.shown.name.clone(),
                |ns| format!("{ns}/{}", issue.shown.name)
            )
        ),
        None => format!(
            "{} {} · No screen for {}",
            issue.shown.kind, issue.shown.name, issue.shown.kind
        ),
    };
    let row = h_flex()
        .id(SharedString::from(format!("issue-{index}")))
        .gap_2()
        .px_3()
        .py_2()
        .when(!is_last, |this| {
            this.border_b_1().border_color(theme.border)
        })
        .tooltip(move |window, cx| Tooltip::new(target_text.clone()).build(window, cx))
        .child(pill)
        .child(what)
        .children(attention_button(data, index, issue, cx));
    match issue.target.clone() {
        Some(key) => row
            .cursor_pointer()
            .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(key.clone(), cx)))
            .into_any_element(),
        None => row.into_any_element(),
    }
}

/// The row's one read-only action. A click stops propagation so the row's reveal does not also fire.
fn attention_button(
    data: &OverviewData,
    index: usize,
    issue: &Issue,
    cx: &Context<AppShell>,
) -> Option<AnyElement> {
    let action = attention_action(issue)?;
    let id = SharedString::from(format!("issue-action-{index}"));
    let button = Button::new(id).ghost().small().flex_none();
    let element = match action {
        AttentionAction::Open { label } => {
            let key = issue.target.clone()?;
            button
                .label(label)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    cx.stop_propagation();
                    shell.reveal(key.clone(), cx);
                }))
        }
        AttentionAction::ViewLogs => {
            let launch = match logs_pod(issue, data.live.pods.items()) {
                Some(pod) => logs_launch(pod, issue.container.as_deref(), &data.live.access),
                None => Err("The pod is gone".into()),
            };
            match launch {
                Err(reason) => button.label("View logs").disabled(true).tooltip(reason),
                Ok(target) => {
                    let connection = data.live.connection().clone();
                    let (dock, row) = (data.dock.clone(), data.row.clone());
                    button.label("View logs").on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        let _ = dock.update(cx, |dock, cx| {
                            let origin = LogOrigin::new(&row, connection.clone());
                            dock.open(origin, target.clone(), window, cx);
                        });
                    })
                }
            }
        }
    };
    Some(element.into_any_element())
}

#[cfg(test)]
mod tests {
    use cluster::{
        NodeResource, NodeScheduling, NodeStatus, NodeSystemInfo, PodStatus, ReadyCount,
    };

    use crate::issue::{IssueKey, IssueObject, IssueRule, IssueSeverity};

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
            is_finished: false,
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

    fn issue(shown: IssueObject, target: Option<ResourceKey>, action: IssueAction) -> Issue {
        Issue {
            key: IssueKey {
                rule: IssueRule::PodCrash,
                object: shown.clone(),
            },
            severity: IssueSeverity::Critical,
            reason: "CrashLoopBackOff".into(),
            cause: "The container exits on start.".to_owned(),
            subject: shown.clone(),
            shown,
            container: None,
            count: 1,
            since: jiff::Timestamp::UNIX_EPOCH,
            onset: None,
            target,
            action,
        }
    }

    fn pod_issue() -> Issue {
        let pod = IssueObject::pod("payments", "api-7d9f8c-x2k4q");
        let target = pod.target();
        issue(pod, target, IssueAction::Open)
    }

    #[test]
    fn object_line_names_namespace_pod_container() {
        let mut crashed = pod_issue();
        crashed.container = Some("api".to_owned());
        assert_eq!(
            object_line(&crashed),
            "payments / api-7d9f8c-x2k4q · container api"
        );
    }

    #[test]
    fn object_line_counts_grouped_pods() {
        let mut group = pod_issue();
        group.count = 3;
        assert_eq!(object_line(&group), "payments / api-7d9f8c-x2k4q · 3 pods");
    }

    #[test]
    fn object_line_cluster_object_has_no_namespace() {
        let node = IssueObject::node("worker-02");
        let issue = issue(node, None, IssueAction::Open);
        assert_eq!(object_line(&issue), "worker-02");
    }

    #[test]
    fn view_logs_action_for_crash() {
        let mut crash = pod_issue();
        crash.action = IssueAction::ViewLogs { container: None };
        assert_eq!(attention_action(&crash), Some(AttentionAction::ViewLogs));
    }

    #[test]
    fn pod_target_reads_see_why() {
        assert_eq!(
            attention_action(&pod_issue()),
            Some(AttentionAction::Open {
                label: "See why".to_owned()
            })
        );
    }

    #[test]
    fn secret_target_reads_open_secret() {
        let secret = IssueObject::new("Secret", Some("payments"), "api-tls");
        let target = secret.target();
        assert!(target.is_some());
        let issue = issue(secret, target, IssueAction::Open);
        assert_eq!(
            attention_action(&issue),
            Some(AttentionAction::Open {
                label: "Open Secret".to_owned()
            })
        );
    }

    #[test]
    fn policy_kinds_read_short_in_the_open_label() {
        let hpa = IssueObject::new("HorizontalPodAutoscaler", Some("payments"), "api");
        let target = hpa.target();
        assert!(target.is_some());
        let issue = issue(hpa, target, IssueAction::Open);
        assert_eq!(
            attention_action(&issue),
            Some(AttentionAction::Open {
                label: "Open HPA".to_owned()
            })
        );
    }
    #[test]
    fn no_target_has_no_action() {
        let widget = IssueObject::new("Widget", Some("payments"), "w");
        let issue = issue(widget, None, IssueAction::Open);
        assert_eq!(attention_action(&issue), None);
    }

    #[test]
    fn attention_shows_at_most_six() {
        let issues: Vec<Issue> = (0..9).map(|_| pod_issue()).collect();
        assert_eq!(attention_issues(&issues).len(), ATTENTION_ROWS);
        assert_eq!(attention_issues(&issues[..2]).len(), 2);
    }

    #[test]
    fn only_deployment_rows_of_the_feed_open_the_diff() {
        let entry = |kind, object_kind: &str, name: &str| ChangeEntry {
            at: jiff::Timestamp::UNIX_EPOCH,
            kind,
            object: format!("payments/{name}"),
            text: String::new(),
            count: 1,
            actor: None,
            actor_source: crate::recent_changes::ActorSource::EventSource,
            replica_set: None,
            target: ResourceKey::of_object(object_kind, Some("payments"), name),
        };
        let diffable = HashSet::from([("payments", "api")]);
        assert!(opens_diff(
            &entry(ChangeKind::Deployment, "Deployment", "api"),
            &diffable
        ));
        // A Deployment the feed does not hold (or holds without a selector) reveals instead.
        assert!(!opens_diff(
            &entry(ChangeKind::Deployment, "Deployment", "web"),
            &diffable
        ));
        assert!(!opens_diff(
            &entry(ChangeKind::Autoscaler, "HorizontalPodAutoscaler", "api"),
            &diffable
        ));
    }
}
