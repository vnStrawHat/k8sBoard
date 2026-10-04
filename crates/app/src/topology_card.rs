//! The node cards of the Topology canvas (W11 `.nd`): kit-styled divs, one per visible node, at the
//! level of detail of the zoom. A card has a 3 px accent bar and a solid chip in the color of its
//! kind, the caption and the name, and a surface that carries a trace of the kind.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::{
    App, BoxShadow, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
    point, px,
};

use crate::status_tone::StatusTone;
use crate::topology_colors::{CanvasColors, KIND_BOX_TINT, KindHue, SelectionGlow, kind_hue};
use crate::topology_graph::{NodeLook, TopologyNode};
use crate::topology_layout::{GraphRect, NODE_HEIGHT, Placement};
use crate::topology_viewport::{MIN_TEXT_ZOOM, Viewport, snap};

/// Below `MIN_TEXT_ZOOM` a card shows its badge only, and below this a plain box.
pub(crate) const MIN_BADGE_ZOOM: f32 = 0.3;
/// The card, in graph units: the accent bar on its left edge, the solid kind chip, the padding,
/// and the font sizes (React Flow nodes use about 12 px text with 10 px padding).
pub(crate) const ACCENT_BAR: f32 = 3.;
pub(crate) const CHIP_SIZE: f32 = 30.;
/// The icon in the chip, as a share of the chip.
const ICON_SHARE: f32 = 0.55;
pub(crate) const CARD_PADDING: f32 = 10.;
pub(crate) const NAME_SIZE: f32 = 13.;
pub(crate) const CAPTION_SIZE: f32 = 11.;
pub(crate) const CHIP_TEXT_SIZE: f32 = 11.;

/// What the card of a node shows besides the node itself.
pub(crate) struct CardState {
    pub(crate) is_selected: bool,
    /// The pointer is over the card.
    pub(crate) is_hovered: bool,
    /// A ghost or unchecked node that was clicked: highlighted, nothing opens.
    pub(crate) is_highlighted: bool,
    /// What the tooltip says: the check text of a ghost, else the full name.
    pub(crate) tooltip: SharedString,
    /// The caption line: the node's own, or its traffic text in Traffic mode (spec 0049).
    pub(crate) caption: SharedString,
}

/// What every card of a frame shares.
pub(crate) struct CardFrame {
    pub(crate) viewport: Viewport,
    pub(crate) scale_factor: f32,
    pub(crate) colors: CanvasColors,
}

/// The card of one node, at its screen position on device pixels.
pub(crate) fn node_card(
    index: usize,
    node: &TopologyNode,
    rect: GraphRect,
    frame: &CardFrame,
    state: &CardState,
    cx: &App,
) -> Stateful<Div> {
    let colors = &frame.colors;
    let zoom = frame.viewport.zoom();
    let hue = kind_hue(node.kind);
    let (x, y) = frame.viewport.to_screen(rect.origin);
    let snapped = |value: f32| snap(value, frame.scale_factor);
    let (left, top) = (snapped(x), snapped(y));
    let (right, bottom) = (
        snapped(x + rect.width * zoom),
        snapped(y + rect.height * zoom),
    );
    let (border_width, border_color) = card_border(node, state, colors);
    let tooltip = state.tooltip.clone();
    let mut card = div()
        .id(("topology-node", index))
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(right - left))
        .h(px(bottom - top))
        .rounded(px(8. * zoom))
        .border(px(border_width))
        .border_color(border_color)
        .overflow_hidden()
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx));
    // Every card is opaque, so an edge never shows through its text; the dimmer looks fade only
    // what is drawn on it.
    card = card.bg(card_surface(node, state, colors));
    card = if state.is_selected {
        card.shadow(selection_glow(colors.kind(hue), colors.glow, zoom))
    } else if state.is_hovered {
        card.shadow_md()
    } else {
        card.shadow_xs()
    };
    if node.look != NodeLook::Plain {
        card = card.border_dashed();
    }
    let Some(detail) = card_detail(zoom) else {
        return card.child(div().size_full().bg(box_fill(colors, hue)));
    };
    let opacity = match node.look {
        NodeLook::Unchecked => 0.7,
        NodeLook::Plain if node.kind.placement() == Placement::ConfigRow => 0.85,
        NodeLook::Plain | NodeLook::Ghost => 1.,
    };
    card.child(card_body(node, &state.caption, zoom, detail, colors, cx).opacity(opacity))
}

/// The surface of a card: the highlight of a clicked ghost, the kind-tinted surface of an object,
/// or the plain card of a ghost or unchecked node.
fn card_surface(node: &TopologyNode, state: &CardState, colors: &CanvasColors) -> Hsla {
    if state.is_highlighted {
        return colors.muted;
    }
    match node.look {
        NodeLook::Plain => colors.card_fill(kind_hue(node.kind)),
        NodeLook::Ghost | NodeLook::Unchecked => colors.card,
    }
}

/// The soft ring around a selected card: the kind color, spread and blurred.
fn selection_glow(kind: Hsla, glow: SelectionGlow, zoom: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: kind.opacity(glow.alpha),
        offset: point(px(0.), px(0.)),
        blur_radius: px(glow.blur * zoom),
        spread_radius: px(glow.spread * zoom),
        inset: false,
    }]
}

/// The fill of a box card of a zoomed-out graph: the kind color, over the card.
fn box_fill(colors: &CanvasColors, hue: KindHue) -> Hsla {
    colors.kind(hue).opacity(KIND_BOX_TINT)
}

/// The level of detail of a zoom: the text from `MIN_TEXT_ZOOM`, the badge alone from
/// `MIN_BADGE_ZOOM`, and below that nothing but the box.
pub(crate) fn card_detail(zoom: f32) -> Option<CardDetail> {
    if zoom >= MIN_TEXT_ZOOM {
        Some(CardDetail::Text)
    } else if zoom >= MIN_BADGE_ZOOM {
        Some(CardDetail::BadgeOnly)
    } else {
        None
    }
}

/// How much of a card is drawn: the level of detail of its zoom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardDetail {
    Text,
    /// Below `MIN_TEXT_ZOOM`: only the kind badge, large enough to read.
    BadgeOnly,
}

/// The border width and color of a card: selection wins (in the kind color), then Bad, then Warn;
/// a hovered card without a tone takes the kind color too.
fn card_border(node: &TopologyNode, state: &CardState, colors: &CanvasColors) -> (f32, Hsla) {
    let kind = colors.kind(kind_hue(node.kind));
    if state.is_selected {
        return (2., kind);
    }
    match node.look {
        NodeLook::Ghost => (
            1.5,
            node.tone
                .map_or(colors.muted_foreground, |tone| colors.tone(tone)),
        ),
        NodeLook::Unchecked => (1., colors.muted_foreground),
        NodeLook::Plain => match node.tone {
            Some(StatusTone::Bad) => (2., colors.bad),
            Some(StatusTone::Warn) => (1., colors.warn),
            _ if state.is_hovered => (1., kind),
            _ => (1., colors.card_border),
        },
    }
}

fn card_body(
    node: &TopologyNode,
    caption: &SharedString,
    zoom: f32,
    detail: CardDetail,
    colors: &CanvasColors,
    cx: &App,
) -> Div {
    if detail == CardDetail::BadgeOnly {
        return badge_only_body(node, zoom, colors, cx);
    }
    let theme = cx.theme();
    let hue = kind_hue(node.kind);
    let has_chip = node.look == NodeLook::Plain;
    let caption_color = match node.tone {
        Some(tone @ (StatusTone::Bad | StatusTone::Warn)) => colors.tone(tone),
        _ => colors.muted_foreground,
    };
    let lines = v_flex()
        .flex_1()
        .min_w_0()
        .justify_center()
        .pl(px(if has_chip { 0. } else { 2. * CARD_PADDING } * zoom))
        .pr(px(CARD_PADDING * zoom))
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(CAPTION_SIZE * zoom))
                .text_color(caption_color)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(caption.to_uppercase()),
        )
        .child(
            div()
                .font_family(theme.font_family.clone())
                .text_size(px(NAME_SIZE * zoom))
                .text_color(theme.foreground)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(node.name.clone()),
        );
    // The accent bar on the left edge and the solid chip: the kind, unmistakably.
    let bar = has_chip.then(|| {
        div()
            .flex_shrink_0()
            .w(px(ACCENT_BAR * zoom))
            .h_full()
            .bg(colors.kind(hue))
    });
    let chip = has_chip.then(|| {
        div()
            .flex_shrink_0()
            .mx(px(CARD_PADDING * zoom))
            .size(px(CHIP_SIZE * zoom))
            .rounded(px(7. * zoom))
            .flex()
            .items_center()
            .justify_center()
            .bg(colors.kind(hue))
            .child(
                Icon::new(node.kind.icon())
                    .size(px(CHIP_SIZE * ICON_SHARE * zoom))
                    .text_color(colors.kind_text(hue)),
            )
    });
    h_flex()
        .size_full()
        .items_center()
        .children(bar)
        .children(chip)
        .child(lines)
}

/// The card of a zoomed-out graph: a solid chip with the kind icon fills the middle, in the tone of
/// the node when it is a problem.
fn badge_only_body(node: &TopologyNode, zoom: f32, colors: &CanvasColors, cx: &App) -> Div {
    let fill = match node.tone {
        Some(tone @ (StatusTone::Bad | StatusTone::Warn)) => colors.tone(tone),
        _ => colors.kind(kind_hue(node.kind)),
    };
    let text_on = colors.text_on(fill);
    // A ghost has no kind to show, so it keeps its question mark.
    let mark = if node.look == NodeLook::Plain {
        Icon::new(node.kind.icon())
            .size(px(NODE_HEIGHT * zoom * 0.4))
            .text_color(text_on)
            .into_any_element()
    } else {
        div()
            .font_family(cx.theme().mono_font_family.clone())
            .font_weight(FontWeight::BOLD)
            .text_size(px(NODE_HEIGHT * zoom * 0.34))
            .text_color(text_on)
            .child("?")
            .into_any_element()
    };
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .h(px(NODE_HEIGHT * zoom * 0.62))
                .px(px(NODE_HEIGHT * zoom * 0.22))
                .rounded(px(7. * zoom))
                .flex()
                .items_center()
                .bg(fill)
                .child(mark),
        )
}

#[cfg(test)]
#[path = "topology_card_tests.rs"]
mod topology_card_tests;
