//! Cutting a table cell's text in the middle, so the part that tells rows apart stays visible:
//! the end of an object name (`…-7mhtv`), not the shared start (`monitoring/vmselect-cachedir-…`).

use std::borrow::Cow;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::table::Column;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, HighlightStyle, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText, div, font, px,
};

use crate::drawer::truncated_text_with_tooltip;

/// Space a cell keeps free around its text: the kit's cell padding, plus one glyph of slack so
/// the estimate below never cuts a name that would have fitted.
const CELL_PADDING: Pixels = px(24.);

pub(crate) const ELLIPSIS: char = '…';

/// How many characters of the mono font fit in a cell of table column `column`, at the theme font
/// size that table cells inherit.
pub(crate) fn mono_capacity(column: Option<&Column>, cx: &App) -> usize {
    let Some(column) = column else {
        return usize::MAX;
    };
    let text_system = cx.text_system();
    let font_id = text_system.resolve_font(&font(cx.theme().mono_font_family.clone()));
    let font_size = cx.theme().font_size;
    let Ok(advance) = text_system.ch_advance(font_id, font_size) else {
        return usize::MAX;
    };
    if advance <= Pixels::ZERO {
        return usize::MAX;
    }
    let room = (column.width - CELL_PADDING).max(Pixels::ZERO);
    (f32::from(room) / f32::from(advance)) as usize
}

/// `text` cut to `max_chars` characters with an ellipsis in the middle. The tail keeps twice the
/// room of the head: the end of a generated name is what differs between its siblings.
pub(crate) fn middle_truncate(text: &str, max_chars: usize) -> Cow<'_, str> {
    cut_middle(text, max_chars, 0)
}

/// `middle_truncate` that keeps at least `min_head` characters of the start, up to half the room:
/// the namespace of `namespace/name` tells rows apart too.
fn cut_middle(text: &str, max_chars: usize, min_head: usize) -> Cow<'_, str> {
    let length = text.chars().count();
    if length <= max_chars {
        return Cow::Borrowed(text);
    }
    let kept = max_chars.saturating_sub(1);
    let head = (kept / 3).max(min_head.min(kept / 2));
    let tail = kept - head;
    let head_end = text
        .char_indices()
        .nth(head)
        .map_or(text.len(), |(at, _)| at);
    let tail_start = text
        .char_indices()
        .nth(length - tail)
        .map_or(text.len(), |(at, _)| at);
    Cow::Owned(format!(
        "{}{ELLIPSIS}{}",
        &text[..head_end],
        &text[tail_start..]
    ))
}

/// Mono text with a muted `{prefix}/`. Both share one text run so a long value is cut instead of
/// wrapping: in the middle once it passes `capacity` characters, so the end of the name stays
/// visible, and with an ellipsis if the estimate was short. The tooltip shows the whole text.
pub(crate) fn qualified_text(
    id: (&'static str, usize),
    prefix: Option<&str>,
    text: &str,
    capacity: usize,
    cx: &App,
) -> AnyElement {
    let mono = cx.theme().mono_font_family.clone();
    let Some(prefix) = prefix else {
        return truncated_text_with_tooltip(
            id,
            middle_truncate(text, capacity).into_owned(),
            text.to_owned(),
        )
        .w_full()
        .font_family(mono)
        .into_any_element();
    };
    let prefix = format!("{prefix}/");
    let full = format!("{prefix}{text}");
    let shown = cut_middle(&full, capacity, prefix.chars().count()).into_owned();
    // A cut inside the prefix leaves a shorter muted run.
    let muted_end = shown
        .find(ELLIPSIS)
        .map_or(prefix.len(), |at| at.min(prefix.len()));
    let muted = HighlightStyle {
        color: Some(cx.theme().muted_foreground),
        ..Default::default()
    };
    let highlights = vec![(0..muted_end, muted)];
    let tooltip_text = SharedString::from(full);
    div()
        .id(id)
        .w_full()
        .truncate()
        .font_family(mono)
        .child(StyledText::new(shown).with_highlights(highlights))
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_that_fits_is_kept() {
        assert_eq!(middle_truncate("kong-kong-61bh7", 15), "kong-kong-61bh7");
        assert!(matches!(middle_truncate("abc", 10), Cow::Borrowed(_)));
    }

    #[test]
    fn long_text_keeps_its_start_and_its_end() {
        let cut = middle_truncate("monitoring/vmselect-cachedir-vmselect-vm-victoria-0", 21);
        assert_eq!(cut, "monito…-vm-victoria-0");
        assert_eq!(cut.chars().count(), 21);
    }

    #[test]
    fn siblings_stay_distinguishable() {
        let first = middle_truncate("monitoring-v2/ingress/grafana-alpha", 20);
        let second = middle_truncate("monitoring-v2/ingress/grafana-bravo", 20);
        assert_ne!(first, second);
    }

    #[test]
    fn a_namespace_prefix_keeps_its_room() {
        let cut = cut_middle(
            "argocd/argocd-notifications-controller-66df6cdfb8-9k298",
            28,
            7,
        );
        assert!(cut.starts_with("argocd/"));
        assert!(cut.ends_with("9k298"));
        assert_eq!(cut.chars().count(), 28);
    }

    #[test]
    fn cutting_never_splits_a_character() {
        let cut = middle_truncate("ñandú-ñandú-ñandú-ñandú", 9);
        assert_eq!(cut.chars().count(), 9);
        assert!(cut.contains(ELLIPSIS));
    }

    #[test]
    fn a_tiny_capacity_leaves_only_the_ellipsis() {
        assert_eq!(middle_truncate("kong-kong", 1), "…");
        assert_eq!(middle_truncate("kong-kong", 0), "…");
    }
}
