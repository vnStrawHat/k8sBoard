//! Export PNG and SVG (W11 pin 5): the graph as arranged on screen, written as SVG text, or
//! rasterized with resvg on the background executor. The SVG holds names, captions, and tones
//! only, never a Secret value. Paths and file names are never traced.

use std::fmt;
use std::path::Path;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Hsla, Rgba};
use resvg::{tiny_skia, usvg};

use crate::status_tone::{StatusTone, tone_color};
use crate::topology_canvas::{LEGEND, edge_curve, relation_stroke};
use crate::topology_graph::{NodeLook, Relation, TopologyGraph, TopologyNode};
use crate::topology_layout::{GraphRect, NODE_HEIGHT, NODE_WIDTH, Placement, TopologyLayout};

/// The strip above the graph that holds the title.
pub(crate) const TITLE_STRIP: f32 = 32.;
/// A PNG is rendered at this scale, unless its longest side would pass `MAX_EXPORT_SIDE`.
const EXPORT_SCALE: f32 = 2.;
const MAX_EXPORT_SIDE: f32 = 4_096.;
/// Mono fonts are about this many em wide, which cuts names to the card.
const MONO_EM_WIDTH: f32 = 0.6;
const ARROW_SIZE: f32 = 8.;
const BADGE_WIDTH: f32 = 24.;
const CARD_RADIUS: f32 = 8.;

/// The colors as `#rrggbb` and the font, resolved from the theme once per export.
pub(crate) struct SvgStyle {
    pub(crate) background: String,
    pub(crate) border: String,
    pub(crate) text: String,
    /// The muted text and the owns and mounts edges.
    pub(crate) muted: String,
    /// The badge fill.
    pub(crate) badge: String,
    pub(crate) accent: String,
    pub(crate) warn: String,
    pub(crate) bad: String,
    pub(crate) font_family: String,
}

pub(crate) fn svg_style(cx: &App) -> SvgStyle {
    let theme = cx.theme();
    SvgStyle {
        background: hex(theme.background),
        border: hex(theme.border),
        text: hex(theme.foreground),
        muted: hex(theme.muted_foreground),
        badge: hex(theme.muted),
        accent: hex(theme.ring),
        warn: hex(tone_color(StatusTone::Warn, cx)),
        bad: hex(tone_color(StatusTone::Bad, cx)),
        font_family: theme.mono_font_family.to_string(),
    }
}

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

    fn edge(&self, relation: Relation) -> &str {
        match relation {
            Relation::RoutesTo => &self.accent,
            Relation::Owns | Relation::Mounts => &self.muted,
        }
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
fn fit_chars(width: f32, font_size: f32) -> usize {
    (width / (font_size * MONO_EM_WIDTH)).floor().max(0.) as usize
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
    for edge in &graph.edges {
        svg.push_str(&edge_svg(
            graph,
            layout,
            edge.from,
            edge.to,
            edge.relation,
            style,
        ));
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        svg.push_str(&node_svg(node, layout.rects[index], style));
    }
    svg.push_str(&legend_svg(extent, style));
    svg.push_str("</svg>\n");
    svg
}

/// The four arrow heads: muted, accent, warn, bad.
fn markers(style: &SvgStyle) -> String {
    let mut defs = String::from("<defs>\n");
    for (name, color) in [
        ("muted", &style.muted),
        ("accent", &style.accent),
        ("warn", &style.warn),
        ("bad", &style.bad),
    ] {
        defs.push_str(&format!(
            "<marker id=\"arrow-{name}\" markerUnits=\"userSpaceOnUse\" markerWidth=\"{ARROW_SIZE}\" \
             markerHeight=\"{ARROW_SIZE}\" refX=\"{ARROW_SIZE}\" refY=\"{}\" orient=\"auto\">\
             <path d=\"M0,0 L{ARROW_SIZE},{} L0,{ARROW_SIZE} z\" fill=\"{color}\"/></marker>\n",
            ARROW_SIZE / 2.,
            ARROW_SIZE / 2.
        ));
    }
    defs.push_str("</defs>\n");
    defs
}

fn band_svg(title: &str, rect: GraphRect, style: &SvgStyle) -> String {
    format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"none\" stroke=\"{}\" \
         stroke-dasharray=\"4 3\"/>\n<text x=\"{}\" y=\"{}\" font-size=\"10\" fill=\"{}\">{}</text>\n",
        rect.origin.x,
        rect.origin.y,
        rect.width,
        rect.height,
        style.border,
        rect.origin.x + 10.,
        rect.origin.y + 14.,
        style.muted,
        escape(title)
    )
}

fn edge_svg(
    graph: &TopologyGraph,
    layout: &TopologyLayout,
    from: usize,
    to: usize,
    relation: Relation,
    style: &SvgStyle,
) -> String {
    let curve = edge_curve(layout.rects[from], layout.rects[to], relation);
    let stroke = relation_stroke(relation);
    let target = &graph.nodes[to];
    let ghost_tone = (target.look == NodeLook::Ghost)
        .then(|| {
            graph
                .checks
                .iter()
                .find(|check| check.node == target.id)
                .map(|check| check.tone)
        })
        .flatten();
    let (color, marker) = match ghost_tone {
        Some(StatusTone::Bad) => (&style.bad, "bad"),
        Some(_) => (&style.warn, "warn"),
        None if relation == Relation::RoutesTo => (&style.accent, "accent"),
        None => (&style.muted, "muted"),
    };
    let dash = stroke
        .dash
        .map(|(on, off)| format!(" stroke-dasharray=\"{on} {off}\""))
        .unwrap_or_default();
    format!(
        "<path d=\"M{} {} C{} {} {} {} {} {}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\"{dash} \
         marker-end=\"url(#arrow-{marker})\"/>\n",
        curve.start.x,
        curve.start.y,
        curve.ctrl1.x,
        curve.ctrl1.y,
        curve.ctrl2.x,
        curve.ctrl2.y,
        curve.end.x,
        curve.end.y,
        stroke.width
    )
}

fn node_svg(node: &TopologyNode, rect: GraphRect, style: &SvgStyle) -> String {
    let (x, y) = (rect.origin.x, rect.origin.y);
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
            (style.background.as_str(), color, width, "", opacity)
        }
        NodeLook::Ghost => {
            let color = node.tone.map_or(&style.muted, |tone| match tone {
                StatusTone::Bad => &style.bad,
                _ => &style.warn,
            });
            ("none", color, 1.5, " stroke-dasharray=\"4 3\"", 1.)
        }
        NodeLook::Unchecked => ("none", &style.muted, 1., " stroke-dasharray=\"4 3\"", 0.7),
    };
    let mut svg = format!(
        "<g opacity=\"{opacity}\">\n<rect x=\"{x}\" y=\"{y}\" width=\"{NODE_WIDTH}\" height=\"{NODE_HEIGHT}\" \
         rx=\"{CARD_RADIUS}\" fill=\"{fill}\" stroke=\"{stroke_color}\" stroke-width=\"{stroke_width}\"{dash}/>\n"
    );
    let has_badge = node.look == NodeLook::Plain;
    let text_x = if has_badge {
        svg.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"7\" fill=\"{}\"/>\n\
             <text x=\"{}\" y=\"{}\" font-size=\"9\" text-anchor=\"middle\" fill=\"{}\">{}</text>\n",
            x + 1.,
            y + 1.,
            BADGE_WIDTH - 1.,
            NODE_HEIGHT - 2.,
            style.badge,
            x + BADGE_WIDTH / 2.,
            y + NODE_HEIGHT / 2. + 3.,
            style.muted,
            escape(node.kind.badge())
        ));
        x + BADGE_WIDTH + 8.
    } else {
        x + 10.
    };
    let available = x + NODE_WIDTH - 8. - text_x;
    let caption_color = match node.tone {
        Some(tone @ (StatusTone::Bad | StatusTone::Warn)) => style.tone(tone),
        _ => &style.muted,
    };
    svg.push_str(&format!(
        "<text x=\"{text_x}\" y=\"{}\" font-size=\"9\" fill=\"{caption_color}\">{}</text>\n\
         <text x=\"{text_x}\" y=\"{}\" font-size=\"10.8\" fill=\"{}\">{}</text>\n</g>\n",
        y + 22.,
        escape(&fitted(
            &node.caption.to_uppercase(),
            fit_chars(available, 9.)
        )),
        y + 38.,
        style.text,
        escape(&fitted(&node.name, fit_chars(available, 10.8)))
    ));
    svg
}

/// The legend at the bottom right, in the same glyph text as on screen.
fn legend_svg(extent: GraphRect, style: &SvgStyle) -> String {
    let entries: Vec<String> = LEGEND
        .iter()
        .map(|(relation, glyph, text)| {
            format!(
                "<tspan fill=\"{}\">{glyph}</tspan><tspan fill=\"{}\"> {text}   </tspan>",
                style.edge(*relation),
                style.muted
            )
        })
        .collect();
    format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"10\">{}</text>\n",
        extent.right() - 220.,
        extent.bottom() - 10.,
        entries.concat()
    )
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
