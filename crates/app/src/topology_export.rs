//! Export PNG and SVG (W11 pin 5): the graph as arranged on screen, written as SVG text, or
//! rasterized with resvg on the background executor. The SVG holds names, captions, and tones
//! only, never a Secret value. Paths and file names are never traced.

use std::fmt;
use std::path::Path;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Hsla, Rgba, point};
use resvg::{tiny_skia, usvg};

use crate::status_tone::StatusTone;
use crate::topology_canvas::{
    ARROW_HALF_WIDTH, ARROW_LENGTH, ARROW_TIP_GAP, HANDLE_SIZE, LEGEND, SWATCH_WIDTH,
    TITLE_PILL_HEIGHT, TITLE_PILL_LEFT, TITLE_PILL_PADDING, TITLE_PILL_TOP, TITLE_SIZE, arrow_head,
    handle_points, relation_stroke,
};
use crate::topology_card::{
    ACCENT_BAR, CAPTION_SIZE, CARD_PADDING, CHIP_SIZE, CHIP_TEXT_SIZE, NAME_SIZE,
};
use crate::topology_colors::{CanvasColors, EDGE_REST_ALPHA, KindHue, kind_hue};
use crate::topology_graph::{NodeLook, Relation, TopologyGraph, TopologyNode};
use crate::topology_layout::{GraphPoint, GraphRect, NODE_HEIGHT, Placement, TopologyLayout};
use crate::topology_route::EdgeRoute;
use crate::topology_stroke::trim_end;

/// The strip above the graph that holds the title.
pub(crate) const TITLE_STRIP: f32 = 32.;
/// A PNG is rendered at this scale, unless its longest side would pass `MAX_EXPORT_SIDE`.
const EXPORT_SCALE: f32 = 2.;
const MAX_EXPORT_SIDE: f32 = 4_096.;
/// Mono fonts are about this many em wide, which cuts names to the card.
const MONO_EM_WIDTH: f32 = 0.6;
/// Proportional fonts are about this many em wide, which cuts names to the card.
const UI_EM_WIDTH: f32 = 0.55;
const CARD_RADIUS: f32 = 8.;

/// The colors as `#rrggbb` and the font, resolved from the theme once per export. The colors are
/// the ones the screen paints (`CanvasColors`); translucency is written as `*-opacity`.
pub(crate) struct SvgStyle {
    pub(crate) background: String,
    /// The border of a card and of a band.
    pub(crate) border: String,
    /// The surface of a card with no kind color.
    pub(crate) card: String,
    pub(crate) text: String,
    /// The muted text.
    pub(crate) muted: String,
    /// The band fill, and its share.
    pub(crate) band: String,
    pub(crate) band_alpha: f32,
    pub(crate) warn: String,
    pub(crate) bad: String,
    /// The mono font of captions and chips, and the one of names.
    pub(crate) font_family: String,
    pub(crate) ui_font_family: String,
    /// Per `KindHue`, in `KindHue::ALL` order: the chip, the text on it, and the card surface.
    pub(crate) kinds: [String; 8],
    pub(crate) kind_texts: [String; 8],
    pub(crate) card_fills: [String; 8],
    /// Owns, RoutesTo, Mounts.
    pub(crate) relations: [String; 4],
}

pub(crate) fn svg_style(cx: &App) -> SvgStyle {
    let colors = CanvasColors::of(cx);
    SvgStyle {
        background: hex(colors.background),
        border: hex(colors.card_border),
        card: hex(colors.card),
        text: hex(colors.foreground),
        muted: hex(colors.muted_foreground),
        band: hex(colors.muted),
        band_alpha: colors.band_alpha,
        warn: hex(colors.warn),
        bad: hex(colors.bad),
        font_family: cx.theme().mono_font_family.to_string(),
        ui_font_family: cx.theme().font_family.to_string(),
        kinds: KindHue::ALL.map(|hue| hex(colors.kind(hue))),
        kind_texts: KindHue::ALL.map(|hue| hex(colors.kind_text(hue))),
        card_fills: KindHue::ALL.map(|hue| hex(colors.card_fill(hue))),
        relations: RELATIONS.map(|relation| hex(colors.relation(relation))),
    }
}

const RELATIONS: [Relation; 4] = [
    Relation::Owns,
    Relation::RoutesTo,
    Relation::Mounts,
    Relation::Access,
];

/// `#rrggbb` from the channels, each rounded from 0 to 1 onto 0 to 255. The alpha is dropped: the
/// export background is opaque, and translucent fills set `opacity` themselves.
fn hex(color: Hsla) -> String {
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b)
    )
}

impl SvgStyle {
    /// The color of a tone; a neutral one reads as muted text.
    fn tone(&self, tone: StatusTone) -> &str {
        match tone {
            StatusTone::Bad => &self.bad,
            StatusTone::Warn => &self.warn,
            StatusTone::Ok | StatusTone::Info | StatusTone::Done => &self.muted,
        }
    }

    fn relation(&self, relation: Relation) -> &str {
        &self.relations[relation as usize]
    }

    fn kind(&self, hue: KindHue) -> &str {
        &self.kinds[hue.index()]
    }

    fn kind_text(&self, hue: KindHue) -> &str {
        &self.kind_texts[hue.index()]
    }

    fn card_fill(&self, hue: KindHue) -> &str {
        &self.card_fills[hue.index()]
    }
}

/// `&`, `<`, `>`, `"`, and `'` as XML entities.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// How many characters of `font_size` fit in `width`.
fn fit_chars(width: f32, font_size: f32, em_width: f32) -> usize {
    (width / (font_size * em_width)).floor().max(0.) as usize
}

/// `text` cut to `limit` characters, with a trailing ellipsis when it was cut.
fn fitted(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

/// The SVG of the whole graph as laid out now: background, title, bands, edges, nodes, legend.
pub(crate) fn topology_svg(
    graph: &TopologyGraph,
    layout: &TopologyLayout,
    title: &str,
    style: &SvgStyle,
) -> String {
    let extent = layout.extent;
    let (left, top) = (extent.origin.x, extent.origin.y - TITLE_STRIP);
    let (width, height) = (extent.width, extent.height + TITLE_STRIP);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"{left} {top} {width} {height}\" font-family=\"{}, monospace\">\n",
        escape(&style.font_family)
    );
    svg.push_str(&markers(style));
    svg.push_str(&format!(
        "<rect x=\"{left}\" y=\"{top}\" width=\"{width}\" height=\"{height}\" fill=\"{}\"/>\n",
        style.background
    ));
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"{}\">{}</text>\n",
        left + 16.,
        top + 21.,
        style.text,
        escape(title)
    ));
    for band in layout.bands.iter().filter(|band| band.title.is_some()) {
        svg.push_str(&band_svg(
            band.title.as_deref().unwrap_or_default(),
            band.rect,
            style,
        ));
    }
    for (index, edge) in graph.edges.iter().enumerate() {
        svg.push_str(&edge_svg(
            graph,
            &layout.routes[index],
            edge.to,
            edge.relation,
            style,
        ));
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        svg.push_str(&node_svg(index, node, layout.rects[index], style));
    }
    for (index, edge) in graph.edges.iter().enumerate() {
        svg.push_str(&handles_svg(
            graph,
            &layout.routes[index],
            edge.from,
            edge.to,
            style,
        ));
    }
    svg.push_str(&legend_svg(extent, style));
    svg.push_str("</svg>\n");
    svg
}

/// The arrow heads, one per edge color: owns, routes, mounts, access, warn, bad. Each is the triangle of
/// the arrow on screen, with its tip on the target.
fn markers(style: &SvgStyle) -> String {
    let (length, half) = (ARROW_LENGTH, ARROW_HALF_WIDTH);
    let mut defs = String::from("<defs>\n");
    for (name, color) in [
        ("owns", style.relation(Relation::Owns)),
        ("routes", style.relation(Relation::RoutesTo)),
        ("mounts", style.relation(Relation::Mounts)),
        ("access", style.relation(Relation::Access)),
        ("warn", &style.warn),
        ("bad", &style.bad),
    ] {
        defs.push_str(&format!(
            "<marker id=\"arrow-{name}\" markerUnits=\"userSpaceOnUse\" markerWidth=\"{length}\" \
             markerHeight=\"{}\" refX=\"0\" refY=\"{half}\" orient=\"auto\">\
             <path d=\"M0,0 L{length},{half} L0,{} z\" fill=\"{color}\"/></marker>\n",
            half * 2.,
            half * 2.
        ));
    }
    defs.push_str("</defs>\n");
    defs
}

fn band_svg(title: &str, rect: GraphRect, style: &SvgStyle) -> String {
    // The pill of the title, as wide as its text.
    let pill_x = rect.origin.x + TITLE_PILL_LEFT;
    let pill_y = rect.origin.y + TITLE_PILL_TOP;
    let pill_width =
        title.chars().count() as f32 * TITLE_SIZE * MONO_EM_WIDTH + 2. * TITLE_PILL_PADDING;
    format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"{}\" \
         fill-opacity=\"{}\" stroke=\"{}\" stroke-dasharray=\"4 3\"/>\n\
         <rect x=\"{pill_x}\" y=\"{pill_y}\" width=\"{pill_width}\" height=\"{TITLE_PILL_HEIGHT}\" \
         rx=\"{}\" fill=\"{}\" stroke=\"{}\"/>\n\
         <text x=\"{}\" y=\"{}\" font-size=\"{TITLE_SIZE}\" fill=\"{}\">{}</text>\n",
        rect.origin.x,
        rect.origin.y,
        rect.width,
        rect.height,
        style.band,
        style.band_alpha,
        style.border,
        TITLE_PILL_HEIGHT / 2.,
        style.card,
        style.border,
        pill_x + TITLE_PILL_PADDING,
        pill_y + TITLE_PILL_HEIGHT / 2. + TITLE_SIZE * 0.35,
        style.text,
        escape(title)
    )
}

fn edge_svg(
    graph: &TopologyGraph,
    route: &EdgeRoute,
    to: usize,
    relation: Relation,
    style: &SvgStyle,
) -> String {
    let stroke = relation_stroke(relation);
    // A tone is drawn at full strength; a relation color at the rest alpha of the screen.
    let (color, marker, opacity) = match (graph.ghost_tone(to), relation) {
        (Some(StatusTone::Bad), _) => (style.bad.as_str(), "bad", 1.),
        (Some(_), _) => (style.warn.as_str(), "warn", 1.),
        (None, Relation::Owns) => (style.relation(relation), "owns", EDGE_REST_ALPHA),
        (None, Relation::RoutesTo) => (style.relation(relation), "routes", EDGE_REST_ALPHA),
        (None, Relation::Mounts) => (style.relation(relation), "mounts", EDGE_REST_ALPHA),
        (None, Relation::Access) => (style.relation(relation), "access", EDGE_REST_ALPHA),
    };
    let dash = stroke
        .dash
        .map(|(on, off)| format!(" stroke-dasharray=\"{on} {off}\""))
        .unwrap_or_default();
    // The line stops at the base of the arrow, as on screen; the marker draws the arrow from there.
    let mut points: Vec<_> = route.points.iter().map(|at| point(at.x, at.y)).collect();
    trim_end(&mut points, ARROW_LENGTH + ARROW_TIP_GAP);
    let mut path = String::new();
    for (index, at) in points.iter().enumerate() {
        let command = if index == 0 { 'M' } else { 'L' };
        path.push_str(&format!("{command}{} {} ", at.x, at.y));
    }
    format!(
        "<path d=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-opacity=\"{opacity}\" \
         stroke-width=\"{}\"{dash} marker-end=\"url(#arrow-{marker})\"/>\n",
        path.trim_end(),
        stroke.width
    )
}

/// The two handle dots of an edge: the kind color of the node each sits on, in a ring of the
/// background, as the handle layer paints them.
fn handles_svg(
    graph: &TopologyGraph,
    route: &EdgeRoute,
    from: usize,
    to: usize,
    style: &SvgStyle,
) -> String {
    let [start, end] = handle_points(route);
    let mut svg = String::new();
    for (at, node) in [(start, from), (end, to)] {
        svg.push_str(&format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>\n",
            at.x,
            at.y,
            HANDLE_SIZE / 2.,
            style.kind(kind_hue(graph.nodes[node].kind)),
            style.background
        ));
    }
    svg
}

fn node_svg(index: usize, node: &TopologyNode, rect: GraphRect, style: &SvgStyle) -> String {
    let node_width = rect.width;
    let (x, y) = (rect.origin.x, rect.origin.y);
    let hue = kind_hue(node.kind);
    let (fill, stroke_color, stroke_width, dash, opacity) = match node.look {
        NodeLook::Plain => {
            let (width, color) = match node.tone {
                Some(StatusTone::Bad) => (2., &style.bad),
                Some(StatusTone::Warn) => (1., &style.warn),
                _ => (1., &style.border),
            };
            let opacity = if node.kind.placement() == Placement::ConfigRow {
                0.85
            } else {
                1.
            };
            (style.card_fill(hue), color, width, "", opacity)
        }
        NodeLook::Ghost => {
            let color = node.tone.map_or(&style.muted, |tone| match tone {
                StatusTone::Bad => &style.bad,
                _ => &style.warn,
            });
            (
                style.card.as_str(),
                color,
                1.5,
                " stroke-dasharray=\"4 3\"",
                1.,
            )
        }
        NodeLook::Unchecked => (
            style.card.as_str(),
            &style.muted,
            1.,
            " stroke-dasharray=\"4 3\"",
            0.7,
        ),
    };
    let mut svg = format!(
        "<g opacity=\"{opacity}\">\n<clipPath id=\"card-{index}\"><rect x=\"{x}\" y=\"{y}\" width=\"{node_width}\" \
         height=\"{NODE_HEIGHT}\" rx=\"{CARD_RADIUS}\"/></clipPath>\n\
         <rect x=\"{x}\" y=\"{y}\" width=\"{node_width}\" height=\"{NODE_HEIGHT}\" \
         rx=\"{CARD_RADIUS}\" fill=\"{fill}\" stroke=\"{stroke_color}\" stroke-width=\"{stroke_width}\"{dash}/>\n"
    );
    let has_chip = node.look == NodeLook::Plain;
    let text_x = if has_chip {
        let chip_x = x + ACCENT_BAR + CARD_PADDING;
        let chip_y = y + (NODE_HEIGHT - CHIP_SIZE) / 2.;
        svg.push_str(&format!(
            "<rect x=\"{x}\" y=\"{y}\" width=\"{ACCENT_BAR}\" height=\"{NODE_HEIGHT}\" fill=\"{}\" \
             clip-path=\"url(#card-{index})\"/>\n\
             <rect x=\"{chip_x}\" y=\"{chip_y}\" width=\"{CHIP_SIZE}\" height=\"{CHIP_SIZE}\" rx=\"7\" fill=\"{}\"/>\n\
             <text x=\"{}\" y=\"{}\" font-size=\"{CHIP_TEXT_SIZE}\" font-weight=\"700\" text-anchor=\"middle\" \
             fill=\"{}\">{}</text>\n",
            style.kind(hue),
            style.kind(hue),
            chip_x + CHIP_SIZE / 2.,
            y + NODE_HEIGHT / 2. + CHIP_TEXT_SIZE * 0.35,
            style.kind_text(hue),
            escape(node.kind.badge())
        ));
        chip_x + CHIP_SIZE + CARD_PADDING
    } else {
        x + 2. * CARD_PADDING
    };
    let available = x + node_width - CARD_PADDING - text_x;
    let caption_color = match node.tone {
        Some(tone @ (StatusTone::Bad | StatusTone::Warn)) => style.tone(tone),
        _ => &style.muted,
    };
    svg.push_str(&format!(
        "<text x=\"{text_x}\" y=\"{}\" font-size=\"{CAPTION_SIZE}\" fill=\"{caption_color}\">{}</text>\n\
         <text x=\"{text_x}\" y=\"{}\" font-size=\"{NAME_SIZE}\" font-family=\"{}, sans-serif\" \
         fill=\"{}\">{}</text>\n</g>\n",
        y + 25.,
        escape(&fitted(
            &node.caption.to_uppercase(),
            fit_chars(available, CAPTION_SIZE, MONO_EM_WIDTH)
        )),
        y + 42.,
        escape(&style.ui_font_family),
        style.text,
        escape(&fitted(
            &node.name,
            fit_chars(available, NAME_SIZE, UI_EM_WIDTH)
        ))
    ));
    svg
}

/// The legend at the bottom right: a swatch drawn like a real edge, and its meaning, for each
/// relation.
fn legend_svg(extent: GraphRect, style: &SvgStyle) -> String {
    let mut svg = String::new();
    let y = extent.bottom() - 16.;
    // Each entry is as wide as its swatch, its text, and a gap; the row ends at the right margin.
    let widths: Vec<f32> = LEGEND
        .iter()
        .map(|(_, text)| {
            SWATCH_WIDTH + 8. + text.chars().count() as f32 * 11. * MONO_EM_WIDTH + 20.
        })
        .collect();
    let mut x = extent.right() - 24. - widths.iter().sum::<f32>();
    for ((relation, text), width) in LEGEND.iter().zip(&widths) {
        let stroke = relation_stroke(*relation);
        let dash = stroke
            .dash
            .map(|(on, off)| format!(" stroke-dasharray=\"{on} {off}\""))
            .unwrap_or_default();
        let color = style.relation(*relation);
        let base = x + SWATCH_WIDTH - ARROW_LENGTH - ARROW_TIP_GAP;
        let route = EdgeRoute {
            points: vec![
                GraphPoint { x, y },
                GraphPoint {
                    x: x + SWATCH_WIDTH,
                    y,
                },
            ],
        };
        let head = arrow_head(&route, ARROW_LENGTH, ARROW_HALF_WIDTH);
        svg.push_str(&format!(
            "<path d=\"M{x} {y} L{base} {y}\" fill=\"none\" stroke=\"{color}\" \
             stroke-opacity=\"{EDGE_REST_ALPHA}\" stroke-width=\"{}\"{dash}/>\n\
             <polygon points=\"{} {} {} {} {} {}\" fill=\"{color}\"/>\n\
             <text x=\"{}\" y=\"{}\" font-size=\"11\" fill=\"{}\">{}</text>\n",
            stroke.width,
            head[0].x,
            head[0].y,
            head[1].x,
            head[1].y,
            head[2].x,
            head[2].y,
            x + SWATCH_WIDTH + 8.,
            y + 4.,
            style.muted,
            escape(text)
        ));
        x += width;
    }
    svg
}

#[derive(Debug)]
pub(crate) enum ExportError {
    /// The SVG did not parse.
    Render(String),
    /// The image would have no pixels.
    EmptyImage,
    Encode(String),
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render(cause) => write!(formatter, "could not render the topology: {cause}"),
            Self::EmptyImage => write!(formatter, "the image is empty"),
            Self::Encode(cause) => write!(formatter, "could not encode the image: {cause}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// The scale of an export of `width` by `height`: 2, or less when the longest side would pass
/// `MAX_EXPORT_SIDE` (a 64 MB pixmap at most).
pub(crate) fn export_scale(width: f32, height: f32) -> f32 {
    EXPORT_SCALE.min(MAX_EXPORT_SIDE / width.max(height).max(1.))
}

/// SVG text as a PNG. It loads the system fonts (100 to 500 ms), so it never runs on the main
/// thread.
pub(crate) fn render_png(svg: &str) -> Result<Vec<u8>, ExportError> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|error| ExportError::Render(error.to_string()))?;
    let size = tree.size();
    let scale = export_scale(size.width(), size.height());
    let width = (size.width() * scale).ceil() as u32;
    let height = (size.height() * scale).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(ExportError::EmptyImage)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|error| ExportError::Encode(error.to_string()))
}

/// What the save flow carries to the background executor: the SVG of the graph.
pub(crate) struct TopologyExport {
    pub(crate) svg: String,
}

/// Writes the export to `path`: the SVG text for a `.svg` path (ASCII case-insensitive), else a PNG.
pub(crate) fn write_export(path: &Path, export: TopologyExport) -> std::io::Result<()> {
    if is_svg_path(path) {
        return std::fs::write(path, export.svg);
    }
    let png = render_png(&export.svg).map_err(std::io::Error::other)?;
    std::fs::write(path, png)
}

fn is_svg_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

#[cfg(test)]
#[path = "topology_export_tests.rs"]
mod topology_export_tests;
