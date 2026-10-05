use super::*;
use crate::topology_viewport::MIN_TEXT_ZOOM;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn straight(from: (f32, f32), to: (f32, f32)) -> EdgeRoute {
    EdgeRoute {
        points: vec![
            GraphPoint {
                x: from.0,
                y: from.1,
            },
            GraphPoint { x: to.0, y: to.1 },
        ],
    }
}

fn edge(from: usize, to: usize) -> TopologyEdge {
    TopologyEdge {
        from,
        to,
        relation: Relation::Owns,
    }
}

fn color(alpha: f32) -> Hsla {
    Hsla {
        h: 0.5,
        s: 0.5,
        l: 0.5,
        a: alpha,
    }
}

#[test]
fn arrow_head_points_at_target() {
    let route = straight((0., 0.), (100., 40.));
    let [tip, left, right] = arrow_head(&route, ARROW_LENGTH, ARROW_HALF_WIDTH);
    // The tip stops a pixel short of the handle, on the line of the edge.
    assert!(close(
        (route.end().x - tip.x).hypot(route.end().y - tip.y),
        ARROW_TIP_GAP
    ));
    // The base lies on the source side of the tip, and the corners are symmetric about the edge.
    assert!(left.x < tip.x && right.x < tip.x);
    let middle = ((left.x + right.x) / 2., (left.y + right.y) / 2.);
    let (dx, dy) = route.end_direction();
    assert!(close(
        (tip.x - middle.0).hypot(tip.y - middle.1),
        ARROW_LENGTH
    ));
    assert!(close((tip.x - middle.0) * dy - (tip.y - middle.1) * dx, 0.));
}

#[test]
fn arrow_head_uses_its_half_width() {
    let route = straight((0., 0.), (100., 0.));
    let [tip, left, right] = arrow_head(&route, 8., 3.5);
    assert!(close(left.x, tip.x - 8.) && close(right.x, tip.x - 8.));
    assert!(close((left.y - right.y).abs(), 7.));
}

#[test]
fn the_arrow_follows_the_last_segment_of_a_bent_route() {
    let route = EdgeRoute {
        points: vec![
            GraphPoint { x: 0., y: 0. },
            GraphPoint { x: 50., y: 0. },
            GraphPoint { x: 50., y: 80. },
        ],
    };
    assert_eq!(route.end_direction(), (0., 1.));
    let [tip, left, right] = arrow_head(&route, ARROW_LENGTH, ARROW_HALF_WIDTH);
    assert!(close(tip.x, 50.) && close(tip.y, 80. - ARROW_TIP_GAP));
    assert!(left.y < tip.y && right.y < tip.y);
}

#[test]
fn relation_strokes_follow_the_legend() {
    assert_eq!(relation_stroke(Relation::Owns).dash, None);
    assert_eq!(relation_stroke(Relation::RoutesTo).dash, Some((7., 4.)));
    assert_eq!(relation_stroke(Relation::Mounts).dash, Some((4., 3.)));
    for relation in [Relation::Owns, Relation::RoutesTo, Relation::Mounts] {
        assert_eq!(relation_stroke(relation).width, 1.5);
    }
}

#[test]
fn dashes_turn_solid_below_text_zoom() {
    let dash = Some((5., 4.));
    assert_eq!(edge_dash(None, 1.), None);
    assert_eq!(
        edge_dash(dash, 2.),
        Some(Dash {
            on: 10.,
            off: 8.,
            phase: 0.
        })
    );
    assert!(edge_dash(dash, MIN_TEXT_ZOOM).is_some());
    assert_eq!(edge_dash(dash, MIN_TEXT_ZOOM - 0.01), None);
}

#[test]
fn edge_width_has_a_screen_floor() {
    assert!(close(edge_width(1.5, 1.), 1.5));
    assert!(close(edge_width(1.5, 2.), 3.));
    assert!(close(edge_width(1.2, 0.3), MIN_EDGE_WIDTH));
}

#[test]
fn dots_skip_below_the_minimum_spacing() {
    assert_eq!(dot_spacing(1.), Some(DOT_SPACING));
    assert!(dot_spacing(MIN_DOT_SPACING / DOT_SPACING).is_some());
    assert_eq!(dot_spacing(MIN_DOT_SPACING / DOT_SPACING - 0.01), None);
}

#[test]
fn minimap_mask_covers_the_outside_of_the_viewport() {
    let (width, height) = (150., 92.);
    let view = |x: f32, y: f32, w: f32, h: f32| GraphRect {
        origin: GraphPoint { x, y },
        width: w,
        height: h,
    };
    // Inside, and sticking out on the left and top.
    for view in [view(30., 20., 60., 40.), view(-20., -10., 80., 50.)] {
        let area: f32 = minimap_mask(view, width, height)
            .iter()
            .map(|part| part.width * part.height)
            .sum();
        let (left, top) = (view.origin.x.max(0.), view.origin.y.max(0.));
        let inside = (view.right().min(width) - left) * (view.bottom().min(height) - top);
        assert!(close(area + inside, width * height), "{area} + {inside}");
    }
}

#[test]
fn no_focus_leaves_every_edge_at_rest() {
    assert_eq!(edge_emphasis(&edge(0, 1), None), Emphasis::Rest);
    assert_eq!(emphasized(color(0.75), Emphasis::Rest), color(0.75));
}

#[test]
fn focus_emphasizes_its_edges_and_dims_the_rest() {
    assert_eq!(edge_emphasis(&edge(0, 1), Some(1)), Emphasis::Focused);
    assert_eq!(edge_emphasis(&edge(1, 2), Some(1)), Emphasis::Focused);
    assert_eq!(edge_emphasis(&edge(2, 3), Some(1)), Emphasis::Dimmed);
    assert_eq!(emphasized(color(0.75), Emphasis::Focused).a, 1.);
    assert_eq!(emphasized(color(0.75), Emphasis::Dimmed).a, EDGE_DIM_ALPHA);
}

#[test]
fn an_edge_arrow_is_solid_at_rest_and_faint_when_dimmed() {
    let solid = Hsla {
        a: 1.,
        ..color(0.8)
    };
    assert_eq!(emphasized(solid, Emphasis::Rest).a, 1.);
    assert_eq!(emphasized(solid, Emphasis::Dimmed).a, EDGE_DIM_ALPHA);
    assert!(emphasized(color(0.8), Emphasis::Rest).a < 1.);
}

#[test]
fn handles_sit_on_both_route_ends() {
    let route = straight((24., 24.), (234., 94.));
    assert_eq!(handle_points(&route), [route.start(), route.end()]);
}

#[test]
fn handles_show_at_text_detail_only() {
    assert!(shows_handles(1.));
    assert!(shows_handles(MIN_TEXT_ZOOM));
    assert!(!shows_handles(MIN_TEXT_ZOOM - 0.01));
    assert!(!shows_handles(MIN_BADGE_ZOOM));
}

#[test]
fn handles_never_shrink_below_their_minimum() {
    assert!(close(handle_diameter(1.), HANDLE_SIZE));
    assert!(close(handle_diameter(2.), 2. * HANDLE_SIZE));
    assert!(close(handle_diameter(MIN_TEXT_ZOOM), MIN_HANDLE_DIAMETER));
    const { assert!(MIN_HANDLE_DIAMETER >= 6.) };
}

#[test]
fn only_the_selected_nodes_edges_animate() {
    // Node 1 is selected and focused (nothing is hovered).
    assert!(is_animated(&edge(0, 1), Some(1), Some(1), 1.));
    assert!(is_animated(&edge(1, 2), Some(1), Some(1), 1.));
    assert!(!is_animated(&edge(2, 3), Some(1), Some(1), 1.));
}

#[test]
fn hover_alone_animates_nothing() {
    assert!(!is_animated(&edge(0, 1), Some(1), None, 1.));
    // Hovering another node dims the selected node's edges, which then stand still.
    assert!(!is_animated(&edge(0, 1), Some(3), Some(1), 1.));
}

#[test]
fn nothing_animates_below_text_zoom() {
    assert!(is_animated(&edge(0, 1), Some(1), Some(1), MIN_TEXT_ZOOM));
    assert!(!is_animated(
        &edge(0, 1),
        Some(1),
        Some(1),
        MIN_TEXT_ZOOM - 0.01
    ));
}

#[test]
fn a_flowing_edge_keeps_its_own_dash_and_a_solid_one_gets_long_dashes() {
    assert_eq!(flow_dash(Relation::RoutesTo), (7., 4.));
    assert_eq!(flow_dash(Relation::Mounts), (4., 3.));
    let owns = flow_dash(Relation::Owns);
    assert_eq!(owns, OWNS_FLOW_DASH);
    // The long dashes of a solid edge are not the dashes of the other relations.
    for relation in [Relation::RoutesTo, Relation::Mounts] {
        assert!(owns.0 > flow_dash(relation).0);
    }
}

#[test]
fn flow_phase_wraps_with_the_dash_period() {
    let period = 10.;
    assert!(close(flow_phase(Duration::ZERO, period, 1.), 0.));
    // 20 graph units a second: a quarter second is half a period.
    assert!(close(
        flow_phase(Duration::from_millis(250), period, 1.),
        period / 2.
    ));
    // A whole number of periods is back at the start; the zoom scales the phase.
    assert!(close(
        flow_phase(Duration::from_millis(500), period, 1.),
        0.
    ));
    assert!(close(
        flow_phase(Duration::from_millis(125), period, 2.),
        period / 2.
    ));
    // The period is the dash period of the relation.
    assert!(close(flow_phase(Duration::from_millis(300), 20., 1.), 6.));
}

#[test]
fn no_frame_request_when_idle_reduced_or_inactive() {
    assert!(needs_flow_frame(2, false, true));
    assert!(!needs_flow_frame(0, false, true));
    assert!(!needs_flow_frame(2, true, true));
    assert!(!needs_flow_frame(2, false, false));
}

fn flow(width: f32, tone: Option<StatusTone>) -> EdgeTraffic {
    EdgeTraffic::Flow(crate::topology_traffic::EdgeFlow {
        rate: 1.,
        unit: crate::topology_traffic::TrafficUnit::Requests,
        error_share: None,
        width,
        tone,
        label: None,
    })
}

#[test]
fn flow_edges_are_solid() {
    // A RoutesTo edge is dashed at rest, and solid once it carries a flow.
    assert_eq!(relation_stroke(Relation::RoutesTo).dash, Some((7., 4.)));
    for relation in [
        Relation::RoutesTo,
        Relation::Mounts,
        Relation::Access,
        Relation::Owns,
    ] {
        let look = traffic_look(&flow(3., None), relation).expect("drawn");
        assert_eq!(look.dash, None, "{relation:?}");
        assert_eq!(look.width, 3.);
    }
    let idle = traffic_look(&EdgeTraffic::Idle, Relation::RoutesTo).expect("drawn");
    assert_eq!(idle.dash, Some((2., 4.)));
    assert_eq!(idle.width, 0.75);
    assert!(idle.is_muted);
}

#[test]
fn hidden_edges_have_no_look() {
    assert_eq!(traffic_look(&EdgeTraffic::Hidden, Relation::Mounts), None);
}

#[test]
fn tones_and_owns_flows_set_the_alpha() {
    let bad = traffic_look(&flow(2., Some(StatusTone::Bad)), Relation::Calls).expect("drawn");
    assert_eq!((bad.tone, bad.alpha), (Some(StatusTone::Bad), 1.));
    let owns = traffic_look(&flow(2., None), Relation::Owns).expect("drawn");
    assert_eq!(owns.alpha, 0.6);
    let routes = traffic_look(&flow(2., None), Relation::RoutesTo).expect("drawn");
    assert_eq!(routes.alpha, EDGE_REST_ALPHA);
}

#[test]
fn traffic_colors_come_from_theme_tokens() {
    let colors = CanvasColors::light();
    let bad = traffic_look(&flow(2., Some(StatusTone::Bad)), Relation::Calls).expect("drawn");
    assert_eq!(
        traffic_color(&colors, &bad, Relation::Calls),
        colors.bad.opacity(1.)
    );
    let idle = traffic_look(&EdgeTraffic::Idle, Relation::Owns).expect("drawn");
    assert_eq!(
        traffic_color(&colors, &idle, Relation::Owns),
        colors.muted_foreground.opacity(EDGE_REST_ALPHA)
    );
    let calls = traffic_look(&flow(2., None), Relation::Calls).expect("drawn");
    assert_eq!(
        traffic_color(&colors, &calls, Relation::Calls),
        colors.relation(Relation::Calls).opacity(EDGE_REST_ALPHA)
    );
}

#[test]
fn the_legend_follows_the_sources() {
    let resources = legend_entries(None);
    assert_eq!(resources.len(), LEGEND.len());
    assert!(
        resources
            .iter()
            .all(|(swatch, _)| matches!(swatch, Swatch::Relation(_)))
    );
    let istio = legend_entries(Some(&[
        TrafficSourceKind::Istio,
        TrafficSourceKind::PodNetwork,
    ]));
    let texts: Vec<&str> = istio.iter().map(|(_, text)| *text).collect();
    assert_eq!(
        texts,
        [
            "routes to \u{b7} width = req/s",
            "calls",
            "owns",
            "\u{2265} 1% 5xx",
            "\u{2265} 5% 5xx"
        ]
    );
    let bytes = legend_entries(Some(&[TrafficSourceKind::PodNetwork]));
    let texts: Vec<&str> = bytes.iter().map(|(_, text)| *text).collect();
    assert_eq!(
        texts,
        ["routes to \u{b7} width = receive bytes/s per pod", "owns"]
    );
    // Every Traffic entry is a flow, as the edges are drawn, except the tones.
    assert!(
        istio
            .iter()
            .all(|(swatch, _)| !matches!(swatch, Swatch::Relation(_)))
    );
}

#[test]
fn a_traffic_flow_breaks_into_relation_dashes_and_an_idle_edge_keeps_its_dots() {
    let routes = traffic_look(&flow(4., None), Relation::RoutesTo).expect("drawn");
    assert_eq!(traffic_flow_dash(&routes, Relation::RoutesTo), (7., 4.));
    let calls = traffic_look(&flow(4., None), Relation::Calls).expect("drawn");
    assert_eq!(traffic_flow_dash(&calls, Relation::Calls), OWNS_FLOW_DASH);
    let idle = traffic_look(&EdgeTraffic::Idle, Relation::Calls).expect("drawn");
    assert_eq!(traffic_flow_dash(&idle, Relation::Calls), IDLE_DASH);
}

#[test]
fn a_traffic_edge_flows_only_at_the_selected_node() {
    // Traffic edges use the same predicate as resource edges: selected node, focused, text zoom.
    assert!(is_animated(&edge(1, 2), Some(1), Some(1), 1.));
    assert!(!is_animated(&edge(2, 3), Some(1), Some(1), 1.));
    assert!(!is_animated(&edge(1, 2), Some(1), None, 1.));
}
