//! The Monitor tab body: toolbar, charts or Table view, and the source note, by feed status.

use std::rc::Rc;

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::cluster_metrics::{FeedStatus, source_note};
use crate::cluster_session::LiveCluster;
use crate::drawer::{DrawerState, MonitorRange, MonitorScope, MonitorState};
use crate::history_rings::{COARSE_POINTS, Resolution, TICKS_PER_COARSE};
use crate::monitor_data::{MonitorData, MonitorRow};
use crate::monitor_source::{SourceFetch, SourceView, fallback_note, step_text};
use crate::status_tone::{StatusTone, tone_color};
use crate::usage_chart::{UsageChartModel, usage_chart_card};
use crate::usage_format::{Measure, format_offset};

const CHART_HEIGHT: f32 = 110.;
const CHART_HEIGHT_EXPANDED: f32 = 140.;
const CHART_MIN_WIDTH: f32 = 280.;
/// The most rows the Table view shows: the sampler holds every coarse point and the fine ticks newer
/// than the last one (fewer than 20, or that tick would have closed a coarse point); a source answer
/// holds at most `cluster::MAX_POINTS` points.
const SAMPLER_ROWS: usize = COARSE_POINTS + TICKS_PER_COARSE - 1;
const SOURCE_ROWS: usize = cluster::MAX_POINTS as usize;
const MAX_TABLE_ROWS: usize = if SAMPLER_ROWS > SOURCE_ROWS {
    SAMPLER_ROWS
} else {
    SOURCE_ROWS
};
const SHORT_HISTORY_TIP: &str = "Showing data since k8sBoard connected; choose a metrics source in Settings › Metrics for up to 30 days";
const SOURCE_NOTE: &str = "CPU and memory: metrics-server, sampled by k8sBoard every 15s while the app is open. Network and disk I/O: kubelet stats summary and cAdvisor through the API server node proxy, sampled every 15s while needed. Kept 24 hours.";
const METRICS_UNAVAILABLE_TITLE: &str = "Metrics unavailable";
const KUBELET_UNAVAILABLE_TITLE: &str = "Network and disk I/O unavailable";
/// A Table view column of a rate or of the time.
const COLUMN_WIDTH: f32 = 78.;

/// What the tab needs besides the cached data.
pub(crate) struct MonitorView<'a> {
    pub(crate) state: &'a MonitorState,
    /// The feed of the subject: pods for pods and workloads, nodes for nodes.
    pub(crate) status: &'a FeedStatus,
    /// The kubelet feed, for the Network and Disk I/O cards.
    pub(crate) kubelet_status: &'a FeedStatus,
    /// The pods feed's note, such as `no access in web`.
    pub(crate) note: Option<&'a str>,
    /// The container sub-tab has no scope selector.
    has_scope: bool,
    pub(crate) is_expanded: bool,
    /// The metrics source query (spec 0048): `Some` makes the tab a source view with six ranges.
    source: Option<&'a SourceFetch>,
    /// Why a saved source is not serving the tab (checking, unreachable, not valid).
    source_note: Option<String>,
}

impl<'a> MonitorView<'a> {
    /// Pods and workloads read the pods feed, and choose a part of the subject.
    pub(crate) fn of_pods(state: &'a DrawerState, live: &'a LiveCluster) -> Self {
        Self {
            state: &state.monitor,
            status: &live.metrics.pods.status,
            kubelet_status: &live.metrics.kubelet.status,
            note: live.metrics.pods.note.as_deref(),
            has_scope: true,
            is_expanded: state.is_expanded,
            source: state.monitor.source.as_ref(),
            source_note: source_note(&live.metrics.source),
        }
    }

    /// One container: the pods feed, and nothing to choose.
    pub(crate) fn of_container(state: &'a DrawerState, live: &'a LiveCluster) -> Self {
        Self {
            has_scope: false,
            ..Self::of_pods(state, live)
        }
    }

    /// A node reads the nodes feed.
    pub(crate) fn of_nodes(state: &'a DrawerState, live: &'a LiveCluster) -> Self {
        Self {
            state: &state.monitor,
            status: &live.metrics.nodes.status,
            kubelet_status: &live.metrics.kubelet.status,
            note: None,
            has_scope: true,
            is_expanded: state.is_expanded,
            source: state.monitor.source.as_ref(),
            source_note: source_note(&live.metrics.source),
        }
    }
}

/// The tab body. A source view draws from the source's answer; everything else (no source, a
/// query still running on a short range, a failed query on a short range) draws the sampler.
pub(crate) fn monitor_tab(view: &MonitorView<'_>, cx: &Context<AppShell>) -> AnyElement {
    let Some(fetch) = view.source else {
        return sampler_tab(view, view.source_note.clone(), cx);
    };
    let muted = |text: String| {
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(text)
    };
    let is_long = view.state.range.is_long();
    match &fetch.view {
        Some(SourceView::Charts { data, was_cut }) => {
            let mut column = v_flex().gap_3().child(toolbar(view, Some(data), cx));
            if let Some(reason) = &fetch.last_failure {
                column = column.child(muted(format!(
                    "Last query failed: {reason}. Showing older results."
                )));
            }
            if *was_cut {
                column = column.child(muted(
                    "Some series were left out (more than 64).".to_owned(),
                ));
            }
            column = if view.state.is_table {
                column.child(table(&data.rows, true, cx))
            } else {
                column.child(charts(
                    data.charts.iter().chain(&data.kubelet_charts),
                    view.is_expanded,
                    cx,
                ))
            };
            column
                .child(muted(format!(
                    "CPU, memory, network, and disk I/O: {}, step {}. Request and limit lines show the current spec.",
                    fetch.key.source.display(),
                    step_text(view.state.range.source_step())
                )))
                .into_any_element()
        }
        Some(SourceView::Fallback(reason)) if is_long => v_flex()
            .gap_3()
            .child(toolbar(view, None, cx))
            .child(
                Alert::warning("monitor-source-failed", reason.clone())
                    .title("Metrics source query failed"),
            )
            .into_any_element(),
        Some(SourceView::Fallback(reason)) => sampler_tab(view, Some(fallback_note(reason)), cx),
        None if is_long => v_flex()
            .gap_3()
            .child(toolbar(view, None, cx))
            .child(muted(format!("Querying {}…", fetch.key.source.display())).text_sm())
            .into_any_element(),
        None => sampler_tab(
            view,
            Some(format!("Querying {}…", fetch.key.source.display())),
            cx,
        ),
    }
}

/// The sampler body: today's Monitor. `lead_note` is a muted line under the toolbar.
fn sampler_tab(
    view: &MonitorView<'_>,
    lead_note: Option<String>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let muted = |text: SharedString| {
        div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element()
    };
    let is_metrics_down = is_down(view.status);
    let is_kubelet_down = is_down(view.kubelet_status);
    if is_metrics_down && is_kubelet_down {
        return v_flex()
            .gap_3()
            .child(unavailable(
                "monitor-unavailable",
                METRICS_UNAVAILABLE_TITLE,
                view.status,
            ))
            .child(unavailable(
                "monitor-kubelet-unavailable",
                KUBELET_UNAVAILABLE_TITLE,
                view.kubelet_status,
            ))
            .into_any_element();
    }
    let data = view.state.cache.as_ref().map(|cache| &cache.data);
    let mut column = v_flex().gap_3();
    if is_metrics_down {
        column = column.child(unavailable(
            "monitor-unavailable",
            METRICS_UNAVAILABLE_TITLE,
            view.status,
        ));
    }
    column = column.child(toolbar(view, data, cx));
    if let Some(note) = view.note {
        column = column.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(note.to_owned()),
        );
    }
    if let Some(note) = lead_note {
        column = column.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(note),
        );
    }
    if let FeedStatus::Interrupted(reason) = view.status {
        column = column.child(muted(
            format!("Last poll failed: {reason}. Showing older samples.").into(),
        ));
    }
    let Some(data) = data else {
        return column
            .child(muted("Collecting the first sample…".into()))
            .into_any_element();
    };
    // A feed that is down has no cards: its alert says why.
    let usage_charts = if is_metrics_down {
        &[][..]
    } else {
        &data.charts[..]
    };
    let kubelet_charts = if is_kubelet_down {
        &[][..]
    } else {
        &data.kubelet_charts[..]
    };
    if !is_metrics_down && usage_charts.is_empty() {
        column = column.child(muted("Collecting the first sample…".into()));
    }
    column = if view.state.is_table {
        column.child(table(&data.rows, !data.charts.is_empty(), cx))
    } else {
        column.child(charts(
            usage_charts.iter().chain(kubelet_charts),
            view.is_expanded,
            cx,
        ))
    };
    if is_kubelet_down {
        column = column.child(unavailable(
            "monitor-kubelet-unavailable",
            KUBELET_UNAVAILABLE_TITLE,
            view.kubelet_status,
        ));
    }
    column
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(SOURCE_NOTE),
        )
        .into_any_element()
}

/// A denied, missing, or broken feed. A failed poll keeps retrying.
fn is_down(status: &FeedStatus) -> bool {
    matches!(status, FeedStatus::Unavailable(_) | FeedStatus::Failed(_))
}

/// The alert of a feed that is down: it replaces that feed's cards.
fn unavailable(id: &'static str, title: &'static str, status: &FeedStatus) -> AnyElement {
    let message = match status {
        FeedStatus::Failed(reason) => format!("{reason} Retrying."),
        FeedStatus::Unavailable(reason) => reason.clone(),
        FeedStatus::Checking
        | FeedStatus::Waiting
        | FeedStatus::Live
        | FeedStatus::Interrupted(_) => String::new(),
    };
    Alert::warning(id, message).title(title).into_any_element()
}

fn toolbar(
    view: &MonitorView<'_>,
    data: Option<&MonitorData>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let state = view.state;
    // The click indexes this same set, so `30d` can never map to another range.
    let shown: &'static [MonitorRange] = if view.source.is_some() {
        &MonitorRange::SOURCE
    } else {
        &MonitorRange::SAMPLER
    };
    let ranges = ButtonGroup::new("monitor-range")
        .outline()
        .small()
        .children(shown.iter().map(|range| {
            // Without a source, a range longer than the history reads dimmed: the data starts
            // when the app did.
            let is_short =
                view.source.is_none() && data.is_none_or(|data| data.is_short_for(*range));
            let button = Button::new(range.label())
                .label(range.label())
                .selected(*range == state.range);
            if is_short {
                button.opacity(0.55).tooltip(SHORT_HISTORY_TIP)
            } else {
                button
            }
        }))
        .on_click(cx.listener(move |shell, clicks: &Vec<usize>, _, cx| {
            if let Some(range) = clicks.first().and_then(|index| shown.get(*index)) {
                shell.set_monitor_range(*range, cx);
            }
        }));
    let mut row = h_flex().gap_2().items_center().flex_wrap().child(ranges);
    if let (true, Some(data)) = (view.has_scope, data) {
        row = row.child(scope_selector(data, cx));
    }
    row = row.child(
        Button::new("monitor-table")
            .label("Table view")
            .outline()
            .small()
            .selected(state.is_table)
            .on_click(cx.listener(|shell, _, _, cx| shell.toggle_monitor_table(cx))),
    );
    row.child(status_text(view, data, cx)).into_any_element()
}

fn scope_selector(data: &MonitorData, cx: &Context<AppShell>) -> AnyElement {
    let current = data
        .choices
        .iter()
        .find(|choice| choice.scope == data.scope)
        .map(|choice| choice.label.clone())
        .unwrap_or_default();
    if data.choices.len() < 2 {
        return div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(current)
            .into_any_element();
    }
    let choices = data.choices.clone();
    let shell = cx.weak_entity();
    Button::new("monitor-scope")
        .label(current)
        .outline()
        .small()
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, _| {
            choices.iter().fold(menu, |menu, choice| {
                let scope: MonitorScope = choice.scope.clone();
                let shell = shell.clone();
                menu.item(
                    PopupMenuItem::new(choice.label.clone()).on_click(move |_, _, cx| {
                        let _ = shell
                            .update(cx, |shell, cx| shell.set_monitor_scope(scope.clone(), cx));
                    }),
                )
            })
        })
        .into_any_element()
}

/// The right end of the toolbar: how the data is flowing.
fn status_text(
    view: &MonitorView<'_>,
    data: Option<&MonitorData>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let text = div().ml_auto().text_xs().text_color(theme.muted_foreground);
    if let Some(fetch) = view.source {
        match &fetch.view {
            Some(SourceView::Charts { .. }) => {
                let step = step_text(view.state.range.source_step());
                return text
                    .child(format!("step {step} · metrics source"))
                    .into_any_element();
            }
            None => return text.child("querying…").into_any_element(),
            Some(SourceView::Fallback(_)) => {}
        }
    }
    match view.status {
        FeedStatus::Interrupted(reason) => {
            let tooltip = SharedString::from(reason.clone());
            text.id("monitor-status")
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .child("paused · retrying")
                .into_any_element()
        }
        FeedStatus::Live => {
            if let Some(since) = data.and_then(|data| data.stale_since) {
                let age = format_age(Some(since), jiff::Timestamp::now());
                return text
                    .child(format!("stale · last sample {age} ago"))
                    .into_any_element();
            }
            let step = match view.state.range.resolution() {
                Resolution::Fine => "15s",
                Resolution::Coarse => "5m",
            };
            text.child(format!("step {step} · live")).into_any_element()
        }
        _ => text.child("collecting…").into_any_element(),
    }
}

/// Two per row when the drawer is expanded, one otherwise.
fn charts<'a>(
    models: impl Iterator<Item = &'a Rc<UsageChartModel>>,
    is_expanded: bool,
    cx: &Context<AppShell>,
) -> AnyElement {
    let height = px(if is_expanded {
        CHART_HEIGHT_EXPANDED
    } else {
        CHART_HEIGHT
    });
    let cards = models.map(|model| {
        let card = usage_chart_card(model.clone(), height, cx);
        if is_expanded {
            div()
                .flex_1()
                .min_w(px(CHART_MIN_WIDTH))
                .child(card)
                .into_any_element()
        } else {
            div().w_full().child(card).into_any_element()
        }
    });
    if is_expanded {
        h_flex()
            .flex_wrap()
            .gap_3()
            .children(cards)
            .into_any_element()
    } else {
        v_flex().gap_3().children(cards).into_any_element()
    }
}

/// The points of the range, newest first: how long ago, CPU, memory, receive, transmit, read,
/// write. `has_metrics` is whether the metrics feed has a sample at all.
fn table(rows: &[MonitorRow], has_metrics: bool, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let mono = theme.mono_font_family.clone();
    let bad = tone_color(StatusTone::Bad, cx);
    let muted = theme.muted_foreground;
    let cell = |text: String| div().w(px(COLUMN_WIDTH)).child(text);
    let value = |value: Option<f64>, measure: Measure| match value {
        Some(value) => cell(measure.format(value)).into_any_element(),
        // Without a metrics tick there is no CPU or Memory to be missing: it is just not read.
        None if !has_metrics => cell("—".to_owned()).text_color(muted).into_any_element(),
        None => cell("not running".to_owned())
            .text_color(muted)
            .into_any_element(),
    };
    let rate = |value: Option<f64>| match value {
        Some(value) => cell(Measure::Rate.format(value)).into_any_element(),
        None => cell("—".to_owned()).text_color(muted).into_any_element(),
    };
    let header = h_flex().gap_2().text_color(muted).children(
        [
            "Time", "CPU", "Memory", "Receive", "Transmit", "Read", "Write",
        ]
        .map(|title| cell(title.to_owned())),
    );
    v_flex()
        .font_family(mono)
        .text_xs()
        .gap_1()
        .child(header)
        .children(rows.iter().take(MAX_TABLE_ROWS).map(|row| {
            h_flex()
                .gap_2()
                .child(cell(format_offset(row.offset)))
                .child(value(row.cpu, Measure::Cpu))
                .child(value(row.memory, Measure::Bytes))
                .child(rate(row.network.map(|pair| pair.first)))
                .child(rate(row.network.map(|pair| pair.second)))
                .child(rate(row.disk.map(|pair| pair.first)))
                .child(rate(row.disk.map(|pair| pair.second)))
                .children(row.is_oom.then(|| div().text_color(bad).child("OOMKilled")))
        }))
        .into_any_element()
}
