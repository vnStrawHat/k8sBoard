//! The overview drawer shared by the explorer kinds. The sections come from the row
//! builders; this module only renders them.

use std::rc::Rc;

use cluster::PodSummary;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::cluster_session::{ClusterSession, LiveCluster};
use crate::drawer::{
    DrawerBody, DrawerHeader, DrawerState, DrawerTab, absent_text, chips, created_text,
    drawer_frame, drawer_tab_bar, drawer_tabs, expand_toggle, link_text, menu_button, port_row,
    section_title, shown_tab, tab_titles, truncated_text, wide_detail_row, yaml_body,
};
use crate::kind_row::{
    DAEMON_SET_KIND, DetailRow, KindCell, KindRow, PodOwner, STATEFUL_SET_KIND, owns_pod,
};
use crate::object_events::{event_subject, recent_events};
use crate::resource_actions::{kind_menu, port_forward_reason};
use crate::resource_kind::ResourceKind;
use crate::status_tone::{pod_status_label, tone_color, toned_text};
use crate::table_selection::ResourceKey;
use crate::workload_rows::sort_by_ordinal;

/// Bounds the render cost of a workload with very many pods.
const MAX_RELATED_PODS: usize = 50;

pub(crate) fn kind_drawer(
    kind: ResourceKind,
    row: &KindRow,
    state: &DrawerState,
    live: &LiveCluster,
    session: &Entity<ClusterSession>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = jiff::Timestamp::now();
    let header = DrawerHeader {
        kind_badge: kind.badge(),
        name: header_name(row),
        subtitle: subtitle(row, now, cx),
        menu: kind_menu_button(kind, row, session, cx.weak_entity()),
        expand: expand_toggle(state, cx),
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
    };
    let key = ResourceKey::of_row(kind, row);
    let events = event_subject(&key).and_then(|subject| live.events_of(&subject));
    let tabs = drawer_tabs(&key);
    let shown = shown_tab(tabs, state.tab);
    let body = match shown {
        DrawerTab::Events => DrawerBody::Scrolling(recent_events(events, cx)),
        DrawerTab::Yaml => yaml_body(state),
        DrawerTab::Overview | DrawerTab::Containers => {
            DrawerBody::Scrolling(overview(kind, row, live, now, cx))
        }
    };
    let tab_bar = drawer_tab_bar(tab_titles(tabs, 0, events), shown, cx);
    drawer_frame(header, tab_bar, body, state.width(), cx).into_any_element()
}

/// The event title for events, else the object name.
fn header_name(row: &KindRow) -> SharedString {
    match &row.event {
        Some(event) => event.title.clone(),
        None => row.name.clone().into(),
    }
}

/// The status, then `· namespace · created 2d ago`; events show their source instead of an age.
fn subtitle(row: &KindRow, now: jiff::Timestamp, cx: &App) -> AnyElement {
    let detail: Vec<String> = match &row.event {
        Some(event) => row
            .namespace
            .iter()
            .cloned()
            .chain(event.source.iter().map(ToString::to_string))
            .collect(),
        None => row
            .namespace
            .iter()
            .cloned()
            .chain(created_text(row.created_at, now))
            .collect(),
    };
    h_flex()
        .gap_1()
        .text_sm()
        .child(toned_text(row.status.clone(), cx))
        .children((!detail.is_empty()).then(|| {
            div()
                .text_color(cx.theme().muted_foreground)
                .child(format!("· {}", detail.join(" · ")))
        }))
        .into_any_element()
}

/// The menu reads the session when it opens, so it shows the access state and the row of
/// that moment.
fn kind_menu_button(
    kind: ResourceKind,
    row: &KindRow,
    session: &Entity<ClusterSession>,
    shell: WeakEntity<AppShell>,
) -> AnyElement {
    let session = session.clone();
    let key = ResourceKey::of_row(kind, row);
    menu_button()
        .dropdown_menu(move |menu, _, cx| {
            let Some(live) = session.read(cx).live() else {
                return menu;
            };
            let current = live.kind_list(kind).and_then(|explorer| {
                explorer
                    .list
                    .items()
                    .iter()
                    .find(|row| key.is_row(kind, row))
            });
            match current {
                Some(row) => kind_menu(menu, kind, row, &live.access, &shell),
                None => menu,
            }
        })
        .into_any_element()
}

/// The row's sections in order, then the related pods, then the labels.
fn overview(
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let forward_reason = port_forward_reason(&live.access);
    // Gives every element that needs an id one that is unique inside the drawer.
    let mut next_id = 0_usize;
    let mut column = v_flex();
    for section in &row.sections {
        column = column.child(section_title(section.title, cx));
        if section.rows.is_empty() {
            column = column.child(absent_text(cx));
        }
        for detail in &section.rows {
            next_id += 1;
            column = column.child(detail_element(detail, next_id, &forward_reason, now, cx));
        }
    }
    if let Some(owner) = &row.related_pods {
        column = column.child(pods_section(owner, live, cx));
    }
    if kind.has_labels() {
        column = column
            .child(section_title("Labels", cx))
            .child(chips(&row.labels, cx));
    }
    column.into_any_element()
}

fn detail_element(
    detail: &DetailRow,
    id: usize,
    forward_reason: &SharedString,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    match detail {
        DetailRow::Field { label, value } => {
            wide_detail_row(label.clone(), field_value(value, id, now, cx), cx).into_any_element()
        }
        DetailRow::Chips(terms) => chips(terms, cx),
        DetailRow::Note(text) => div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.clone())
            .into_any_element(),
        DetailRow::Code(text) => code_block(text, cx),
        DetailRow::Link {
            label,
            text,
            target,
        } => {
            let link = link_text(id, text, target.clone(), cx);
            wide_detail_row(label.clone(), link, cx).into_any_element()
        }
        DetailRow::Port { text } => port_row(text, id, forward_reason, cx),
        DetailRow::Stacked { label, value } => {
            stacked_row(label, field_value(value, id, now, cx), id, cx)
        }
    }
}

/// Preformatted text that wraps, such as an event message.
fn code_block(text: &SharedString, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .w_full()
        .min_w_0()
        .px_2()
        .py_1p5()
        .rounded(theme.radius)
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .child(text.clone())
        .into_any_element()
}

/// The label above its value, for labels that do not fit the label column.
fn stacked_row(label: &SharedString, value: AnyElement, id: usize, cx: &App) -> AnyElement {
    v_flex()
        .py_1()
        .text_sm()
        .child(
            truncated_text(("stacked", id), label.clone()).text_color(cx.theme().muted_foreground),
        )
        .child(div().min_w_0().overflow_hidden().child(value))
        .into_any_element()
}

fn field_value(value: &KindCell, id: usize, now: jiff::Timestamp, cx: &App) -> AnyElement {
    let mono = cx.theme().mono_font_family.clone();
    match value {
        KindCell::Text(text) => truncated_text(("detail", id), text.clone()).into_any_element(),
        KindCell::Mono(text) => truncated_text(("detail", id), text.clone())
            .font_family(mono)
            .into_any_element(),
        KindCell::Qualified { prefix, text } => {
            let text = match prefix {
                Some(prefix) => SharedString::from(format!("{prefix}/{text}")),
                None => text.clone(),
            };
            truncated_text(("detail", id), text)
                .font_family(mono)
                .into_any_element()
        }
        KindCell::Toned(label) => toned_text(label.clone(), cx).truncate().into_any_element(),
        KindCell::Absent => absent_text(cx).into_any_element(),
        KindCell::Duration {
            started_at: None, ..
        } => absent_text(cx).into_any_element(),
        KindCell::Duration {
            started_at,
            finished_at,
        } => div()
            .truncate()
            .child(format_age(*started_at, finished_at.unwrap_or(now)))
            .into_any_element(),
        KindCell::Age { at: None, .. } => absent_text(cx).into_any_element(),
        KindCell::Age { at: Some(at), tone } => {
            let text = div()
                .truncate()
                .child(format!("{at} ({} ago)", format_age(Some(*at), now)));
            match tone {
                Some(tone) => text.text_color(tone_color(*tone, cx)),
                None => text,
            }
            .into_any_element()
        }
    }
}

/// The pods of `owner`, read from the live pods list at render time so they stay current.
/// A click opens the pod on the Pods screen.
fn pods_section(owner: &PodOwner, live: &LiveCluster, cx: &Context<AppShell>) -> AnyElement {
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
    let detail = if matches!(owner, PodOwner::Controller { kind, .. } if *kind == DAEMON_SET_KIND) {
        PodRowDetail::StatusAndNode
    } else {
        PodRowDetail::StatusOnly
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

/// What a related-pod row shows after the pod name.
#[derive(Clone, Copy)]
enum PodRowDetail {
    StatusOnly,
    StatusAndNode,
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
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(key.clone(), cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(theme.mono_font_family.clone())
                .child(pod.name.clone()),
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
