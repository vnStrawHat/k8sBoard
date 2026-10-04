use super::*;
use crate::topology_graph::{NodeId, TopologyKind};
use crate::topology_layout::MIN_NODE_WIDTH;

fn node(kind: TopologyKind, tone: Option<StatusTone>) -> TopologyNode {
    TopologyNode {
        id: NodeId::Object {
            kind,
            name: "api".to_owned(),
        },
        kind,
        look: NodeLook::Plain,
        name: "api".into(),
        caption: "caption".into(),
        tone,
        group: None,
        objects: 1,
        key: None,
    }
}

fn card_state(is_selected: bool, is_hovered: bool) -> CardState {
    CardState {
        is_selected,
        is_hovered,
        is_highlighted: false,
        tooltip: "api".into(),
        caption: "Service".into(),
    }
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
fn a_box_card_is_filled_below_badge_zoom() {
    let colors = CanvasColors::light();
    let hue = kind_hue(TopologyKind::Deployment);
    assert_eq!(card_detail(MIN_BADGE_ZOOM - 0.01), None);
    let fill = box_fill(&colors, hue);
    assert_eq!(fill.a, KIND_BOX_TINT * colors.kind(hue).a);
    assert_eq!((fill.h, fill.s, fill.l), {
        let kind = colors.kind(hue);
        (kind.h, kind.s, kind.l)
    });
}

#[test]
fn an_object_card_carries_a_trace_of_its_kind_and_a_ghost_does_not() {
    let colors = CanvasColors::light();
    let plain = node(TopologyKind::Service, None);
    let state = card_state(false, false);
    let surface = card_surface(&plain, &state, &colors);
    assert_eq!(surface, colors.card_fill(kind_hue(TopologyKind::Service)));
    assert_ne!(surface, colors.card);
    let mut ghost = node(TopologyKind::Service, None);
    ghost.look = NodeLook::Ghost;
    assert_eq!(card_surface(&ghost, &state, &colors), colors.card);
    let highlighted = CardState {
        is_highlighted: true,
        ..card_state(false, false)
    };
    assert_eq!(card_surface(&ghost, &highlighted, &colors), colors.muted);
}

#[test]
fn the_selection_glow_is_the_kind_color_and_follows_the_theme() {
    for colors in [CanvasColors::light(), CanvasColors::dark()] {
        let kind = colors.kind(kind_hue(TopologyKind::Pod));
        let glow = selection_glow(kind, colors.glow, 2.);
        assert_eq!(glow.len(), 1);
        assert_eq!((glow[0].color.h, glow[0].color.s), (kind.h, kind.s));
        assert!((glow[0].color.a - colors.glow.alpha * kind.a).abs() < 1e-4);
        assert!((f32::from(glow[0].blur_radius) - 2. * colors.glow.blur).abs() < 1e-4);
        assert!((f32::from(glow[0].spread_radius) - 2. * colors.glow.spread).abs() < 1e-4);
        assert!(!glow[0].inset);
    }
}

#[test]
fn selection_wins_over_tone_borders() {
    let colors = CanvasColors::light();
    let bad = node(TopologyKind::Pod, Some(StatusTone::Bad));
    let kind = colors.kind(kind_hue(TopologyKind::Pod));
    assert_eq!(
        card_border(&bad, &card_state(true, false), &colors),
        (2., kind)
    );
    assert_eq!(
        card_border(&bad, &card_state(false, false), &colors),
        (2., colors.bad)
    );
    let warn = node(TopologyKind::Pod, Some(StatusTone::Warn));
    assert_eq!(
        card_border(&warn, &card_state(false, true), &colors),
        (1., colors.warn)
    );
}

#[test]
fn a_hovered_card_takes_the_kind_border() {
    let colors = CanvasColors::light();
    let plain = node(TopologyKind::Service, None);
    let kind = colors.kind(kind_hue(TopologyKind::Service));
    assert_eq!(
        card_border(&plain, &card_state(false, true), &colors),
        (1., kind)
    );
    assert_eq!(
        card_border(&plain, &card_state(false, false), &colors),
        (1., colors.card_border)
    );
}

#[test]
fn the_card_text_is_large_enough_to_read() {
    // React Flow nodes use about 12 px text; the LOD hides it before it drops under 7 px. The
    // chip, the bar, and the padding leave the name most of the card.
    const NAME_WIDTH: f32 = MIN_NODE_WIDTH - ACCENT_BAR - 3. * CARD_PADDING - CHIP_SIZE;
    const {
        assert!(NAME_SIZE >= 13.);
        assert!(CAPTION_SIZE >= 11.);
        assert!(NAME_SIZE * MIN_TEXT_ZOOM >= 7.);
        assert!(NAME_WIDTH >= 0.6 * MIN_NODE_WIDTH);
        assert!(CHIP_SIZE < NODE_HEIGHT);
    }
}

#[test]
fn the_card_chrome_matches_what_the_layout_reserves() {
    // `CARD_CHROME` of the layout is the bar, the chip, and three paddings.
    const { assert!(ACCENT_BAR + CHIP_SIZE + 3. * CARD_PADDING == 63.) };
}
