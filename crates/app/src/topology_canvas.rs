//! The Topology canvas (W11): the viewport math, the edge curves, and the painting. One GPUI
//! `canvas` paints the dots, bands, edges, and arrows and registers the window mouse handlers;
//! the node cards are kit-styled divs that only the visible nodes get (decision 24).

use std::rc::Rc;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    App, BorderStyle, Bounds, DispatchPhase, Div, Hitbox, HitboxBehavior, Hsla,
    InteractiveElement as _, IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, PathBuilder, Point, ScrollWheelEvent, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, canvas, div, fill, point, px,
    quad, size, transparent_black,
};

use crate::status_tone::{StatusTone, tone_color};
use crate::topology_graph::{NodeLook, Relation, TopologyGraph, TopologyNode};
use crate::topology_layout::{
    GraphPoint, GraphRect, NODE_HEIGHT, NODE_WIDTH, Placement, TopologyLayout,
};
use crate::topology_view::TopologyView;
use crate::topology_viewport::{
    MIN_TEXT_ZOOM, MINIMAP_HEIGHT, MINIMAP_WIDTH, Viewport, minimap_transform,
};

const ARROW_SIZE: f32 = 8.;
/// Backward edges leave the nodes this far below the lower one.
const BACKWARD_DROP: f32 = 40.;
const DOT_SPACING: f32 = 16.;
/// Dots closer than this (in px) are skipped.
const MIN_DOT_SPACING: f32 = 8.;
/// Below `MIN_TEXT_ZOOM` a card shows its badge only, and below this a plain box.
const MIN_BADGE_ZOOM: f32 = 0.3;

/// A cubic Bezier in graph units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EdgeCurve {
    pub(crate) start: GraphPoint,
    pub(crate) ctrl1: GraphPoint,
    pub(crate) ctrl2: GraphPoint,
    pub(crate) end: GraphPoint,
}

/// Mounts run from the bottom of the source to the top of the config node; other edges run from the
/// right side of the source to the left side of the target; an edge that does not move right goes
/// under the nodes.
pub(crate) fn edge_curve(from: GraphRect, to: GraphRect, relation: Relation) -> EdgeCurve {
    let at = |x: f32, y: f32| GraphPoint { x, y };
    if relation == Relation::Mounts {
        let start = at(from.center().x, from.bottom());
        let end = at(to.center().x, to.origin.y);
        let half = (end.y - start.y) / 2.;
        return EdgeCurve {
            start,
            ctrl1: at(start.x, start.y + half),
            ctrl2: at(end.x, end.y - half),
            end,
        };
    }
    if to.origin.x > from.origin.x {
        let start = at(from.right(), from.center().y);
        let end = at(to.origin.x, to.center().y);
        let half = (end.x - start.x) / 2.;
        return EdgeCurve {
            start,
            ctrl1: at(start.x + half, start.y),
            ctrl2: at(end.x - half, end.y),
            end,
        };
    }
    let start = at(from.center().x, from.bottom());
    let end = at(to.center().x, to.bottom());
    let low = start.y.max(end.y) + BACKWARD_DROP;
    EdgeCurve {
        start,
        ctrl1: at(start.x, low),
        ctrl2: at(end.x, low),
        end,
    }
}

/// The triangle of an arrow at the end of `curve`: the tip on the target and two base corners
/// `size` behind it.
pub(crate) fn arrow_head(curve: &EdgeCurve, size: f32) -> [GraphPoint; 3] {
    let mut direction = (curve.end.x - curve.ctrl2.x, curve.end.y - curve.ctrl2.y);
    if direction == (0., 0.) {
        direction = (curve.end.x - curve.start.x, curve.end.y - curve.start.y);
    }
    let length = direction.0.hypot(direction.1).max(f32::EPSILON);
    let (dx, dy) = (direction.0 / length, direction.1 / length);
    let base = (curve.end.x - dx * size, curve.end.y - dy * size);
    let half = size / 2.;
    [
        curve.end,
        GraphPoint {
            x: base.0 - dy * half,
            y: base.1 + dx * half,
        },
        GraphPoint {
            x: base.0 + dy * half,
            y: base.1 - dx * half,
        },
    ]
}

/// How an edge relation is stroked: its width and dash (in graph units), as the screen and the
/// export both draw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stroke {
    pub(crate) width: f32,
    pub(crate) dash: Option<(f32, f32)>,
}

pub(crate) fn relation_stroke(relation: Relation) -> Stroke {
    match relation {
        Relation::Owns => Stroke {
            width: 1.4,
            dash: None,
        },
        Relation::RoutesTo => Stroke {
            width: 1.6,
            dash: Some((5., 4.)),
        },
        Relation::Mounts => Stroke {
            width: 1.2,
            dash: Some((2., 3.)),
        },
    }
}

/// The extra width of the edges of the selected node.
const SELECTED_EDGE_EXTRA: f32 = 0.6;

/// The theme colors the painting needs, resolved once per frame.
#[derive(Clone, Copy)]
pub(crate) struct CanvasColors {
    pub(crate) border: Hsla,
    pub(crate) muted_foreground: Hsla,
    pub(crate) ring: Hsla,
    pub(crate) warn: Hsla,
    pub(crate) bad: Hsla,
}

impl CanvasColors {
    pub(crate) fn of(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            border: theme.border,
            muted_foreground: theme.muted_foreground,
            ring: theme.ring,
            warn: tone_color(StatusTone::Warn, cx),
            bad: tone_color(StatusTone::Bad, cx),
        }
    }

    pub(crate) fn tone(&self, tone: StatusTone) -> Hsla {
        match tone {
            StatusTone::Bad => self.bad,
            StatusTone::Warn => self.warn,
            StatusTone::Ok | StatusTone::Info | StatusTone::Done => self.muted_foreground,
        }
    }

    fn edge(&self, relation: Relation) -> Hsla {
        match relation {
            Relation::RoutesTo => self.ring,
            Relation::Owns | Relation::Mounts => self.muted_foreground,
        }
    }
}

/// What the canvas layer paints and which handlers it registers.
pub(crate) struct CanvasPaint {
    pub(crate) graph: Rc<TopologyGraph>,
    pub(crate) layout: Rc<TopologyLayout>,
    pub(crate) viewport: Viewport,
    /// The node whose edges are drawn in the accent color.
    pub(crate) selected: Option<usize>,
    pub(crate) colors: CanvasColors,
    pub(crate) view: WeakEntity<TopologyView>,
    /// A drag runs, so the move and up handlers are registered this frame.
    pub(crate) is_dragging: bool,
}

/// The paint layer under the cards. It registers the wheel handler always and the move and up
/// handlers while a drag runs, and tells the view its size, which Fit and focus need.
pub(crate) fn graph_canvas(paint: CanvasPaint) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            paint_graph(&paint, bounds, window);
            register_handlers(&paint, bounds, hitbox, window, cx);
        },
    )
    .absolute()
    .size_full()
}

fn register_handlers(
    paint: &CanvasPaint,
    bounds: Bounds<gpui_kit::Pixels>,
    hitbox: Hitbox,
    window: &mut Window,
    cx: &mut App,
) {
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let _ = paint
        .view
        .update(cx, |view, cx| view.set_canvas_size(width, height, cx));
    let view = paint.view.clone();
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        // Over the drawer the wheel scrolls the drawer: its hitbox hides this one.
        if phase != DispatchPhase::Bubble || !hitbox.should_handle_scroll(window) {
            return;
        }
        let local = event.position - bounds.origin;
        let _ = view.update(cx, |view, cx| {
            view.zoom_by_wheel(event.delta, f32::from(local.x), f32::from(local.y), cx);
        });
        cx.stop_propagation();
    });
    if !paint.is_dragging {
        return;
    }
    let view = paint.view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            let _ = view.update(cx, |view, cx| view.drag_to(event.position, cx));
        }
    });
    let view = paint.view.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            let _ = view.update(cx, |view, cx| {
                view.finish_drag(event.position, event.click_count, cx);
            });
        }
    });
}

fn screen_point(
    viewport: Viewport,
    bounds: Bounds<gpui_kit::Pixels>,
    at: GraphPoint,
) -> Point<gpui_kit::Pixels> {
    let (x, y) = viewport.to_screen(at);
    point(bounds.origin.x + px(x), bounds.origin.y + px(y))
}

/// Dots, bands, edges, then arrows, back to front.
fn paint_graph(paint: &CanvasPaint, bounds: Bounds<gpui_kit::Pixels>, window: &mut Window) {
    paint_dots(paint, bounds, window);
    paint_bands(paint, bounds, window);
    paint_edges(paint, bounds, window);
}

fn paint_dots(paint: &CanvasPaint, bounds: Bounds<gpui_kit::Pixels>, window: &mut Window) {
    let viewport = paint.viewport;
    let spacing = DOT_SPACING * viewport.zoom();
    if spacing < MIN_DOT_SPACING {
        return;
    }
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let first = viewport.to_graph(0., 0.);
    let start_x = (first.x / DOT_SPACING).ceil() * DOT_SPACING;
    let start_y = (first.y / DOT_SPACING).ceil() * DOT_SPACING;
    let dot = size(px(1.), px(1.));
    let mut y = start_y;
    loop {
        let (_, screen_y) = viewport.to_screen(GraphPoint { x: 0., y });
        if screen_y > height {
            break;
        }
        let mut x = start_x;
        loop {
            let (screen_x, _) = viewport.to_screen(GraphPoint { x, y: 0. });
            if screen_x > width {
                break;
            }
            let origin = point(
                bounds.origin.x + px(screen_x),
                bounds.origin.y + px(screen_y),
            );
            window.paint_quad(fill(Bounds { origin, size: dot }, paint.colors.border));
            x += DOT_SPACING;
        }
        y += DOT_SPACING;
    }
}

fn paint_bands(paint: &CanvasPaint, bounds: Bounds<gpui_kit::Pixels>, window: &mut Window) {
    let zoom = paint.viewport.zoom();
    for band in paint
        .layout
        .bands
        .iter()
        .filter(|band| band.title.is_some())
    {
        let origin = screen_point(paint.viewport, bounds, band.rect.origin);
        let band_size = size(px(band.rect.width * zoom), px(band.rect.height * zoom));
        window.paint_quad(quad(
            Bounds {
                origin,
                size: band_size,
            },
            px(10. * zoom),
            transparent_black(),
            px(1.),
            paint.colors.border,
            BorderStyle::Dashed,
        ));
    }
}

fn paint_edges(paint: &CanvasPaint, bounds: Bounds<gpui_kit::Pixels>, window: &mut Window) {
    let viewport = paint.viewport;
    let zoom = viewport.zoom();
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    for edge in &paint.graph.edges {
        let (from, to) = (paint.layout.rects[edge.from], paint.layout.rects[edge.to]);
        let curve = edge_curve(from, to, edge.relation);
        if !touches(viewport, curve_bounds(&curve), width, height) {
            continue;
        }
        let is_selected = paint
            .selected
            .is_some_and(|node| node == edge.from || node == edge.to);
        let stroke = relation_stroke(edge.relation);
        let line_width = stroke.width + if is_selected { SELECTED_EDGE_EXTRA } else { 0. };
        let color = edge_color(paint, edge.to, edge.relation, is_selected);
        let at = |p: GraphPoint| screen_point(viewport, bounds, p);
        let mut builder = PathBuilder::stroke(px(line_width * zoom.max(0.5)));
        if let Some((on, off)) = stroke.dash {
            builder = builder.dash_array(&[px(on * zoom), px(off * zoom)]);
        }
        builder.move_to(at(curve.start));
        builder.cubic_bezier_to(at(curve.end), at(curve.ctrl1), at(curve.ctrl2));
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
        let [tip, left, right] = arrow_head(&curve, ARROW_SIZE);
        let mut arrow = PathBuilder::fill();
        arrow.move_to(at(tip));
        arrow.line_to(at(left));
        arrow.line_to(at(right));
        arrow.close();
        if let Ok(path) = arrow.build() {
            window.paint_path(path, color);
        }
    }
}

/// An edge into a ghost takes the tone of its check; the edges of the selected node are accented.
fn edge_color(paint: &CanvasPaint, target: usize, relation: Relation, is_selected: bool) -> Hsla {
    let node = &paint.graph.nodes[target];
    let ghost_tone = (node.look == NodeLook::Ghost)
        .then(|| {
            paint
                .graph
                .checks
                .iter()
                .find(|check| check.node == node.id)
                .map(|check| check.tone)
        })
        .flatten();
    match ghost_tone {
        Some(tone) => paint.colors.tone(tone),
        None if is_selected => paint.colors.ring,
        None => paint.colors.edge(relation),
    }
}

/// The box of the four points of a curve, which holds the whole curve: `(left, top, right, bottom)`.
fn curve_bounds(curve: &EdgeCurve) -> (f32, f32, f32, f32) {
    let points = [curve.start, curve.ctrl1, curve.ctrl2, curve.end];
    let low =
        |value: fn(&GraphPoint) -> f32| points.iter().map(value).fold(f32::INFINITY, f32::min);
    let high =
        |value: fn(&GraphPoint) -> f32| points.iter().map(value).fold(f32::NEG_INFINITY, f32::max);
    (low(|p| p.x), low(|p| p.y), high(|p| p.x), high(|p| p.y))
}

/// Whether a graph-space box touches a canvas of `width` by `height` at this viewport. The edges
/// are culled by the box of their curve: a long edge crosses the canvas without an end in it.
fn touches(viewport: Viewport, bounds: (f32, f32, f32, f32), width: f32, height: f32) -> bool {
    let (left, top) = viewport.to_screen(GraphPoint {
        x: bounds.0,
        y: bounds.1,
    });
    let (right, bottom) = viewport.to_screen(GraphPoint {
        x: bounds.2,
        y: bounds.3,
    });
    right >= 0. && bottom >= 0. && left <= width && top <= height
}

/// What the card of a node shows besides the node itself.
pub(crate) struct CardState {
    pub(crate) is_selected: bool,
    /// A ghost or unchecked node that was clicked: highlighted, nothing opens.
    pub(crate) is_highlighted: bool,
    /// The check text of a ghost, for its tooltip.
    pub(crate) tooltip: Option<SharedString>,
}

/// The card of one node (W11 `.nd`), at its screen position.
pub(crate) fn node_card(
    index: usize,
    node: &TopologyNode,
    rect: GraphRect,
    viewport: Viewport,
    state: &CardState,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let zoom = viewport.zoom();
    let (x, y) = viewport.to_screen(rect.origin);
    let (border_width, border_color) = card_border(node, state, cx);
    let mut card = div()
        .id(("topology-node", index))
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(NODE_WIDTH * zoom))
        .h(px(NODE_HEIGHT * zoom))
        .rounded(px(8. * zoom))
        .border(px(border_width))
        .border_color(border_color)
        .overflow_hidden()
        .cursor_pointer();
    // Every card is opaque, so an edge never shows through its text; the dimmer looks fade only
    // what is drawn on it.
    card = card.bg(if state.is_highlighted {
        theme.muted
    } else {
        theme.background
    });
    if node.look != NodeLook::Plain {
        card = card.border_dashed();
    }
    if let Some(tooltip) = state.tooltip.clone() {
        card = card.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx));
    }
    let Some(detail) = card_detail(zoom) else {
        return card;
    };
    let opacity = match node.look {
        NodeLook::Unchecked => 0.7,
        NodeLook::Plain if node.kind.placement() == Placement::ConfigRow => 0.85,
        NodeLook::Plain | NodeLook::Ghost => 1.,
    };
    card.child(card_body(node, zoom, detail, cx).opacity(opacity))
}

/// The level of detail of a zoom: the text from `MIN_TEXT_ZOOM`, the badge alone from
/// `MIN_BADGE_ZOOM`, and below that nothing but the box.
fn card_detail(zoom: f32) -> Option<CardDetail> {
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
enum CardDetail {
    Text,
    /// Below `MIN_TEXT_ZOOM`: only the kind badge, large enough to read.
    BadgeOnly,
}

/// The border width and color of a card: selection wins, then Bad, then Warn.
fn card_border(node: &TopologyNode, state: &CardState, cx: &App) -> (f32, Hsla) {
    let theme = cx.theme();
    if state.is_selected {
        return (2., theme.ring);
    }
    match node.look {
        NodeLook::Ghost => (
            1.5,
            node.tone
                .map_or(theme.muted_foreground, |tone| tone_color(tone, cx)),
        ),
        NodeLook::Unchecked => (1., theme.muted_foreground),
        NodeLook::Plain => match node.tone {
            Some(StatusTone::Bad) => (2., tone_color(StatusTone::Bad, cx)),
            Some(StatusTone::Warn) => (1., tone_color(StatusTone::Warn, cx)),
            _ => (1., theme.border),
        },
    }
}

fn card_body(node: &TopologyNode, zoom: f32, detail: CardDetail, cx: &App) -> Div {
    let theme = cx.theme();
    let mono = theme.mono_font_family.clone();
    if detail == CardDetail::BadgeOnly {
        return badge_only_body(node, zoom, cx);
    }
    let has_badge = node.look == NodeLook::Plain;
    let caption_color = match node.tone {
        Some(tone @ (StatusTone::Bad | StatusTone::Warn)) => tone_color(tone, cx),
        _ => theme.muted_foreground,
    };
    let lines = v_flex()
        .flex_1()
        .min_w_0()
        .justify_center()
        .px(px(8. * zoom))
        .font_family(mono.clone())
        .child(
            div()
                .text_size(px(9. * zoom))
                .text_color(caption_color)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(node.caption.to_uppercase()),
        )
        .child(
            div()
                .text_size(px(10.8 * zoom))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(node.name.clone()),
        );
    let badge = has_badge.then(|| {
        div()
            .flex_shrink_0()
            .w(px(24. * zoom))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.muted)
            .font_family(mono)
            .text_size(px(9. * zoom))
            .text_color(theme.muted_foreground)
            .child(node.kind.badge())
    });
    h_flex().size_full().children(badge).child(lines)
}

/// The card of a zoomed-out graph: the badge alone fills it, in the tone of the node.
fn badge_only_body(node: &TopologyNode, zoom: f32, cx: &App) -> Div {
    let theme = cx.theme();
    let color = match node.tone {
        Some(tone @ (StatusTone::Bad | StatusTone::Warn)) => tone_color(tone, cx),
        _ => theme.muted_foreground,
    };
    let label = if node.look == NodeLook::Plain {
        node.kind.badge()
    } else {
        "?"
    };
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_size(px(NODE_HEIGHT * zoom * 0.45))
        .text_color(color)
        .child(label)
}

/// The minimap (W11 `.mini`): every node as a small rect and the viewport as an outline.
pub(crate) struct MinimapPaint {
    pub(crate) graph: Rc<TopologyGraph>,
    pub(crate) layout: Rc<TopologyLayout>,
    pub(crate) viewport: Viewport,
    /// The size of the main canvas, so the viewport outline matches what it shows.
    pub(crate) canvas: (f32, f32),
    pub(crate) colors: CanvasColors,
    pub(crate) view: WeakEntity<TopologyView>,
    pub(crate) is_dragging: bool,
}

pub(crate) fn minimap_canvas(paint: MinimapPaint) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            paint_minimap(&paint, bounds, window);
            register_minimap_handlers(&paint, bounds, hitbox, window, cx);
        },
    )
    .size_full()
}

fn paint_minimap(paint: &MinimapPaint, bounds: Bounds<gpui_kit::Pixels>, window: &mut Window) {
    let transform = minimap_transform(paint.layout.extent, MINIMAP_WIDTH, MINIMAP_HEIGHT);
    let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    for (index, node) in paint.graph.nodes.iter().enumerate() {
        let rect = paint.layout.rects[index];
        let (x, y) = transform.to_minimap(rect.origin);
        let node_size = size(
            px((rect.width * transform.scale()).max(2.)),
            px((rect.height * transform.scale()).max(2.)),
        );
        let color = match node.tone {
            Some(tone @ (StatusTone::Bad | StatusTone::Warn)) => paint.colors.tone(tone),
            _ => paint.colors.muted_foreground,
        };
        window.paint_quad(fill(
            Bounds {
                origin: at(x, y),
                size: node_size,
            },
            color,
        ));
    }
    let zoom = paint.viewport.zoom();
    let (x, y) = transform.to_minimap(paint.viewport.origin);
    let view_size = size(
        px(paint.canvas.0 / zoom * transform.scale()),
        px(paint.canvas.1 / zoom * transform.scale()),
    );
    window.paint_quad(quad(
        Bounds {
            origin: at(x, y),
            size: view_size,
        },
        px(0.),
        transparent_black(),
        px(1.5),
        paint.colors.ring,
        BorderStyle::Solid,
    ));
}

fn register_minimap_handlers(
    paint: &MinimapPaint,
    bounds: Bounds<gpui_kit::Pixels>,
    hitbox: Hitbox,
    window: &mut Window,
    _cx: &mut App,
) {
    let transform = minimap_transform(paint.layout.extent, MINIMAP_WIDTH, MINIMAP_HEIGHT);
    let view = paint.view.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
            return;
        }
        let local = event.position - bounds.origin;
        let _ = view.update(cx, |view, cx| {
            view.minimap_press(
                transform.to_graph(f32::from(local.x), f32::from(local.y)),
                cx,
            );
        });
        cx.stop_propagation();
    });
    if !paint.is_dragging {
        return;
    }
    let view = paint.view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let local = event.position - bounds.origin;
        let _ = view.update(cx, |view, cx| {
            view.minimap_drag(
                transform.to_graph(f32::from(local.x), f32::from(local.y)),
                cx,
            );
        });
    });
    let view = paint.view.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            let _ = view.update(cx, |view, cx| {
                view.finish_drag(event.position, event.click_count, cx);
            });
        }
    });
}

/// The kinds of the legend, and what each stroke means.
pub(crate) const LEGEND: [(Relation, &str, &str); 3] = [
    (Relation::Owns, "\u{2500}\u{2500}", "owns"),
    (Relation::RoutesTo, "\u{254c}\u{254c}", "routes to"),
    (Relation::Mounts, "\u{2508}\u{2508}", "mounts"),
];

/// The legend glyph color of a relation, as the edges are drawn.
pub(crate) fn legend_color(relation: Relation, colors: &CanvasColors) -> Hsla {
    colors.edge(relation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32) -> GraphRect {
        GraphRect {
            origin: GraphPoint { x, y },
            width: NODE_WIDTH,
            height: NODE_HEIGHT,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn forward_edge_runs_right_to_left_side() {
        let (from, to) = (rect(24., 24.), rect(234., 94.));
        let curve = edge_curve(from, to, Relation::Owns);
        assert_eq!(
            curve.start,
            GraphPoint {
                x: from.right(),
                y: from.center().y
            }
        );
        assert_eq!(
            curve.end,
            GraphPoint {
                x: to.origin.x,
                y: to.center().y
            }
        );
        assert!(close(curve.ctrl1.y, curve.start.y) && close(curve.ctrl2.y, curve.end.y));
    }

    #[test]
    fn mounts_edge_runs_bottom_to_top() {
        let (from, to) = (rect(24., 24.), rect(234., 200.));
        let curve = edge_curve(from, to, Relation::Mounts);
        assert_eq!(
            curve.start,
            GraphPoint {
                x: from.center().x,
                y: from.bottom()
            }
        );
        assert_eq!(
            curve.end,
            GraphPoint {
                x: to.center().x,
                y: to.origin.y
            }
        );
        assert!(close(curve.ctrl1.x, curve.start.x) && close(curve.ctrl2.x, curve.end.x));
    }

    #[test]
    fn backward_edge_runs_under_nodes() {
        let (from, to) = (rect(444., 24.), rect(234., 94.));
        let curve = edge_curve(from, to, Relation::RoutesTo);
        let lowest = from.bottom().max(to.bottom());
        assert!(close(curve.ctrl1.y, lowest + BACKWARD_DROP));
        assert!(close(curve.ctrl2.y, lowest + BACKWARD_DROP));
        assert_eq!(curve.start.y, from.bottom());
    }

    #[test]
    fn arrow_head_points_at_target() {
        let curve = edge_curve(rect(24., 24.), rect(234., 94.), Relation::Owns);
        let [tip, left, right] = arrow_head(&curve, ARROW_SIZE);
        assert_eq!(tip, curve.end);
        // The base lies on the source side of the tip, and the corners are symmetric.
        assert!(left.x < tip.x && right.x < tip.x);
        assert!(close(left.y + right.y, 2. * curve.end.y));
    }

    #[test]
    fn the_level_of_detail_steps_down_with_the_zoom() {
        assert_eq!(card_detail(1.), Some(CardDetail::Text));
        assert_eq!(card_detail(MIN_TEXT_ZOOM), Some(CardDetail::Text));
        assert_eq!(
            card_detail(MIN_TEXT_ZOOM - 0.01),
            Some(CardDetail::BadgeOnly)
        );
        assert_eq!(card_detail(MIN_BADGE_ZOOM), Some(CardDetail::BadgeOnly));
        assert_eq!(card_detail(MIN_BADGE_ZOOM - 0.01), None);
    }

    #[test]
    fn a_curve_is_inside_the_box_of_its_four_points() {
        let curve = edge_curve(rect(444., 24.), rect(234., 94.), Relation::RoutesTo);
        let (left, top, right, bottom) = curve_bounds(&curve);
        for point in [curve.start, curve.ctrl1, curve.ctrl2, curve.end] {
            assert!(point.x >= left && point.x <= right);
            assert!(point.y >= top && point.y <= bottom);
        }
    }

    #[test]
    fn relation_strokes_follow_the_legend() {
        assert_eq!(relation_stroke(Relation::Owns).dash, None);
        assert_eq!(relation_stroke(Relation::RoutesTo).dash, Some((5., 4.)));
        assert_eq!(relation_stroke(Relation::Mounts).dash, Some((2., 3.)));
    }
}
