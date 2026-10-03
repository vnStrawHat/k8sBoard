//! The Topology viewport (W11): which part of the graph shows and at what zoom, the wheel and drag
//! rules, and the minimap transform. Pure: no GPUI context.

use gpui_kit::ScrollDelta;

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
/// A press that moves less than this is a click.
const DRAG_SLOP: f32 = 4.;
/// Cards draw their text lines from this zoom on (and the first view never goes below it).
pub(crate) const MIN_TEXT_ZOOM: f32 = 0.55;
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

    /// The view a graph opens with: Fit when the whole graph is readable at that zoom, else the
    /// readable zoom (`MIN_TEXT_ZOOM`) anchored at the top-left of the extent. The minimap gives
    /// the overview of what is out of view.
    pub(crate) fn first_view(extent: GraphRect, width: f32, height: f32) -> Self {
        let readable = readable_step();
        if fit_step(extent, width, height) >= readable {
            return Self::fit(extent, width, height);
        }
        Self {
            origin: extent.origin,
            zoom_step: readable,
        }
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

/// The smallest grid step whose zoom still draws the text of a card.
fn readable_step() -> i32 {
    (MIN_FIT_STEP..=MAX_ZOOM_STEP)
        .find(|step| WHEEL_STEP.powi(*step) >= MIN_TEXT_ZOOM)
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
        assert!(view.zoom() >= MIN_TEXT_ZOOM);
        assert!(view.zoom() < MIN_TEXT_ZOOM * WHEEL_STEP);
        assert_eq!(view.origin, large.origin);
        // The fit of the same graph is far smaller.
        assert!(Viewport::fit(large, 1000., 700.).zoom() < MIN_TEXT_ZOOM);
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
}
