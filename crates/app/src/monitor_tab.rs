//! The Monitor tab body: toolbar, charts or Table view, and the source note, by feed status.

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
use crate::cluster_metrics::FeedStatus;
use crate::cluster_session::LiveCluster;
use crate::drawer::{DrawerState, MonitorRange, MonitorScope, MonitorState};
use crate::history_rings::{COARSE_POINTS, Resolution, TICKS_PER_COARSE};
use crate::monitor_data::{MonitorData, MonitorRow};
use crate::status_tone::{StatusTone, tone_color};
use crate::usage_chart::usage_chart_card;
use crate::usage_format::{Measure, format_offset};

const CHART_HEIGHT: f32 = 110.;
const CHART_HEIGHT_EXPANDED: f32 = 140.;
const CHART_MIN_WIDTH: f32 = 280.;
/// The most points a range can hold: every coarse point, and the fine ticks newer than the last
/// one (fewer than 20, or that tick would have closed a coarse point).
const MAX_TABLE_ROWS: usize = COARSE_POINTS + TICKS_PER_COARSE - 1;
const SHORT_HISTORY_TIP: &str =
    "Showing data since k8sBoard connected; connect Prometheus for 30 days";
const SOURCE_NOTE: &str = "CPU and memory: metrics-server, sampled by k8sBoard every 15s while the app is open; kept 24 hours.";

/// What the tab needs besides the cached data.
pub(crate) struct MonitorView<'a> {
    pub(crate) state: &'a MonitorState,
    /// The feed of the subject: pods for pods and workloads, nodes for nodes.
    pub(crate) status: &'a FeedStatus,
    /// The pods feed's note, such as `no access in web`.
    pub(crate) note: Option<&'a str>,
    /// The container sub-tab has no scope selector.
    has_scope: bool,
    pub(crate) is_expanded: bool,
}

impl<'a> MonitorView<'a> {
    /// Pods and workloads read the pods feed, and choose a part of the subject.
    pub(crate) fn of_pods(state: &'a DrawerState, live: &'a LiveCluster) -> Self {
        Self {
            state: &state.monitor,
            status: &live.metrics.pods.status,
            note: live.metrics.pods.note.as_deref(),
            has_scope: true,
            is_expanded: state.is_expanded,
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
            note: None,
            has_scope: true,
            is_expanded: state.is_expanded,
        }
    }
}

pub(crate) fn monitor_tab(view: &MonitorView<'_>, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let muted = |text: SharedString| {
        div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element()
    };
    match view.status {
        FeedStatus::Unavailable(_) | FeedStatus::Failed(_) => return unavailable(view.status),
        FeedStatus::Checking
        | FeedStatus::Waiting
        | FeedStatus::Live
        | FeedStatus::Interrupted(_) => {}
    }
    let data = view.state.cache.as_ref().map(|cache| &cache.data);
    let mut column = v_flex().gap_3().child(toolbar(view, data, cx));
    if let Some(note) = view.note {
        column = column.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(note.to_owned()),
        );
    }
    if let FeedStatus::Interrupted(reason) = view.status {
        column = column.child(muted(
            format!("Last poll failed: {reason}. Showing older samples.").into(),
        ));
    }
    let Some(data) = data.filter(|data| !data.charts.is_empty()) else {
        return column
            .child(muted("Collecting the first sample…".into()))
            .into_any_element();
    };
    column = if view.state.is_table {
        column.child(table(&data.rows, cx))
    } else {
        column.child(charts(data, view.is_expanded, cx))
    };
    column
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(SOURCE_NOTE),
        )
        .into_any_element()
}

/// A denied, missing, or broken metrics API replaces the whole tab. A failed poll keeps retrying.
fn unavailable(status: &FeedStatus) -> AnyElement {
    let message = match status {
        FeedStatus::Failed(reason) => format!("{reason} Retrying."),
        FeedStatus::Unavailable(reason) => reason.clone(),
        FeedStatus::Checking
        | FeedStatus::Waiting
        | FeedStatus::Live
        | FeedStatus::Interrupted(_) => String::new(),
    };
    Alert::warning("monitor-unavailable", message)
        .title("Metrics unavailable")
        .into_any_element()
}

fn toolbar(
    view: &MonitorView<'_>,
    data: Option<&MonitorData>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let state = view.state;
    let ranges = ButtonGroup::new("monitor-range")
        .outline()
        .small()
        .children(MonitorRange::ALL.into_iter().map(|range| {
            // A range longer than the history reads dimmed: the data starts when the app did.
            let is_short = data.is_none_or(|data| data.is_short_for(range));
            let button = Button::new(range.label())
                .label(range.label())
                .selected(range == state.range);
            if is_short {
                button.opacity(0.55).tooltip(SHORT_HISTORY_TIP)
            } else {
                button
            }
        }))
        .on_click(cx.listener(|shell, clicks: &Vec<usize>, _, cx| {
            if let Some(range) = clicks
                .first()
                .and_then(|index| MonitorRange::ALL.get(*index))
            {
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
fn charts(data: &MonitorData, is_expanded: bool, cx: &Context<AppShell>) -> AnyElement {
    let height = px(if is_expanded {
        CHART_HEIGHT_EXPANDED
    } else {
        CHART_HEIGHT
    });
    let cards = data.charts.iter().map(|model| {
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

/// The points of the range, newest first: how long ago, CPU, memory.
fn table(rows: &[MonitorRow], cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let mono = theme.mono_font_family.clone();
    let bad = tone_color(StatusTone::Bad, cx);
    let cell = |text: String| div().w(px(96.)).child(text);
    let value = |value: Option<f64>, measure: Measure| match value {
        Some(value) => div()
            .w(px(96.))
            .child(measure.format(value))
            .into_any_element(),
        None => div()
            .w(px(96.))
            .text_color(theme.muted_foreground)
            .child("not running")
            .into_any_element(),
    };
    let header = h_flex()
        .gap_2()
        .text_color(theme.muted_foreground)
        .child(cell("Time".to_owned()))
        .child(cell("CPU".to_owned()))
        .child(cell("Memory".to_owned()));
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
                .children(row.is_oom.then(|| div().text_color(bad).child("OOMKilled")))
        }))
        .into_any_element()
}
