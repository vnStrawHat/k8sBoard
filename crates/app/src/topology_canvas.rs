//! The Topology canvas (W11): the edge strokes and the painting. One GPUI `canvas` paints the dots,
//! bands, edges, and arrows and registers the window mouse handlers; the node cards are in
//! `topology_card.rs`, and a second canvas over them paints the handle dots.

use std::rc::Rc;
use std::time::Duration;

use cluster::TrafficSourceKind;
use gpui_kit::{
    App, BorderStyle, Bounds, DispatchPhase, Hitbox, HitboxBehavior, Hsla, IntoElement,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Point, ScrollWheelEvent, Styled as _, WeakEntity,
    Window, canvas, fill, point, px, quad, size, transparent_black,
};

use crate::status_tone::StatusTone;
use crate::topology_card::{CardDetail, MIN_BADGE_ZOOM, card_detail};
use crate::topology_colors::{CanvasColors, EDGE_REST_ALPHA, edge_color, kind_hue};
use crate::topology_graph::{Relation, TopologyEdge, TopologyGraph};
use crate::topology_layout::{GraphPoint, GraphRect, TopologyLayout};
use crate::topology_route::EdgeRoute;
use crate::topology_stroke::{Dash, feather, fill_convex, stroke_dashed, stroke_line, trim_end};
use crate::topology_traffic::{EdgeTraffic, TrafficLayer};
use crate::topology_view::TopologyView;
use crate::topology_viewport::{MIN_TEXT_ZOOM, Viewport, minimap_transform, snap};

/// The arrow of an edge: its length along the edge and half its base, in graph units, and how far
/// its tip stops short of the handle on the card.
pub(crate) const ARROW_LENGTH: f32 = 12.;
pub(crate) const ARROW_HALF_WIDTH: f32 = 5.5;
pub(crate) const ARROW_TIP_GAP: f32 = 1.;
/// An edge is never thinner than this on screen (logical px): with analytic coverage a 0.75 px
/// line reads as a lighter line, not a broken one.
const MIN_EDGE_WIDTH: f32 = 0.75;
/// The dot grid: the gap in graph units (React Flow `gap`) and the dot's fixed screen size.
const DOT_SPACING: f32 = 20.;
const DOT_SIZE: f32 = 1.5;
/// Dots closer than this (in px) are skipped.
const MIN_DOT_SPACING: f32 = 12.;
/// The extra width of the edges of the focused node.
const FOCUS_EXTRA_WIDTH: f32 = 1.;
/// The alpha of an edge that does not touch the focused node.
const EDGE_DIM_ALPHA: f32 = 0.1;
/// The handle dots where edges attach, in graph units (React Flow: 6 px), and their floor on screen.
pub(crate) const HANDLE_SIZE: f32 = 6.;
const MIN_HANDLE_DIAMETER: f32 = 6.;
/// The animated flow moves 20 graph units a second (React Flow's `dashdraw`). An edge keeps its own
/// dash; a solid one flows in long dashes, which stay apart from the routes and mounts dashes.
const FLOW_SPEED: f32 = 20.;
const OWNS_FLOW_DASH: (f32, f32) = (16., 4.);
/// The band title pill, in graph units: its offset into the band, its height, and its padding.
pub(crate) const TITLE_PILL_LEFT: f32 = 12.;
pub(crate) const TITLE_PILL_TOP: f32 = 7.;
pub(crate) const TITLE_PILL_HEIGHT: f32 = 20.;
pub(crate) const TITLE_PILL_PADDING: f32 = 9.;
pub(crate) const TITLE_SIZE: f32 = 11.5;
/// The corner radius of a minimap node.
const MINIMAP_NODE_RADIUS: f32 = 1.;
/// The share of the background over the part of the minimap outside the viewport.
const MINIMAP_MASK_ALPHA: f32 = 0.55;
/// The legend swatch: a short edge with its arrow, drawn by the stroke code of the edges.
pub(crate) const SWATCH_WIDTH: f32 = 38.;
pub(crate) const SWATCH_HEIGHT: f32 = 14.;

/// The triangle of an arrow at the end of `route`: the tip `ARROW_TIP_GAP` before the end, and two
/// base corners `length` behind the tip, `half_width` to each side.
pub(crate) fn arrow_head(route: &EdgeRoute, length: f32, half_width: f32) -> [GraphPoint; 3] {
    let (dx, dy) = route.end_direction();
    let end = route.end();
    let tip = GraphPoint {
        x: end.x - dx * ARROW_TIP_GAP,
        y: end.y - dy * ARROW_TIP_GAP,
    };
    let base = (tip.x - dx * length, tip.y - dy * length);
    [
        tip,
        GraphPoint {
            x: base.0 - dy * half_width,
            y: base.1 + dx * half_width,
        },
        GraphPoint {
            x: base.0 + dy * half_width,
            y: base.1 - dx * half_width,
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
            width: 1.5,
            dash: None,
        },
        Relation::RoutesTo => Stroke {
            width: 1.5,
            dash: Some((7., 4.)),
        },
        Relation::Mounts => Stroke {
            width: 1.5,
            dash: Some((4., 3.)),
        },
        Relation::Access => Stroke {
            width: 1.5,
            dash: Some((2., 3.)),
        },
        Relation::Calls => Stroke {
            width: 1.5,
            dash: None,
        },
    }
}

/// The idle edge of Traffic mode: thin, muted, and dotted (spec 0049).
const IDLE_WIDTH: f32 = 0.75;
const IDLE_DASH: (f32, f32) = (2., 4.);
/// An `Owns` flow is the quietest: it only shows that bytes pass.
const OWNS_FLOW_ALPHA: f32 = 0.6;

/// How an edge is stroked in Traffic mode, as the screen and the export both draw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TrafficLook {
    /// In graph units.
    pub(crate) width: f32,
    pub(crate) dash: Option<(f32, f32)>,
    /// The 5xx tone of a flow, which colors the stroke and the arrow.
    pub(crate) tone: Option<StatusTone>,
    /// An idle edge takes the muted text color.
    pub(crate) is_muted: bool,
    pub(crate) alpha: f32,
}

/// `None` for a hidden edge. A flow is solid whatever its relation's dash (decision 13): width
/// must read as rate.
pub(crate) fn traffic_look(traffic: &EdgeTraffic, relation: Relation) -> Option<TrafficLook> {
    match traffic {
        EdgeTraffic::Hidden => None,
        EdgeTraffic::Idle => Some(TrafficLook {
            width: IDLE_WIDTH,
            dash: Some(IDLE_DASH),
            tone: None,
            is_muted: true,
            alpha: EDGE_REST_ALPHA,
        }),
        EdgeTraffic::Flow(flow) => Some(TrafficLook {
            width: flow.width,
            dash: None,
            tone: flow.tone,
            is_muted: false,
            alpha: match (flow.tone, relation) {
                (Some(_), _) => 1.,
                (None, Relation::Owns) => OWNS_FLOW_ALPHA,
                (
                    None,
                    Relation::RoutesTo | Relation::Mounts | Relation::Access | Relation::Calls,
                ) => EDGE_REST_ALPHA,
            },
        }),
    }
}

/// The color of an edge in Traffic mode, with its alpha.
fn traffic_color(colors: &CanvasColors, look: &TrafficLook, relation: Relation) -> Hsla {
    let base = match (look.is_muted, look.tone) {
        (true, _) => colors.muted_foreground,
        (false, Some(tone)) => colors.tone(tone),
        (false, None) => colors.relation(relation),
    };
    base.opacity(look.alpha)
}

/// The dash of an edge that flows: its own, or long dashes for a solid edge.
fn flow_dash(relation: Relation) -> (f32, f32) {
    relation_stroke(relation).dash.unwrap_or(OWNS_FLOW_DASH)
}

/// The dash of an edge that flows in Traffic mode: an idle edge keeps its dots, which then
/// march; a solid flow breaks into the dashes of its relation (its width and color stay).
fn traffic_flow_dash(look: &TrafficLook, relation: Relation) -> (f32, f32) {
    look.dash.unwrap_or_else(|| flow_dash(relation))
}

/// How an edge stands out from the focused node (the hovered one, else the selected one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Emphasis {
    /// No focus: every edge at its rest look.
    Rest,
    /// The edge touches the focused node.
    Focused,
    Dimmed,
}

pub(crate) fn edge_emphasis(edge: &TopologyEdge, focus: Option<usize>) -> Emphasis {
    match focus {
        None => Emphasis::Rest,
        Some(node) if node == edge.from || node == edge.to => Emphasis::Focused,
        Some(_) => Emphasis::Dimmed,
    }
}

/// `color` as `emphasis` draws it: full strength when focused, faint when dimmed.
fn emphasized(color: Hsla, emphasis: Emphasis) -> Hsla {
    match emphasis {
        Emphasis::Rest => color,
        Emphasis::Focused => Hsla { a: 1., ..color },
        Emphasis::Dimmed => Hsla {
            a: EDGE_DIM_ALPHA,
            ..color
        },
    }
}

/// Whether an edge flows: it touches the selected node and is focused (hover alone animates
/// nothing), at a zoom where the cards show their text.
fn is_animated(
    edge: &TopologyEdge,
    focus: Option<usize>,
    selected: Option<usize>,
    zoom: f32,
) -> bool {
    let touches_selected = selected.is_some_and(|node| node == edge.from || node == edge.to);
    touches_selected && edge_emphasis(edge, focus) == Emphasis::Focused && zoom >= MIN_TEXT_ZOOM
}

/// How far the flow dashes have moved after `elapsed`, in screen px, wrapped at the dash `period`
/// (graph units).
fn flow_phase(elapsed: Duration, period: f32, zoom: f32) -> f32 {
    let moved = (elapsed.as_secs_f64() * f64::from(FLOW_SPEED)) % f64::from(period);
    moved as f32 * zoom
}

/// Whether the canvas needs frames: only while an edge flows, and not when the user reduces
/// motion or the window is in the background. Idle Topology never repaints on its own.
pub(crate) fn needs_flow_frame(
    animated: usize,
    is_motion_reduced: bool,
    is_window_active: bool,
) -> bool {
    animated > 0 && !is_motion_reduced && is_window_active
}

/// Where an edge attaches to its two cards: the dots the handle layer draws and the export puts
/// its circles.
pub(crate) fn handle_points(route: &EdgeRoute) -> [GraphPoint; 2] {
    [route.start(), route.end()]
}

/// The handle dot on screen: 6 px at zoom 1, never below `MIN_HANDLE_DIAMETER`.
fn handle_diameter(zoom: f32) -> f32 {
    (HANDLE_SIZE * zoom).max(MIN_HANDLE_DIAMETER)
}

/// Handles show only where the cards show their text.
fn shows_handles(zoom: f32) -> bool {
    card_detail(zoom) == Some(CardDetail::Text)
}

/// What the canvas layer paints and which handlers it registers.
pub(crate) struct CanvasPaint {
    pub(crate) graph: Rc<TopologyGraph>,
    pub(crate) layout: Rc<TopologyLayout>,
    pub(crate) viewport: Viewport,
    /// The node whose edges stand out: the hovered one, else the selected one.
    pub(crate) focus: Option<usize>,
    /// The selected node: its edges flow.
    pub(crate) selected: Option<usize>,
    /// The time since the view was created, which moves the flow dashes.
    pub(crate) elapsed: Duration,
    pub(crate) colors: CanvasColors,
    pub(crate) view: WeakEntity<TopologyView>,
    /// A drag runs, so the move and up handlers are registered this frame.
    pub(crate) is_dragging: bool,
    /// Traffic mode (spec 0049): what flows on each edge, and the `Calls` edges beside the graph.
    pub(crate) traffic: Option<Rc<TrafficLayer>>,
}

/// The paint layer under the cards. It registers the wheel handler always and the move and up
/// handlers while a drag runs, and tells the view its size, which Fit and focus need, and whether
/// an edge flows, which decides whether the view keeps a frame timer.
pub(crate) fn graph_canvas(paint: CanvasPaint) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let is_motion_reduced = cx.reduce_motion();
            let animated = paint_graph(&paint, bounds, window, is_motion_reduced);
            let wants_frames =
                needs_flow_frame(animated, is_motion_reduced, window.is_window_active());
            let _ = paint
                .view
                .update(cx, |view, cx| view.sync_flow(wants_frames, cx));
            register_handlers(&paint, bounds, hitbox, window, cx);
        },
    )
    .absolute()
    .size_full()
}

/// The handle dots over the cards. It has no hitbox, so the mouse goes to what is under it.
pub(crate) fn handle_canvas(
    graph: Rc<TopologyGraph>,
    layout: Rc<TopologyLayout>,
    traffic: Option<Rc<TrafficLayer>>,
    viewport: Viewport,
    colors: CanvasColors,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            paint_handles(
                &graph,
                &layout,
                traffic.as_deref(),
                viewport,
                &colors,
                bounds,
                window,
            );
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

/// Dots, bands, edges, then arrows, back to front. Returns how many edges flow.
fn paint_graph(
    paint: &CanvasPaint,
    bounds: Bounds<gpui_kit::Pixels>,
    window: &mut Window,
    is_motion_reduced: bool,
) -> usize {
    paint_dots(paint, bounds, window);
    paint_bands(paint, bounds, window);
    paint_edges(paint, bounds, window, is_motion_reduced)
}

fn paint_dots(paint: &CanvasPaint, bounds: Bounds<gpui_kit::Pixels>, window: &mut Window) {
    let viewport = paint.viewport;
    if dot_spacing(viewport.zoom()).is_none() {
        return;
    }
    let scale_factor = window.scale_factor();
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let first = viewport.to_graph(0., 0.);
    let start_x = (first.x / DOT_SPACING).ceil() * DOT_SPACING;
    let start_y = (first.y / DOT_SPACING).ceil() * DOT_SPACING;
    let dot = size(px(DOT_SIZE), px(DOT_SIZE));
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
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
            // Centered on the grid point, then snapped, so every dot has the same coverage.
            let origin = point(
                px(snap(left + screen_x - DOT_SIZE / 2., scale_factor)),
                px(snap(top + screen_y - DOT_SIZE / 2., scale_factor)),
            );
            window.paint_quad(quad(
                Bounds { origin, size: dot },
                px(DOT_SIZE / 2.),
                paint.colors.ring,
                px(0.),
                transparent_black(),
                BorderStyle::Solid,
            ));
            x += DOT_SPACING;
        }
        y += DOT_SPACING;
    }
}

/// The dot gap on screen at `zoom`, or `None` when the dots would be too dense to draw.
fn dot_spacing(zoom: f32) -> Option<f32> {
    let spacing = DOT_SPACING * zoom;
    (spacing >= MIN_DOT_SPACING).then_some(spacing)
}

/// A band frame on device pixels, so its dashed border is crisp.
fn paint_bands(paint: &CanvasPaint, bounds: Bounds<gpui_kit::Pixels>, window: &mut Window) {
    let viewport = paint.viewport;
    let zoom = viewport.zoom();
    let scale_factor = window.scale_factor();
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    for band in paint
        .layout
        .bands
        .iter()
        .filter(|band| band.title.is_some())
    {
        let (x, y) = viewport.to_screen(band.rect.origin);
        let (x0, y0) = (snap(left + x, scale_factor), snap(top + y, scale_factor));
        let x1 = snap(left + x + band.rect.width * zoom, scale_factor);
        let y1 = snap(top + y + band.rect.height * zoom, scale_factor);
        window.paint_quad(quad(
            Bounds {
                origin: point(px(x0), px(y0)),
                size: size(px(x1 - x0), px(y1 - y0)),
            },
            px(10. * zoom),
            paint.colors.muted.opacity(paint.colors.band_alpha),
            px(1.),
            paint.colors.card_border,
            BorderStyle::Dashed,
        ));
    }
}

/// The width of an edge on screen, in logical px.
fn edge_width(graph_width: f32, zoom: f32) -> f32 {
    (graph_width * zoom).max(MIN_EDGE_WIDTH)
}

/// The dash of an edge on screen. Below `MIN_TEXT_ZOOM` the pieces would be a few px long, so the
/// edge is solid.
fn edge_dash(dash: Option<(f32, f32)>, zoom: f32) -> Option<Dash> {
    let (on, off) = dash?;
    (zoom >= MIN_TEXT_ZOOM).then_some(Dash {
        on: on * zoom,
        off: off * zoom,
        phase: 0.,
    })
}

/// Every edge with its route and, in Traffic mode, what it carries: the graph's own edges first,
/// then the `Calls` edges the traffic added beside it (`overlay.edges` has the same order).
pub(crate) fn drawn_edges<'a>(
    graph: &'a TopologyGraph,
    routes: &'a [EdgeRoute],
    traffic: Option<&'a TrafficLayer>,
) -> impl Iterator<Item = (&'a TopologyEdge, &'a EdgeRoute, Option<&'a EdgeTraffic>)> {
    let (calls, call_routes): (&[TopologyEdge], &[EdgeRoute]) = traffic
        .map_or((&[], &[]), |layer| {
            (layer.calls.as_slice(), layer.call_routes.as_slice())
        });
    graph
        .edges
        .iter()
        .zip(routes)
        .chain(calls.iter().zip(call_routes))
        .enumerate()
        .map(move |(index, (edge, route))| {
            (
                edge,
                route,
                traffic.map(|layer| &layer.overlay.edges[index]),
            )
        })
}

/// Puts the edges of the focused node last, so the ones painted over them never hide them (a stable
/// sort: the rest keep their order).
fn sort_focused_last<T>(
    edges: &mut [(&TopologyEdge, T, Option<&EdgeTraffic>)],
    focus: Option<usize>,
) {
    edges.sort_by_key(|(edge, _, _)| edge_emphasis(edge, focus) == Emphasis::Focused);
}

/// Paints the edges and their arrows. Returns how many of them flow.
fn paint_edges(
    paint: &CanvasPaint,
    bounds: Bounds<gpui_kit::Pixels>,
    window: &mut Window,
    is_motion_reduced: bool,
) -> usize {
    let viewport = paint.viewport;
    let zoom = viewport.zoom();
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let feather = feather(window.scale_factor());
    let has_arrows = zoom >= MIN_BADGE_ZOOM;
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let at = |p: GraphPoint| {
        let (x, y) = viewport.to_screen(p);
        point(left + x, top + y)
    };
    let mut animated = 0;
    let mut edges: Vec<_> =
        drawn_edges(&paint.graph, &paint.layout.routes, paint.traffic.as_deref()).collect();
    sort_focused_last(&mut edges, paint.focus);
    for (edge, route, edge_traffic) in edges {
        if !touches(viewport, route.bounds(), width, height) {
            continue;
        }
        // A hidden edge (a mount or an access edge in Traffic mode) is not drawn.
        let look = match edge_traffic {
            Some(edge_traffic) => match traffic_look(edge_traffic, edge.relation) {
                Some(look) => Some(look),
                None => continue,
            },
            None => None,
        };
        let emphasis = edge_emphasis(edge, paint.focus);
        let stroke = relation_stroke(edge.relation);
        let is_flowing = is_animated(edge, paint.focus, paint.selected, zoom);
        let extra = if emphasis == Emphasis::Focused {
            FOCUS_EXTRA_WIDTH
        } else {
            0.
        };
        let (base, graph_width, rest_dash) = match &look {
            Some(look) => (
                traffic_color(&paint.colors, look, edge.relation),
                look.width,
                look.dash,
            ),
            None => (
                edge_color(
                    &paint.colors,
                    edge.relation,
                    paint.graph.ghost_tone(edge.to),
                ),
                stroke.width,
                stroke.dash,
            ),
        };
        let color = emphasized(base, emphasis);
        // The arrow stays solid at rest, so the direction reads at a glance; an idle or `Owns`
        // edge keeps its quiet arrow.
        let is_quiet = look
            .as_ref()
            .is_some_and(|look| look.alpha < EDGE_REST_ALPHA || look.is_muted);
        let arrow_base = if is_quiet {
            base
        } else {
            Hsla { a: 1., ..base }
        };
        let arrow_color = emphasized(arrow_base, emphasis);
        let mut points: Vec<Point<f32>> = route.points.iter().map(|p| at(*p)).collect();
        // The stroke stops at the arrow base, so a translucent edge does not double up under it.
        if has_arrows {
            trim_end(&mut points, (ARROW_LENGTH + ARROW_TIP_GAP) * zoom);
        }
        let line_width = edge_width(graph_width + extra, zoom);
        let dash = if is_flowing {
            animated += 1;
            let (on, off) = match &look {
                Some(look) => traffic_flow_dash(look, edge.relation),
                None => flow_dash(edge.relation),
            };
            // Reduced motion shows the flow dashes, standing still.
            let phase = if is_motion_reduced {
                0.
            } else {
                flow_phase(paint.elapsed, on + off, zoom)
            };
            Some(Dash {
                on: on * zoom,
                off: off * zoom,
                phase,
            })
        } else {
            edge_dash(rest_dash, zoom)
        };
        let path = match dash {
            Some(dash) => stroke_dashed(&points, dash, line_width, feather),
            None => stroke_line(&points, line_width, feather),
        };
        if let Some(path) = path {
            window.paint_path(path, color);
        }
        if !has_arrows {
            continue;
        }
        let head = arrow_head(route, ARROW_LENGTH, ARROW_HALF_WIDTH).map(at);
        if let Some(path) = fill_convex(&head, feather) {
            window.paint_path(path, arrow_color);
        }
    }
    animated
}

/// A round dot, in the color of its node, on both ends of every visible edge.
fn paint_handles(
    graph: &TopologyGraph,
    layout: &TopologyLayout,
    traffic: Option<&TrafficLayer>,
    viewport: Viewport,
    colors: &CanvasColors,
    bounds: Bounds<gpui_kit::Pixels>,
    window: &mut Window,
) {
    let zoom = viewport.zoom();
    if !shows_handles(zoom) {
        return;
    }
    let scale_factor = window.scale_factor();
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let diameter = handle_diameter(zoom);
    for (edge, route, edge_traffic) in drawn_edges(graph, &layout.routes, traffic) {
        let is_hidden = edge_traffic
            .is_some_and(|edge_traffic| traffic_look(edge_traffic, edge.relation).is_none());
        if is_hidden || !touches(viewport, route.bounds(), width, height) {
            continue;
        }
        let [start, end] = handle_points(route);
        for (at, node) in [(start, edge.from), (end, edge.to)] {
            let (x, y) = viewport.to_screen(at);
            let origin = point(
                px(snap(left + x - diameter / 2., scale_factor)),
                px(snap(top + y - diameter / 2., scale_factor)),
            );
            window.paint_quad(quad(
                Bounds {
                    origin,
                    size: size(px(diameter), px(diameter)),
                },
                px(diameter / 2.),
                colors.kind(kind_hue(graph.nodes[node].kind)),
                px(1.),
                colors.background,
                BorderStyle::Solid,
            ));
        }
    }
}

/// Whether a graph-space box touches a canvas of `width` by `height` at this viewport. The edges
/// are culled by the box of their route: a long edge crosses the canvas without an end in it.
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

/// What a legend swatch shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Swatch {
    /// The stroke of a relation at rest.
    Relation(Relation),
    /// A solid flow of a relation (Traffic mode).
    Flow(Relation),
    /// A flow in the tone of its 5xx share (Traffic mode).
    Tone(StatusTone),
}

/// The width of the swatch of a flow, in graph units.
pub(crate) const FLOW_SWATCH_WIDTH: f32 = 3.;

/// A legend swatch: a short edge with its arrow, drawn like the real ones.
pub(crate) fn legend_swatch(swatch: Swatch, colors: CanvasColors) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| paint_swatch(swatch, &colors, bounds, window),
    )
    .w(px(SWATCH_WIDTH))
    .h(px(SWATCH_HEIGHT))
}

/// How a swatch strokes: its width, its dash, and its color.
fn swatch_stroke(swatch: Swatch, colors: &CanvasColors) -> (f32, Option<Dash>, Hsla) {
    match swatch {
        Swatch::Relation(relation) => {
            let stroke = relation_stroke(relation);
            (
                stroke.width,
                edge_dash(stroke.dash, 1.),
                edge_color(colors, relation, None),
            )
        }
        Swatch::Flow(relation) => (
            FLOW_SWATCH_WIDTH,
            None,
            colors.relation(relation).opacity(EDGE_REST_ALPHA),
        ),
        Swatch::Tone(tone) => (FLOW_SWATCH_WIDTH, None, colors.tone(tone)),
    }
}

fn paint_swatch(
    swatch: Swatch,
    colors: &CanvasColors,
    bounds: Bounds<gpui_kit::Pixels>,
    window: &mut Window,
) {
    let feather = feather(window.scale_factor());
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let y = top + SWATCH_HEIGHT / 2.;
    let route = EdgeRoute {
        points: vec![
            GraphPoint { x: left, y },
            GraphPoint {
                x: left + SWATCH_WIDTH,
                y,
            },
        ],
    };
    let at = |p: GraphPoint| point(p.x, p.y);
    let mut points: Vec<Point<f32>> = route.points.iter().map(|p| at(*p)).collect();
    trim_end(&mut points, ARROW_LENGTH + ARROW_TIP_GAP);
    let (width, dash, color) = swatch_stroke(swatch, colors);
    let path = match dash {
        Some(dash) => stroke_dashed(&points, dash, width, feather),
        None => stroke_line(&points, width, feather),
    };
    if let Some(path) = path {
        window.paint_path(path, color);
    }
    let head = arrow_head(&route, ARROW_LENGTH, ARROW_HALF_WIDTH).map(at);
    if let Some(path) = fill_convex(&head, feather) {
        window.paint_path(path, Hsla { a: 1., ..color });
    }
}

/// The minimap (W11 `.mini`): every node as a small rect and the viewport as an outline.
pub(crate) struct MinimapPaint {
    pub(crate) graph: Rc<TopologyGraph>,
    pub(crate) layout: Rc<TopologyLayout>,
    pub(crate) viewport: Viewport,
    /// The size of the main canvas, so the viewport outline matches what it shows.
    pub(crate) canvas: (f32, f32),
    /// The size the minimap is drawn at: smaller while the drawer is open.
    pub(crate) size: (f32, f32),
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
    let transform = minimap_transform(paint.layout.extent, paint.size.0, paint.size.1);
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
            _ => paint.colors.kind(kind_hue(node.kind)),
        };
        window.paint_quad(quad(
            Bounds {
                origin: at(x, y),
                size: node_size,
            },
            px(MINIMAP_NODE_RADIUS),
            color,
            px(0.),
            transparent_black(),
            BorderStyle::Solid,
        ));
    }
    let zoom = paint.viewport.zoom();
    let (x, y) = transform.to_minimap(paint.viewport.origin);
    let view = GraphRect {
        origin: GraphPoint { x, y },
        width: paint.canvas.0 / zoom * transform.scale(),
        height: paint.canvas.1 / zoom * transform.scale(),
    };
    let mask = paint.colors.background.opacity(MINIMAP_MASK_ALPHA);
    for part in minimap_mask(view, paint.size.0, paint.size.1) {
        if part.width <= 0. || part.height <= 0. {
            continue;
        }
        window.paint_quad(fill(
            Bounds {
                origin: at(part.origin.x, part.origin.y),
                size: size(px(part.width), px(part.height)),
            },
            mask,
        ));
    }
    window.paint_quad(quad(
        Bounds {
            origin: at(x, y),
            size: size(px(view.width), px(view.height)),
        },
        px(0.),
        transparent_black(),
        px(1.5),
        paint.colors.ring,
        BorderStyle::Solid,
    ));
}

/// The four rects of a `width` by `height` minimap that lie outside `view`, which is clamped to
/// the minimap first: top, bottom, left, right. Together with the viewport they cover the whole.
fn minimap_mask(view: GraphRect, width: f32, height: f32) -> [GraphRect; 4] {
    let left = view.origin.x.clamp(0., width);
    let right = view.right().clamp(0., width);
    let top = view.origin.y.clamp(0., height);
    let bottom = view.bottom().clamp(0., height);
    let rect = |x: f32, y: f32, w: f32, h: f32| GraphRect {
        origin: GraphPoint { x, y },
        width: w,
        height: h,
    };
    [
        rect(0., 0., width, top),
        rect(0., bottom, width, height - bottom),
        rect(0., top, left, bottom - top),
        rect(right, top, width - right, bottom - top),
    ]
}

fn register_minimap_handlers(
    paint: &MinimapPaint,
    bounds: Bounds<gpui_kit::Pixels>,
    hitbox: Hitbox,
    window: &mut Window,
    _cx: &mut App,
) {
    let transform = minimap_transform(paint.layout.extent, paint.size.0, paint.size.1);
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

/// The relations of the legend, and what each stroke means.
pub(crate) const LEGEND: [(Relation, &str); 4] = [
    (Relation::Owns, "owns"),
    (Relation::RoutesTo, "routes to"),
    (Relation::Mounts, "mounts"),
    (Relation::Access, "access"),
];

/// The entries of the legend: the relations at rest, or in Traffic mode the flows and the 5xx
/// tones (the tones only when a source reports requests). `sources` are the ones that answered.
pub(crate) fn legend_entries(sources: Option<&[TrafficSourceKind]>) -> Vec<(Swatch, &'static str)> {
    let Some(sources) = sources else {
        return LEGEND
            .iter()
            .map(|(relation, text)| (Swatch::Relation(*relation), *text))
            .collect();
    };
    let has_requests = sources.contains(&TrafficSourceKind::Istio);
    let routes_to = if has_requests {
        "routes to \u{b7} width = req/s"
    } else {
        "routes to \u{b7} width = receive bytes/s per pod"
    };
    let mut entries = vec![(Swatch::Flow(Relation::RoutesTo), routes_to)];
    if has_requests {
        // Only Istio makes pairs without a Resources edge.
        entries.push((Swatch::Flow(Relation::Calls), "calls"));
    }
    entries.push((Swatch::Flow(Relation::Owns), "owns"));
    if has_requests {
        entries.push((Swatch::Tone(StatusTone::Warn), "\u{2265} 1% 5xx"));
        entries.push((Swatch::Tone(StatusTone::Bad), "\u{2265} 5% 5xx"));
    }
    entries
}
#[cfg(test)]
#[path = "topology_canvas_tests.rs"]
mod topology_canvas_tests;
