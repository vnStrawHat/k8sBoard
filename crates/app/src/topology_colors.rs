//! The Topology colors (0022b step 2): one table from node kind to theme token, the relation
//! colors, and the theme colors the canvas paints with, resolved once per frame. Every color comes
//! from a theme token; red and yellow stay reserved for the Bad and Warn tones.

use gpui_kit::component::{ActiveTheme as _, Colorize as _, ThemeColor};
use gpui_kit::{App, Hsla};

use crate::status_tone::{StatusTone, contrast, tone_color};
use crate::topology_graph::{Relation, TopologyKind};

/// The alpha of an edge at rest (React Flow draws its edges light too).
pub(crate) const EDGE_REST_ALPHA: f32 = 0.8;
/// The share of the kind color in a box card of a zoomed-out graph.
pub(crate) const KIND_BOX_TINT: f32 = 0.4;

/// How the card surface differs from the canvas, per theme. A light card is plain white; a dark
/// card is raised toward `muted`, with a border pulled toward `muted_foreground` so it shows.
const CARD_RAISE_LIGHT: f32 = 0.;
const CARD_RAISE_DARK: f32 = 0.85;
const CARD_BORDER_PULL_LIGHT: f32 = 0.;
const CARD_BORDER_PULL_DARK: f32 = 0.45;
/// The share of the kind color in the surface of a card.
const CARD_KIND_TINT_LIGHT: f32 = 0.05;
const CARD_KIND_TINT_DARK: f32 = 0.1;
/// The share of the muted fill in a band, on screen and in the export.
const BAND_ALPHA_LIGHT: f32 = 0.6;
const BAND_ALPHA_DARK: f32 = 0.5;

/// The ring of kind color around a selected card, in graph units. On a dark canvas a glow needs
/// more strength and reach to show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SelectionGlow {
    pub(crate) alpha: f32,
    pub(crate) blur: f32,
    pub(crate) spread: f32,
}

const GLOW_LIGHT: SelectionGlow = SelectionGlow {
    alpha: 0.45,
    blur: 12.,
    spread: 2.,
};
const GLOW_DARK: SelectionGlow = SelectionGlow {
    alpha: 0.6,
    blur: 16.,
    spread: 3.,
};

/// The color family of a node kind. Related kinds share one, so a graph reads by color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KindHue {
    Ingress,
    Service,
    Workload,
    Pod,
    ConfigMap,
    Secret,
    Claim,
    /// ServiceAccount, bindings, and roles (the RBAC layer).
    Access,
}

impl KindHue {
    pub(crate) const ALL: [Self; 8] = [
        Self::Ingress,
        Self::Service,
        Self::Workload,
        Self::Pod,
        Self::ConfigMap,
        Self::Secret,
        Self::Claim,
        Self::Access,
    ];

    /// The place of the hue in `ALL`, which the per-frame color arrays use.
    pub(crate) fn index(self) -> usize {
        self as usize
    }
}

pub(crate) fn kind_hue(kind: TopologyKind) -> KindHue {
    match kind {
        TopologyKind::Ingress => KindHue::Ingress,
        TopologyKind::Service => KindHue::Service,
        TopologyKind::Deployment
        | TopologyKind::StatefulSet
        | TopologyKind::DaemonSet
        | TopologyKind::ReplicaSet
        | TopologyKind::HorizontalPodAutoscaler => KindHue::Workload,
        TopologyKind::Pod => KindHue::Pod,
        TopologyKind::ConfigMap => KindHue::ConfigMap,
        TopologyKind::Secret => KindHue::Secret,
        TopologyKind::PersistentVolumeClaim => KindHue::Claim,
        TopologyKind::ServiceAccount
        | TopologyKind::RoleBinding
        | TopologyKind::ClusterRoleBinding
        | TopologyKind::Role
        | TopologyKind::ClusterRole => KindHue::Access,
    }
}

/// The theme token of a hue. Red and yellow are not here: they are the Bad and Warn tones.
fn hue_color(theme: &ThemeColor, hue: KindHue) -> Hsla {
    match hue {
        KindHue::Ingress => theme.magenta,
        KindHue::Service => theme.cyan,
        KindHue::Workload => theme.blue,
        KindHue::Pod => theme.blue_light,
        KindHue::ConfigMap => theme.green,
        KindHue::Secret => theme.green_light,
        KindHue::Claim => theme.magenta_light,
        KindHue::Access => theme.cyan_light,
    }
}

/// The theme token of a relation's edge.
fn relation_color(theme: &ThemeColor, relation: Relation) -> Hsla {
    match relation {
        Relation::Owns => theme.blue,
        Relation::RoutesTo => theme.cyan,
        Relation::Mounts => theme.green,
        Relation::Access => theme.cyan_light,
        Relation::Calls => theme.cyan,
    }
}

const RELATIONS: [Relation; 5] = [
    Relation::Owns,
    Relation::RoutesTo,
    Relation::Mounts,
    Relation::Access,
    Relation::Calls,
];

/// The theme colors the painting needs, resolved once per frame.
#[derive(Clone, Copy)]
pub(crate) struct CanvasColors {
    pub(crate) background: Hsla,
    pub(crate) foreground: Hsla,
    /// The surface of a card with no kind color (ghosts, unchecked).
    pub(crate) card: Hsla,
    /// The border of a card and of a band.
    pub(crate) card_border: Hsla,
    pub(crate) muted: Hsla,
    pub(crate) muted_foreground: Hsla,
    pub(crate) ring: Hsla,
    pub(crate) warn: Hsla,
    pub(crate) bad: Hsla,
    /// The share of the muted fill in a band.
    pub(crate) band_alpha: f32,
    pub(crate) glow: SelectionGlow,
    kinds: [Hsla; 8],
    /// The text on a solid kind chip: whichever of the background and the foreground contrasts more.
    kind_texts: [Hsla; 8],
    /// The surface of a card of each kind: the card with a little of the kind color.
    card_fills: [Hsla; 8],
    relations: [Hsla; 5],
}

impl CanvasColors {
    pub(crate) fn of(cx: &App) -> Self {
        let theme = cx.theme();
        Self::build(
            theme,
            theme.is_dark(),
            tone_color(StatusTone::Warn, cx),
            tone_color(StatusTone::Bad, cx),
        )
    }

    fn build(theme: &ThemeColor, is_dark: bool, warn: Hsla, bad: Hsla) -> Self {
        let pick = |light: f32, dark: f32| if is_dark { dark } else { light };
        let card = tint(
            theme.background,
            theme.muted,
            pick(CARD_RAISE_LIGHT, CARD_RAISE_DARK),
        );
        let kinds = KindHue::ALL.map(|hue| hue_color(theme, hue));
        let kind_tint = pick(CARD_KIND_TINT_LIGHT, CARD_KIND_TINT_DARK);
        Self {
            background: theme.background,
            foreground: theme.foreground,
            card,
            card_border: tint(
                theme.border,
                theme.muted_foreground,
                pick(CARD_BORDER_PULL_LIGHT, CARD_BORDER_PULL_DARK),
            ),
            muted: theme.muted,
            muted_foreground: theme.muted_foreground,
            ring: theme.ring,
            warn,
            bad,
            band_alpha: pick(BAND_ALPHA_LIGHT, BAND_ALPHA_DARK),
            glow: if is_dark { GLOW_DARK } else { GLOW_LIGHT },
            kinds,
            kind_texts: kinds.map(|kind| readable_on(kind, theme.background, theme.foreground)),
            card_fills: kinds.map(|kind| tint(card, kind, kind_tint)),
            relations: RELATIONS.map(|relation| relation_color(theme, relation)),
        }
    }

    pub(crate) fn tone(&self, tone: StatusTone) -> Hsla {
        match tone {
            StatusTone::Bad => self.bad,
            StatusTone::Warn => self.warn,
            StatusTone::Ok | StatusTone::Info | StatusTone::Done => self.muted_foreground,
        }
    }

    pub(crate) fn kind(&self, hue: KindHue) -> Hsla {
        self.kinds[hue.index()]
    }

    /// The text on a solid chip of the hue.
    pub(crate) fn kind_text(&self, hue: KindHue) -> Hsla {
        self.kind_texts[hue.index()]
    }

    /// The surface of a card of the hue.
    pub(crate) fn card_fill(&self, hue: KindHue) -> Hsla {
        self.card_fills[hue.index()]
    }

    /// The text on a chip of any fill (a tone color at overview zoom).
    pub(crate) fn text_on(&self, fill: Hsla) -> Hsla {
        readable_on(fill, self.background, self.foreground)
    }

    /// The colors of the default light theme, with a fixed warn and bad, for tests that need no app.
    #[cfg(test)]
    pub(crate) fn light() -> Self {
        let theme = ThemeColor::light();
        Self::build(&theme, false, theme.yellow, theme.red)
    }

    /// The colors of the default dark theme, for tests that need no app.
    #[cfg(test)]
    pub(crate) fn dark() -> Self {
        let theme = ThemeColor::dark();
        Self::build(&theme, true, theme.yellow, theme.red)
    }

    /// The color of the edge of a relation, before its alpha.
    pub(crate) fn relation(&self, relation: Relation) -> Hsla {
        self.relations[relation as usize]
    }
}

/// The color of an edge at rest: an edge into a ghost takes the tone of its check, any other the
/// color of its relation at `EDGE_REST_ALPHA`.
pub(crate) fn edge_color(
    colors: &CanvasColors,
    relation: Relation,
    ghost_tone: Option<StatusTone>,
) -> Hsla {
    match ghost_tone {
        Some(tone) => colors.tone(tone),
        None => colors.relation(relation).opacity(EDGE_REST_ALPHA),
    }
}

/// `base` with `share` (0 to 1) of `toward` mixed in, in Oklab.
fn tint(base: Hsla, toward: Hsla, share: f32) -> Hsla {
    if share <= 0. {
        return base;
    }
    // `mix_oklab` takes the share of its receiver.
    toward.mix_oklab(base, share)
}

/// Whichever of `first` and `second` has the higher contrast ratio (WCAG) against `fill`.
fn readable_on(fill: Hsla, first: Hsla, second: Hsla) -> Hsla {
    if contrast(fill, first) >= contrast(fill, second) {
        first
    } else {
        second
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_KINDS: [TopologyKind; 16] = [
        TopologyKind::Ingress,
        TopologyKind::HorizontalPodAutoscaler,
        TopologyKind::Service,
        TopologyKind::Deployment,
        TopologyKind::StatefulSet,
        TopologyKind::DaemonSet,
        TopologyKind::ReplicaSet,
        TopologyKind::Pod,
        TopologyKind::ConfigMap,
        TopologyKind::Secret,
        TopologyKind::PersistentVolumeClaim,
        TopologyKind::ServiceAccount,
        TopologyKind::RoleBinding,
        TopologyKind::ClusterRoleBinding,
        TopologyKind::Role,
        TopologyKind::ClusterRole,
    ];

    #[test]
    fn every_kind_has_one_hue() {
        let expected = [
            (TopologyKind::Ingress, KindHue::Ingress),
            (TopologyKind::HorizontalPodAutoscaler, KindHue::Workload),
            (TopologyKind::Service, KindHue::Service),
            (TopologyKind::Deployment, KindHue::Workload),
            (TopologyKind::StatefulSet, KindHue::Workload),
            (TopologyKind::DaemonSet, KindHue::Workload),
            (TopologyKind::ReplicaSet, KindHue::Workload),
            (TopologyKind::Pod, KindHue::Pod),
            (TopologyKind::ConfigMap, KindHue::ConfigMap),
            (TopologyKind::Secret, KindHue::Secret),
            (TopologyKind::PersistentVolumeClaim, KindHue::Claim),
            (TopologyKind::ServiceAccount, KindHue::Access),
            (TopologyKind::RoleBinding, KindHue::Access),
            (TopologyKind::ClusterRoleBinding, KindHue::Access),
            (TopologyKind::Role, KindHue::Access),
            (TopologyKind::ClusterRole, KindHue::Access),
        ];
        assert_eq!(expected.len(), ALL_KINDS.len());
        for (kind, hue) in expected {
            assert_eq!(kind_hue(kind), hue, "{kind:?}");
        }
    }

    #[test]
    fn workload_kinds_share_the_workload_hue() {
        for kind in [
            TopologyKind::Deployment,
            TopologyKind::StatefulSet,
            TopologyKind::DaemonSet,
            TopologyKind::ReplicaSet,
            TopologyKind::HorizontalPodAutoscaler,
        ] {
            assert_eq!(kind_hue(kind), KindHue::Workload);
        }
    }

    #[test]
    fn pods_and_groups_use_the_pod_hue() {
        // A pod group is a node of kind `Pod`.
        assert_eq!(kind_hue(TopologyKind::Pod), KindHue::Pod);
        assert_ne!(KindHue::Pod, KindHue::Workload);
    }

    #[test]
    fn hue_indexes_follow_the_table_order() {
        for (position, hue) in KindHue::ALL.into_iter().enumerate() {
            assert_eq!(hue.index(), position);
        }
    }

    #[test]
    fn kind_hues_avoid_the_tone_tokens() {
        for theme in [ThemeColor::light(), ThemeColor::dark()] {
            let reserved = [theme.red, theme.red_light, theme.yellow, theme.yellow_light];
            for hue in KindHue::ALL {
                assert!(!reserved.contains(&hue_color(&theme, hue)), "{hue:?}");
            }
            for relation in RELATIONS {
                assert!(!reserved.contains(&relation_color(&theme, relation)));
            }
        }
    }

    fn colors() -> CanvasColors {
        CanvasColors::light()
    }

    #[test]
    fn edge_color_follows_the_relation() {
        let theme = ThemeColor::light();
        assert_eq!(relation_color(&theme, Relation::Owns), theme.blue);
        assert_eq!(relation_color(&theme, Relation::RoutesTo), theme.cyan);
        assert_eq!(relation_color(&theme, Relation::Mounts), theme.green);
        let colors = colors();
        let edge = edge_color(&colors, Relation::RoutesTo, None);
        assert_eq!(edge, theme.cyan.opacity(EDGE_REST_ALPHA));
    }

    #[test]
    fn a_ghost_edge_takes_its_check_tone() {
        let colors = colors();
        assert_eq!(
            edge_color(&colors, Relation::Owns, Some(StatusTone::Bad)),
            colors.bad
        );
        assert_eq!(
            edge_color(&colors, Relation::Mounts, Some(StatusTone::Warn)),
            colors.warn
        );
    }

    #[test]
    fn access_relation_is_cyan_light() {
        for theme in [ThemeColor::light(), ThemeColor::dark()] {
            assert_eq!(relation_color(&theme, Relation::Access), theme.cyan_light);
            assert_eq!(hue_color(&theme, KindHue::Access), theme.cyan_light);
        }
    }

    #[test]
    fn relations_index_in_enum_order() {
        for relation in RELATIONS {
            assert_eq!(RELATIONS[relation as usize], relation);
        }
    }

    #[test]
    fn chip_text_contrasts_with_every_kind_color() {
        for colors in [CanvasColors::light(), CanvasColors::dark()] {
            for hue in KindHue::ALL {
                let ratio = contrast(colors.kind(hue), colors.kind_text(hue));
                assert!(ratio >= 4.5, "{hue:?}: {ratio}");
            }
        }
    }

    #[test]
    fn readable_on_picks_the_higher_contrast() {
        let colors = CanvasColors::light();
        // White on a dark fill, dark on a light one.
        assert_eq!(
            readable_on(colors.foreground, colors.background, colors.foreground),
            colors.background
        );
        assert_eq!(
            readable_on(colors.background, colors.background, colors.foreground),
            colors.foreground
        );
        assert!(contrast(colors.background, colors.foreground) > 15.);
    }

    #[test]
    fn a_light_card_is_plain_and_a_dark_card_is_raised() {
        let light = CanvasColors::light();
        assert_eq!(light.card, light.background);
        let dark = CanvasColors::dark();
        assert_ne!(dark.card, dark.background);
        assert!(dark.card.l > dark.background.l);
        // The dark border shows against the card.
        assert!(dark.card_border.l > dark.card.l + 0.1);
    }

    #[test]
    fn kinds_tint_the_card_surface_more_in_the_dark() {
        let hue = KindHue::Service;
        let shift = |colors: &CanvasColors| (colors.card_fill(hue).l - colors.card.l).abs();
        let (light, dark) = (CanvasColors::light(), CanvasColors::dark());
        assert_ne!(light.card_fill(hue), light.card);
        assert!(shift(&dark) > 0.);
        assert!(dark.band_alpha > 0. && light.band_alpha > 0.);
    }

    #[test]
    fn the_dark_glow_is_stronger_and_wider() {
        let (light, dark) = (CanvasColors::light().glow, CanvasColors::dark().glow);
        assert!(dark.alpha > light.alpha);
        assert!(dark.blur > light.blur && dark.spread >= light.spread);
    }
}
