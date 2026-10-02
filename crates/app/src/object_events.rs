//! The recent events of the object whose drawer is open. Event text is arbitrary, so nothing
//! here logs it.

use cluster::{EventSummary, InvolvedObject};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div,
};

use crate::age::format_age;
use crate::cluster_session::LiveList;
use crate::event_rows::event_tone;
use crate::resource_kind::ResourceKind;
use crate::status_tone::toned_text;
use crate::table_selection::ResourceKey;

/// Bounds the render cost of an object with very many events.
const MAX_EVENT_ROWS: usize = 50;
/// The message lines shown per row; the tooltip has the whole message.
const MESSAGE_LINES: usize = 3;

/// What a drawer watch (the object events, the related objects) must do when the selection
/// changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SubjectChange<S> {
    Keep,
    Stop,
    Start(S),
}

/// Pure: what a watch must do when the selection becomes `next`. `running` is the session's
/// subject, which is `None` while a debounced start is still pending.
pub(crate) fn subject_change<S: PartialEq>(
    running: Option<&S>,
    next: Option<S>,
) -> SubjectChange<S> {
    if running == next.as_ref() {
        return SubjectChange::Keep;
    }
    match next {
        Some(subject) => SubjectChange::Start(subject),
        None => SubjectChange::Stop,
    }
}

/// The object whose events a drawer shows; `None` for an event's own drawer.
pub(crate) fn event_subject(key: &ResourceKey) -> Option<InvolvedObject> {
    match key {
        ResourceKey::Pod { namespace, name } => Some(InvolvedObject {
            kind: "Pod".to_owned(),
            namespace: Some(namespace.clone()),
            name: name.clone(),
        }),
        ResourceKey::Node { name } => Some(InvolvedObject {
            kind: "Node".to_owned(),
            namespace: None,
            name: name.clone(),
        }),
        ResourceKey::Kind {
            kind: ResourceKind::Events,
            ..
        } => None,
        // A release is a set of Secrets; the events of those are not the release's.
        ResourceKey::Kind {
            kind: ResourceKind::HelmReleases,
            ..
        } => None,
        ResourceKey::Kind {
            kind,
            namespace,
            name,
        } => Some(InvolvedObject {
            kind: kind.object_kind().to_owned(),
            namespace: namespace.clone(),
            name: name.clone(),
        }),
    }
}

/// `Events`, or `Events {n}` once the list is ready.
pub(crate) fn events_title(list: Option<&LiveList<EventSummary>>) -> String {
    match list.and_then(LiveList::ready_count) {
        Some(count) => format!("Events {count}"),
        None => "Events".to_owned(),
    }
}

fn event_note(list: Option<&LiveList<EventSummary>>) -> Option<&'static str> {
    match list {
        None | Some(LiveList::Loading) => Some("Loading events…"),
        Some(LiveList::Failed { .. }) => Some("Events are unavailable"),
        Some(LiveList::Ready { items, .. }) if items.is_empty() => Some("No recent events"),
        Some(LiveList::Ready { .. }) => None,
    }
}

/// The note, or at most `MAX_EVENT_ROWS` rows (already newest first). It has no title, so a
/// section or a tab can host it unchanged. Rows are not interactive; a tooltip shows the full
/// message.
pub(crate) fn recent_events(list: Option<&LiveList<EventSummary>>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    if let Some(note) = event_note(list) {
        let reason = list.and_then(LiveList::failure);
        return v_flex()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(note),
            )
            .children(reason.map(|reason| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(reason.to_owned())
            }))
            .into_any_element();
    }
    let events = list.map_or(&[][..], LiveList::items);
    let hidden = events.len().saturating_sub(MAX_EVENT_ROWS);
    let now = jiff::Timestamp::now();
    v_flex()
        .children(
            events
                .iter()
                .take(MAX_EVENT_ROWS)
                .enumerate()
                .map(|(index, event)| event_item(index, event, now, cx)),
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

fn event_item(index: usize, event: &EventSummary, now: jiff::Timestamp, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let message = SharedString::from(event.message.clone());
    let tooltip_text = message.clone();
    let age = event.last_seen.map_or_else(
        || "—".to_owned(),
        |seen| format!("{} ago", format_age(Some(seen), now)),
    );
    v_flex()
        .id(("object-event", index))
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(toned_text(event_tone(event.event_type), cx).flex_shrink_0())
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_semibold()
                        .child(event.reason.clone()),
                )
                .children((event.count > 1).then(|| {
                    div()
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child(format!("×{}", event.count))
                }))
                .child(
                    div()
                        .ml_auto()
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child(age),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .line_clamp(MESSAGE_LINES)
                .child(message),
        )
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .into_any_element()
}

#[cfg(test)]
#[path = "object_events_tests.rs"]
mod object_events_tests;
