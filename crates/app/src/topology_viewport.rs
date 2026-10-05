//! The Topology viewport (W11): which part of the graph shows and at what zoom, the wheel and drag
//! rules, and the minimap transform. Pure: no GPUI context.

use gpui_kit::ScrollDelta;

use crate::topology_card::MIN_BADGE_ZOOM;
use crate::topology_layout::{GraphPoint, GraphRect, TopologyLayout};

/// One wheel notch multiplies the zoom by this.
pub(crate) const WHEEL_STEP: f32 = 1.1;
/// The zoom steps of the wheel: `WHEEL_STEP^step` from about 0.2 to about 1.95.
const MIN_ZOOM_STEP: i32 = -17;
const MAX_ZOOM_STEP: i32 = 7;
/// Fit may go further out than the wheel (about 0.09), so it can show a very large graph whole.
const MIN_FIT_STEP: i32 = -25;
/// Pixel-precise scroll deltas add up to one step per this many pixels.
const PIXELS_PER_ZOOM_STEP: f32 = 50.;
/// The + and - buttons zoom this many wheel steps (x1.21, close to React Flow's 1.2).
pub(crate) const ZOOM_BUTTON_STEPS: i32 = 2;
/// A press that moves less than this is a click.
const DRAG_SLOP: f32 = 4.;
/// Cards draw their text lines from this zoom on: the 12.5 px name is then about 7 px.
pub(crate) const MIN_TEXT_ZOOM: f32 = 0.55;
/// The first view of a graph never goes below this zoom: the name is then about 10 px.
const FIRST_VIEW_ZOOM: f32 = 0.8;
/// The left strip the zoom panel covers (its width, its offset, and a gutter): the first view and
/// Fit keep the graph out of it.
pub(crate) const CONTROLS_INSET: f32 = 60.;
pub(crate) const MINIMAP_WIDTH: f32 = 150.;
pub(crate) const MINIMAP_HEIGHT: f32 = 92.;
/// The bottom strip the minimap and legend cover: Fit and the first view keep the graph above it.
pub(crate) const OVERLAY_GUTTER: f32 = MINIMAP_HEIGHT + 16.;

/// What part of the graph shows, and at what zoom. The zoom is a step of a fixed grid, so wheel
/// zooming is predictable (decision 25).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport {
    /// The graph point at the top-left of the canvas.
    pub(crate) origin: GraphPoint,
    zoom_step: i32,
}

impl Viewport {
    pub(crate) fn zoom(&self) -> f32 {
        WHEEL_STEP.powi(self.zoom_step)
    }

    /// A graph point in canvas pixels, relative to the canvas bounds.
    pub(crate) fn to_screen(self, point: GraphPoint) -> (f32, f32) {
        let zoom = self.zoom();
        (
            (point.x - self.origin.x) * zoom,
            (point.y - self.origin.y) * zoom,
        )
    }

    pub(crate) fn to_graph(self, x: f32, y: f32) -> GraphPoint {
        let zoom = self.zoom();
        GraphPoint {
            x: x / zoom + self.origin.x,
            y: y / zoom + self.origin.y,
        }
    }

    /// `steps` notches around the canvas pixel `(x, y)`, which keeps its graph point. The wheel
    /// zooms out to the wheel floor, but a view that Fit put below it is not pushed back up.
    pub(crate) fn zoom_at(self, x: f32, y: f32, steps: i32) -> Self {
        let anchor = self.to_graph(x, y);
        let floor = MIN_ZOOM_STEP.min(self.zoom_step);
        let zoom_step = (self.zoom_step + steps).clamp(floor, MAX_ZOOM_STEP);
        let zoom = WHEEL_STEP.powi(zoom_step);
        Self {
            origin: GraphPoint {
                x: anchor.x - x / zoom,
                y: anchor.y - y / zoom,
            },
            zoom_step,
        }
    }

    /// Moves the content by `(dx, dy)` canvas pixels.
    pub(crate) fn pan(self, dx: f32, dy: f32) -> Self {
        let zoom = self.zoom();
        Self {
            origin: GraphPoint {
                x: self.origin.x - dx / zoom,
                y: self.origin.y - dy / zoom,
            },
            ..self
        }
    }

    /// The largest zoom of the grid that shows all of `extent` and is at most 1, centered. It
    /// reaches below the wheel floor, down to `MIN_FIT_STEP`.
    pub(crate) fn fit(extent: GraphRect, width: f32, height: f32) -> Self {
        Self {
            origin: GraphPoint { x: 0., y: 0. },
            zoom_step: fit_step(extent, width, height),
        }
        .center_on(extent.center(), width, height)
    }

    /// The view a graph opens with: Fit when the whole graph keeps the kind badges
    /// (`MIN_BADGE_ZOOM`; below `MIN_TEXT_ZOOM` the cards show no name), else the readable zoom
    /// (`FIRST_VIEW_ZOOM`) anchored at the top-left of the extent. The minimap gives the overview
    /// of what is out of view.
    pub(crate) fn first_view(extent: GraphRect, width: f32, height: f32) -> Self {
        if WHEEL_STEP.powi(fit_step(extent, width, height)) >= MIN_BADGE_ZOOM {
            return Self::fit(extent, width, height);
        }
        Self {
            origin: extent.origin,
            zoom_step: readable_step(),
        }
    }

    /// Whether all of `extent` shows in a canvas of `width` by `height` at this zoom.
    pub(crate) fn shows_whole(self, extent: GraphRect, width: f32, height: f32) -> bool {
        let zoom = self.zoom();
        extent.width * zoom <= width + 0.5 && extent.height * zoom <= height + 0.5
    }

    /// Pans just enough for `rect` to lie `margin` px inside an area of `width` by `height`, or
    /// into its middle when it does not fit. The area is the part of the canvas nothing covers.
    pub(crate) fn reveal(self, rect: GraphRect, width: f32, height: f32, margin: f32) -> Self {
        let zoom = self.zoom();
        let (x, y) = self.to_screen(rect.origin);
        let shift = |position: f32, extent: f32, area: f32| {
            if extent + 2. * margin > area {
                area / 2. - (position + extent / 2.)
            } else if position < margin {
                margin - position
            } else if position + extent > area - margin {
                area - margin - (position + extent)
            } else {
                0.
            }
        };
        self.pan(
            shift(x, rect.width * zoom, width),
            shift(y, rect.height * zoom, height),
        )
    }

    /// Brings `focus` and its `neighbours` into the free area. The least pan when they all fit at
    /// this zoom; else centered at the largest zoom of the grid below this one that fits them and
    /// still shows the card text; else at this zoom, the nearest neighbours that fit with `focus`
    /// (none, when the node alone fills the area).
    pub(crate) fn reveal_group(
        self,
        focus: GraphRect,
        neighbours: &[GraphRect],
        width: f32,
        height: f32,
        margin: f32,
    ) -> Self {
        let fits = |group: &GraphRect, zoom: f32| {
            group.width * zoom + 2. * margin <= width && group.height * zoom + 2. * margin <= height
        };
        let whole = neighbours
            .iter()
            .fold(focus, |group, other| group.union(other));
        if fits(&whole, self.zoom()) {
            return self.reveal(whole, width, height, margin);
        }
        let smaller = (MIN_FIT_STEP..self.zoom_step).rev().find(|zoom_step| {
            let zoom = WHEEL_STEP.powi(*zoom_step);
            zoom >= MIN_TEXT_ZOOM && fits(&whole, zoom)
        });
        if let Some(zoom_step) = smaller {
            return Self { zoom_step, ..self }.center_on(whole.center(), width, height);
        }
        let center = focus.center();
        let distance = |rect: &GraphRect| {
            let other = rect.center();
            (other.x - center.x).hypot(other.y - center.y)
        };
        let mut nearest: Vec<&GraphRect> = neighbours.iter().collect();
        nearest.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
        let group = nearest.into_iter().fold(focus, |group, other| {
            let wider = group.union(other);
            if fits(&wider, self.zoom()) {
                wider
            } else {
                group
            }
        });
        self.reveal(group, width, height, margin)
    }

    /// Puts `target` in the middle of a canvas of `width` by `height`.
    pub(crate) fn center_on(self, target: GraphPoint, width: f32, height: f32) -> Self {
        let zoom = self.zoom();
        Self {
            origin: GraphPoint {
                x: target.x - width / 2. / zoom,
                y: target.y - height / 2. / zoom,
            },
            ..self
        }
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            origin: GraphPoint { x: 0., y: 0. },
            zoom_step: 0,
        }
    }
}

/// The fit zoom step: the largest grid step at most 1 that shows `extent` whole.
fn fit_step(extent: GraphRect, width: f32, height: f32) -> i32 {
    let wanted = (width / extent.width.max(1.))
        .min(height / extent.height.max(1.))
        .min(1.);
    (MIN_FIT_STEP..=0)
        .rev()
        .find(|step| WHEEL_STEP.powi(*step) <= wanted)
        .unwrap_or(MIN_FIT_STEP)
}

/// The smallest grid step whose zoom reads comfortably: the one a first view starts at.
fn readable_step() -> i32 {
    (MIN_FIT_STEP..=MAX_ZOOM_STEP)
        .find(|step| WHEEL_STEP.powi(*step) >= FIRST_VIEW_ZOOM)
        .unwrap_or(0)
}

/// The notches a scroll delta adds up to; `carry` keeps the part of a notch not reached yet.
/// Positive (scrolling up) zooms in.
pub(crate) fn wheel_steps(delta: ScrollDelta, carry: &mut f32) -> i32 {
    *carry += match delta {
        ScrollDelta::Lines(lines) => lines.y,
        ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / PIXELS_PER_ZOOM_STEP,
    };
    let steps = carry.trunc();
    *carry -= steps;
    steps as i32
}

/// Whether a press that moved by `(dx, dy)` is a drag and not a click.
pub(crate) fn is_drag(dx: f32, dy: f32) -> bool {
    dx.hypot(dy) >= DRAG_SLOP
}

/// `value` (logical px) on the nearest device pixel.
pub(crate) fn snap(value: f32, scale_factor: f32) -> f32 {
    (value * scale_factor).round() / scale_factor
}

/// The indices of the nodes whose card shows in a canvas of `width` by `height`.
pub(crate) fn visible_nodes(
    layout: &TopologyLayout,
    viewport: Viewport,
    width: f32,
    height: f32,
) -> Vec<usize> {
    let zoom = viewport.zoom();
    layout
        .rects
        .iter()
        .enumerate()
        .filter(|(_, rect)| {
            let (x, y) = viewport.to_screen(rect.origin);
            x + rect.width * zoom >= 0. && y + rect.height * zoom >= 0. && x <= width && y <= height
        })
        .map(|(index, _)| index)
        .collect()
}

/// A graph fitted into the minimap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MinimapTransform {
    scale: f32,
    offset_x: f32,
    offset_y: f32,
    origin: GraphPoint,
}

impl MinimapTransform {
    pub(crate) fn to_minimap(self, point: GraphPoint) -> (f32, f32) {
        (
            (point.x - self.origin.x) * self.scale + self.offset_x,
            (point.y - self.origin.y) * self.scale + self.offset_y,
        )
    }

    pub(crate) fn to_graph(self, x: f32, y: f32) -> GraphPoint {
        GraphPoint {
            x: (x - self.offset_x) / self.scale + self.origin.x,
            y: (y - self.offset_y) / self.scale + self.origin.y,
        }
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }
}

/// `extent` scaled to fit a minimap of `width` by `height`, centered.
pub(crate) fn minimap_transform(extent: GraphRect, width: f32, height: f32) -> MinimapTransform {
    let scale = (width / extent.width.max(1.)).min(height / extent.height.max(1.));
    MinimapTransform {
        scale,
        offset_x: (width - extent.width * scale) / 2.,
        offset_y: (height - extent.height * scale) / 2.,
        origin: extent.origin,
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{point, px};

    use super::*;

    fn extent(width: f32, height: f32) -> GraphRect {
        GraphRect {
            origin: GraphPoint { x: 0., y: 0. },
            width,
            height,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn screen_graph_round_trip() {
        let viewport = Viewport::default().zoom_at(30., 40., 3).pan(12., -7.);
        let graph = GraphPoint { x: 123., y: 456. };
        let (x, y) = viewport.to_screen(graph);
        let back = viewport.to_graph(x, y);
        assert!(close(back.x, graph.x) && close(back.y, graph.y));
    }

    #[test]
    fn zoom_at_keeps_cursor_point() {
        let viewport = Viewport::default().pan(-50., -20.);
        let before = viewport.to_graph(300., 200.);
        let zoomed = viewport.zoom_at(300., 200., 4);
        let after = zoomed.to_graph(300., 200.);
        assert!(close(before.x, after.x) && close(before.y, after.y));
        assert!(zoomed.zoom() > viewport.zoom());
    }

    #[test]
    fn zoom_steps_are_on_the_grid() {
        let zoomed = Viewport::default().zoom_at(0., 0., 2).zoom_at(0., 0., -5);
        assert!(close(zoomed.zoom(), WHEEL_STEP.powi(-3)));
    }

    #[test]
    fn zoom_is_clamped() {
        let most = Viewport::default().zoom_at(0., 0., 100);
        let least = Viewport::default().zoom_at(0., 0., -100);
        assert!(close(most.zoom(), WHEEL_STEP.powi(MAX_ZOOM_STEP)));
        assert!(close(least.zoom(), WHEEL_STEP.powi(MIN_ZOOM_STEP)));
        assert!(most.zoom() < 2. && least.zoom() > 0.19);
    }

    #[test]
    fn fit_picks_grid_zoom_and_never_upscales() {
        assert!(close(
            Viewport::fit(extent(200., 100.), 1000., 800.).zoom(),
            1.
        ));
        let fitted = Viewport::fit(extent(2000., 400.), 1000., 800.);
        assert!(fitted.zoom() <= 0.5);
        assert!(fitted.zoom() > 0.5 / WHEEL_STEP);
        let step = (fitted.zoom().ln() / WHEEL_STEP.ln()).round() as i32;
        assert!(close(fitted.zoom(), WHEEL_STEP.powi(step)));
    }

    #[test]
    fn fit_goes_below_the_wheel_floor_to_show_everything() {
        // A tall graph needs about 0.1: the wheel stops at about 0.2, Fit does not.
        let fitted = Viewport::fit(extent(800., 7_000.), 1000., 700.);
        assert!(fitted.zoom() < 0.2);
        assert!(fitted.zoom() * 7_000. <= 700.);
        // The floor of Fit itself is step -25.
        let huge = Viewport::fit(extent(800., 70_000.), 1000., 700.);
        assert!(close(huge.zoom(), WHEEL_STEP.powi(MIN_FIT_STEP)));
    }

    #[test]
    fn the_wheel_does_not_push_a_fitted_view_back_up() {
        let fitted = Viewport::fit(extent(800., 7_000.), 1000., 700.);
        let out = fitted.zoom_at(0., 0., -3);
        assert!(close(out.zoom(), fitted.zoom()));
        assert!(fitted.zoom_at(0., 0., 1).zoom() > fitted.zoom());
    }

    #[test]
    fn first_view_fits_a_graph_that_is_readable_whole() {
        let small = extent(600., 300.);
        assert_eq!(
            Viewport::first_view(small, 1000., 700.),
            Viewport::fit(small, 1000., 700.)
        );
    }

    #[test]
    fn first_view_of_a_large_graph_is_readable_and_anchored_top_left() {
        let large = GraphRect {
            origin: GraphPoint { x: 10., y: 20. },
            width: 3_000.,
            height: 4_000.,
        };
        let view = Viewport::first_view(large, 1000., 700.);
        assert!(view.zoom() >= FIRST_VIEW_ZOOM);
        assert!(view.zoom() < FIRST_VIEW_ZOOM * WHEEL_STEP);
        assert_eq!(view.origin, large.origin);
        // The fit of the same graph is far smaller.
        assert!(Viewport::fit(large, 1000., 700.).zoom() < MIN_BADGE_ZOOM);
    }

    #[test]
    fn center_on_puts_point_mid_view() {
        let viewport = Viewport::default().zoom_at(0., 0., -3);
        let target = GraphPoint { x: 500., y: 300. };
        let centered = viewport.center_on(target, 800., 600.);
        let (x, y) = centered.to_screen(target);
        assert!(close(x, 400.) && close(y, 300.));
    }

    #[test]
    fn visible_nodes_culls_offscreen() {
        use crate::topology_fixtures::Fixture;
        use crate::topology_graph::GroupBy;
        let graph = Fixture::default()
            .with_deployment("api", 1, 1)
            .with_deployment("web", 1, 1)
            .graph();
        let layout = crate::topology_layout::layout(
            &graph,
            GroupBy::Components,
            1.,
            &std::collections::HashMap::new(),
            None,
        );
        let all = visible_nodes(&layout, Viewport::default(), 2000., 2000.);
        assert_eq!(all.len(), graph.nodes.len());
        let panned = Viewport::default().pan(-5000., 0.);
        assert!(visible_nodes(&layout, panned, 800., 600.).is_empty());
    }

    #[test]
    fn minimap_maps_both_ways() {
        let transform = minimap_transform(extent(1500., 400.), MINIMAP_WIDTH, MINIMAP_HEIGHT);
        let graph = GraphPoint { x: 750., y: 200. };
        let (x, y) = transform.to_minimap(graph);
        assert!((0. ..=MINIMAP_WIDTH).contains(&x) && (0. ..=MINIMAP_HEIGHT).contains(&y));
        let back = transform.to_graph(x, y);
        assert!(close(back.x, graph.x) && close(back.y, graph.y));
    }

    #[test]
    fn drag_below_slop_is_a_click() {
        assert!(!is_drag(2., 2.));
        assert!(!is_drag(0., DRAG_SLOP - 0.1));
        assert!(is_drag(DRAG_SLOP, 0.));
        assert!(is_drag(3., 3.));
    }

    #[test]
    fn wheel_steps_count_notches_and_carry_pixels() {
        let mut carry = 0.;
        assert_eq!(
            wheel_steps(ScrollDelta::Lines(point(0., 1.)), &mut carry),
            1
        );
        assert_eq!(
            wheel_steps(ScrollDelta::Lines(point(0., -2.)), &mut carry),
            -2
        );
        let pixels = |y: f32| ScrollDelta::Pixels(point(px(0.), px(y)));
        assert_eq!(wheel_steps(pixels(30.), &mut carry), 0);
        assert_eq!(wheel_steps(pixels(30.), &mut carry), 1);
    }

    #[test]
    fn button_zoom_keeps_the_view_center() {
        let viewport = Viewport::default().pan(-120., -80.);
        let (width, height) = (1000., 600.);
        let center = viewport.to_graph(width / 2., height / 2.);
        for steps in [ZOOM_BUTTON_STEPS, -ZOOM_BUTTON_STEPS] {
            let zoomed = viewport.zoom_at(width / 2., height / 2., steps);
            let kept = zoomed.to_graph(width / 2., height / 2.);
            assert!(close(kept.x, center.x) && close(kept.y, center.y));
            assert!(!close(zoomed.zoom(), viewport.zoom()));
        }
    }

    #[test]
    fn button_zoom_is_clamped_like_the_wheel() {
        let mut viewport = Viewport::default();
        for _ in 0..20 {
            viewport = viewport.zoom_at(500., 300., ZOOM_BUTTON_STEPS);
        }
        let most = WHEEL_STEP.powi(MAX_ZOOM_STEP);
        assert!(close(viewport.zoom(), most));
        for _ in 0..40 {
            viewport = viewport.zoom_at(500., 300., -ZOOM_BUTTON_STEPS);
        }
        assert!(close(viewport.zoom(), WHEEL_STEP.powi(MIN_ZOOM_STEP)));
    }

    #[test]
    fn a_graph_opens_whole_while_its_badges_show_whatever_its_node_count() {
        // Fit would be 0.7: below the first-view zoom, above the text zoom.
        let wide = extent(1_400., 900.);
        let fit = Viewport::fit(wide, 1000., 700.);
        assert!(fit.zoom() < FIRST_VIEW_ZOOM && fit.zoom() >= MIN_TEXT_ZOOM);
        assert_eq!(Viewport::first_view(wide, 1000., 700.), fit);
        assert!(fit.shows_whole(wide, 1000., 700.));
        // Below the text zoom the cards show badges only, which still reads as an overview.
        let many = extent(3_000., 2_000.);
        let overview = Viewport::first_view(many, 1000., 700.);
        assert!(overview.zoom() < MIN_TEXT_ZOOM && overview.zoom() >= MIN_BADGE_ZOOM);
        assert!(overview.shows_whole(many, 1000., 700.));
        // A graph whose fit is below the badge zoom keeps the readable zoom, anchored top-left.
        let huge = extent(4_000., 3_000.);
        let part = Viewport::first_view(huge, 1000., 700.);
        assert!(part.zoom() >= FIRST_VIEW_ZOOM);
        assert_eq!(part.origin, huge.origin);
        assert!(!part.shows_whole(huge, 1000., 700.));
    }

    #[test]
    fn reveal_pans_a_hidden_card_into_the_free_area() {
        let card = GraphRect {
            origin: GraphPoint { x: 900., y: 300. },
            width: 200.,
            height: 60.,
        };
        let view = Viewport::default();
        // Under the drawer: the free area is 600 px wide.
        let moved = view.reveal(card, 600., 700., 20.);
        let (x, _) = moved.to_screen(card.origin);
        assert!(close(x + card.width * moved.zoom(), 600. - 20.));
        // Already inside: nothing moves.
        assert_eq!(view.reveal(card, 1_400., 700., 20.), view);
        // A card that cannot fit is centered.
        let centered = view.reveal(card, 150., 700., 20.);
        let (x, _) = centered.to_screen(card.origin);
        assert!(close(x + card.width / 2., 75.));
    }

    fn card(x: f32, y: f32) -> GraphRect {
        GraphRect {
            origin: GraphPoint { x, y },
            width: 200.,
            height: 60.,
        }
    }

    #[test]
    fn reveal_group_pans_when_the_group_fits_the_free_area() {
        let node = card(900., 300.);
        let neighbours = [card(600., 250.), card(1_100., 390.)];
        let whole = node.union(&neighbours[0]).union(&neighbours[1]);
        let view = Viewport::default();
        let moved = view.reveal_group(node, &neighbours, 900., 700., 20.);
        assert!(close(moved.zoom(), view.zoom()), "the zoom is not changed");
        let (left, top) = moved.to_screen(whole.origin);
        assert!(left >= 20. - 0.01 && top >= 20. - 0.01);
        assert!(left + whole.width <= 900. - 20. + 0.01);
        // Already inside: nothing moves.
        assert_eq!(
            view.reveal_group(node, &neighbours, 1_400., 700., 20.),
            view
        );
    }

    #[test]
    fn reveal_group_zooms_out_only_as_far_as_needed_and_keeps_the_text() {
        let node = card(900., 300.);
        let neighbours = [card(100., 100.), card(1_300., 440.)];
        let whole = node.union(&neighbours[0]).union(&neighbours[1]);
        let view = Viewport::default();
        let zoomed = view.reveal_group(node, &neighbours, 1_000., 600., 20.);
        assert!(zoomed.zoom() < view.zoom() && zoomed.zoom() >= MIN_TEXT_ZOOM);
        assert!(whole.width * zoomed.zoom() + 40. <= 1_000.);
        // One step further out would not be needed.
        assert!(whole.width * zoomed.zoom() * WHEEL_STEP + 40. > 1_000.);
        let (left, _) = zoomed.to_screen(whole.origin);
        assert!(left >= 20. - 0.01);
    }

    #[test]
    fn reveal_group_keeps_the_nearest_neighbours_when_no_readable_zoom_fits_all() {
        let node = card(900., 300.);
        let near = card(1_150., 300.);
        let far = card(-3_000., 300.);
        let view = Viewport::default();
        let moved = view.reveal_group(node, &[far, near], 1_000., 600., 20.);
        assert_eq!(moved.zoom(), view.zoom());
        // The node and the near card are inside the area; the far one is not.
        let inside = |rect: GraphRect| {
            let (x, _) = moved.to_screen(rect.origin);
            x >= 20. - 0.01 && x + rect.width <= 1_000. - 20. + 0.01
        };
        assert!(inside(node) && inside(near) && !inside(far));
        // A node too wide for any readable zoom is revealed like before.
        let wide = GraphRect {
            width: 3_000.,
            ..node
        };
        assert_eq!(
            view.reveal_group(wide, &[near], 1_000., 600., 20.),
            view.reveal(wide, 1_000., 600., 20.)
        );
    }

    #[test]
    fn snap_rounds_to_device_pixels() {
        assert!(close(snap(10.4, 1.), 10.));
        assert!(close(snap(10.4, 2.), 10.5));
        assert!(close(snap(10.4, 1.5), 10. + 2. / 3.));
        assert!(close(snap(-0.3, 1.), 0.));
    }
}
