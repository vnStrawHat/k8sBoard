//! The terminal's element (spec 0036): one GPUI `canvas` that sizes the grid to its parent,
//! queues one resize per change, and paints the snapshot of a `TerminalSession`. Fonts and colors
//! come from the kit theme; nothing here names a color.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use cluster::{GridSize, ShellInput};
use futures::channel::mpsc::UnboundedSender;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, BorderStyle, Bounds, Font, FontStyle, FontWeight, Hsla, IntoElement, Pixels, Point, Rgba,
    SharedString, Size, StrikethroughStyle, Styled as _, TextAlign, TextRun, UnderlineStyle,
    Window, canvas, fill, outline, point, px, size,
};
use oneterm_vt::search::SearchMatch;
use oneterm_vt::{
    Attrs, CellWidth, Color, CursorShape, Palette, Rgb, RowId, SelectionRange, SnapshotContent,
    SnapshotRow, SnapshotState, Style,
};

use crate::terminal_session::{TerminalSession, terminal_palette};

/// The cell is this much taller than the font size, which leaves room for descenders.
const LINE_HEIGHT_RATIO: f32 = 1.3;
/// A guess for a font that cannot report the advance of `M`: the usual monospace proportion.
const FALLBACK_ADVANCE_RATIO: f32 = 0.6;
/// The block cursor lets the glyph under it show through.
const BLOCK_CURSOR_ALPHA: f32 = 0.6;
/// Thickness of the beam and underline cursors and of text decorations.
const LINE_THICKNESS: f32 = 1.;
const BAR_CURSOR_THICKNESS: f32 = 2.;
/// The smallest grid the engine is given: one row, and two columns so a wide glyph fits.
const MIN_COLS: u16 = 2;
const MIN_ROWS: u16 = 1;
/// A Find match is washed with the warning color; the current one is stronger.
const FIND_MATCH_ALPHA: f32 = 0.4;
const FIND_CURRENT_ALPHA: f32 = 0.75;

/// Where the grid sits in the window and how big a cell is, as the last frame measured them. The
/// element writes it and the owner reads it to turn a pointer position into a cell.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerminalMetrics {
    pub(crate) origin: Point<Pixels>,
    pub(crate) cell: Size<Pixels>,
}

pub(crate) type SharedMetrics = Rc<Cell<Option<TerminalMetrics>>>;

/// A pointer position as fractional rows and columns of the grid; a position above or left of the
/// grid counts as its first row or column.
pub(crate) fn cell_at(metrics: TerminalMetrics, position: Point<Pixels>) -> (f32, f32) {
    let offset = position - metrics.origin;
    let row = (offset.y / metrics.cell.height).max(0.);
    let col = (offset.x / metrics.cell.width).max(0.);
    (row, col)
}

/// What one frame measured before painting.
pub(crate) struct Frame {
    cell: Size<Pixels>,
    font: Font,
    font_size: Pixels,
    selection: Hsla,
    find: Hsla,
}

/// The terminal for `session`, filling its parent. Each frame sizes the grid to the bounds and,
/// when the size changed, sends one `ShellInput::Resize` (the remote `SIGWINCH`); the session
/// keeps the latest size, so a burst of window sizes costs one resize per frame at most.
pub(crate) fn terminal_element(
    session: Rc<RefCell<TerminalSession>>,
    input: UnboundedSender<ShellInput>,
    metrics: SharedMetrics,
) -> impl IntoElement {
    let paint_session = Rc::clone(&session);
    canvas(
        move |bounds, window, cx| measure(&session, &input, &metrics, bounds, window, cx),
        move |bounds, frame, window, cx| paint(&paint_session, bounds, &frame, window, cx),
    )
    .size_full()
}

fn measure(
    session: &RefCell<TerminalSession>,
    input: &UnboundedSender<ShellInput>,
    metrics: &Cell<Option<TerminalMetrics>>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Frame {
    let theme = cx.theme();
    let font = Font {
        family: theme.mono_font_family.clone(),
        ..Font::default()
    };
    let font_size = theme.mono_font_size;
    let palette = terminal_palette(theme);
    let selection = theme.selection;
    let find = theme.warning;
    let cell = cell_size(window, &font, font_size);
    metrics.set(Some(TerminalMetrics {
        origin: bounds.origin,
        cell,
    }));

    let frame = Frame {
        cell,
        font,
        font_size,
        selection,
        find,
    };
    // The owner never holds the session across a frame; if it does, the frame is skipped
    // rather than panicking in the middle of a paint.
    let Ok(mut session) = session.try_borrow_mut() else {
        return frame;
    };
    // A theme change shows up here: the palette is rebuilt from the theme every frame, and the
    // session repaints every row only when it differs.
    session.set_palette(palette);
    session.set_cell_pixels(pixel_count(cell.width), pixel_count(cell.height));
    let grid = grid_size(bounds.size, cell);
    if session.resize(grid) {
        // A closed receiver means the session ended; there is nobody to resize.
        let _ = input.unbounded_send(ShellInput::Resize(grid));
    }
    frame
}

/// The width of one `M` and a line of 1.3 times the font size, so that rows sit apart.
fn cell_size(window: &Window, font: &Font, font_size: Pixels) -> Size<Pixels> {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(font);
    let width = text_system
        .advance(font_id, font_size, 'M')
        .map_or(font_size * FALLBACK_ADVANCE_RATIO, |advance| advance.width);
    size(width, (font_size * LINE_HEIGHT_RATIO).round())
}

fn pixel_count(extent: Pixels) -> u16 {
    f32::from(extent).round().clamp(0., f32::from(u16::MAX)) as u16
}

/// How many whole cells fit in `bounds`, never fewer than 2 columns by 1 row.
pub(crate) fn grid_size(bounds: Size<Pixels>, cell: Size<Pixels>) -> GridSize {
    let fit = |extent: Pixels, cell: Pixels, minimum: u16| -> u16 {
        let count = (extent / cell).floor();
        if !count.is_finite() {
            return minimum;
        }
        count.clamp(f32::from(minimum), f32::from(u16::MAX)) as u16
    };
    GridSize {
        cols: fit(bounds.width, cell.width, MIN_COLS),
        rows: fit(bounds.height, cell.height, MIN_ROWS),
    }
}

fn paint(
    session: &RefCell<TerminalSession>,
    bounds: Bounds<Pixels>,
    frame: &Frame,
    window: &mut Window,
    cx: &mut App,
) {
    let Ok(mut session) = session.try_borrow_mut() else {
        return;
    };
    let palette = *session.palette();
    let cursor_shape = session.cursor_shape();
    // Copied first: the snapshot below borrows the session mutably until the last paint.
    let matches = session.find_matches().to_vec();
    let current_match = session.current_find_match();
    let snapshot = session.snapshot(Instant::now());

    window.paint_quad(fill(bounds, hsla_of(palette.background)));
    let cell = frame.cell;
    for (index, row) in snapshot.rows().iter().enumerate() {
        let origin = bounds.origin + point(px(0.), cell.height * index as f32);
        paint_row_backgrounds(row, origin, cell, &palette, window);
        paint_row_text(row, origin, frame, &palette, window, cx);
    }
    paint_selection(snapshot, bounds.origin, frame, window);
    paint_find_matches(
        &matches,
        current_match,
        snapshot,
        bounds.origin,
        frame,
        window,
    );
    paint_cursor(
        snapshot,
        cursor_shape,
        bounds.origin,
        cell,
        &palette,
        window,
    );
}

fn paint_row_backgrounds(
    row: &SnapshotRow,
    origin: Point<Pixels>,
    cell: Size<Pixels>,
    palette: &Palette,
    window: &mut Window,
) {
    for run in &row.runs {
        let (_, background) = cell_colors(&run.style, palette);
        if background == palette.background {
            continue;
        }
        let start = origin + point(cell.width * f32::from(run.cols.start), px(0.));
        let columns = run.cols.end.saturating_sub(run.cols.start);
        let extent = size(cell.width * f32::from(columns), cell.height);
        window.paint_quad(fill(Bounds::new(start, extent), hsla_of(background)));
    }
}

/// A wide glyph is drawn on its own at its column, so the row's text keeps every column.
struct WideGlyph {
    col: u16,
    text: String,
    run: TextRun,
}

fn paint_row_text(
    row: &SnapshotRow,
    origin: Point<Pixels>,
    frame: &Frame,
    palette: &Palette,
    window: &mut Window,
    cx: &mut App,
) {
    let (text, runs, wide) = row_text_runs(row, frame, palette);
    let cell = frame.cell;
    if text.trim().is_empty() && wide.is_empty() {
        return;
    }
    // `force_width` gives every glyph one cell, so columns hold even where the font disagrees.
    let line = window.text_system().shape_line(
        SharedString::from(text),
        frame.font_size,
        &runs,
        Some(cell.width),
    );
    // A paint error means the glyph could not be rasterized; the frame goes on without it.
    let _ = line.paint(origin, cell.height, TextAlign::Left, None, window, cx);
    for glyph in wide {
        let line = window.text_system().shape_line(
            SharedString::from(glyph.text),
            frame.font_size,
            &[glyph.run],
            Some(cell.width * 2.),
        );
        let at = origin + point(cell.width * f32::from(glyph.col), px(0.));
        let _ = line.paint(at, cell.height, TextAlign::Left, None, window, cx);
    }
}

/// The text of one row with a `TextRun` per run of cells of one style. A wide glyph and its
/// spacer read as two spaces here, and the glyph is returned to be drawn on its own.
fn row_text_runs(
    row: &SnapshotRow,
    frame: &Frame,
    palette: &Palette,
) -> (String, Vec<TextRun>, Vec<WideGlyph>) {
    let mut text = String::new();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut wide = Vec::new();
    let mut current: Option<u16> = None;
    for (col, cell) in row.cells.iter().enumerate() {
        let before = text.len();
        let glyph = glyph_text(row, cell);
        let style = row.style_of(cell);
        match cell.width {
            CellWidth::Wide => {
                text.push(' ');
                wide.push(WideGlyph {
                    col: u16::try_from(col).unwrap_or(u16::MAX),
                    run: text_run(glyph.len(), style, frame, palette),
                    text: glyph,
                });
            }
            CellWidth::WideSpacer | CellWidth::LeadingWideSpacer => text.push(' '),
            CellWidth::Narrow => text.push_str(&glyph),
        }
        let added = text.len() - before;
        match runs.last_mut() {
            Some(last) if current == Some(cell.run) => last.len += added,
            _ => {
                runs.push(text_run(added, style, frame, palette));
                current = Some(cell.run);
            }
        }
    }
    (text, runs, wide)
}

/// The characters of one cell. Control characters never reach the shaper.
fn glyph_text(row: &SnapshotRow, cell: &oneterm_vt::SnapshotCell) -> String {
    let readable = |character: char| {
        if character.is_control() {
            ' '
        } else {
            character
        }
    };
    match cell.content {
        SnapshotContent::Scalar(character) => readable(character).to_string(),
        SnapshotContent::Cluster { start, len } => {
            let cluster: String = row
                .cluster(start, len)
                .iter()
                .copied()
                .filter(|character| !character.is_control())
                .collect();
            if cluster.is_empty() {
                " ".to_owned()
            } else {
                cluster
            }
        }
    }
}

fn text_run(len: usize, style: &Style, frame: &Frame, palette: &Palette) -> TextRun {
    let (foreground, _) = cell_colors(style, palette);
    let color = hsla_of(foreground);
    let mut font = frame.font.clone();
    if style.attrs.contains(Attrs::BOLD) {
        font.weight = FontWeight::BOLD;
    }
    if style.attrs.contains(Attrs::ITALIC) {
        font.style = FontStyle::Italic;
    }
    TextRun {
        len,
        font,
        color,
        background_color: None,
        underline: style
            .attrs
            .intersects(Attrs::ALL_UNDERLINES)
            .then_some(UnderlineStyle {
                thickness: px(LINE_THICKNESS),
                color: Some(color),
                wavy: style.attrs.contains(Attrs::UNDERCURL),
            }),
        strikethrough: style
            .attrs
            .contains(Attrs::STRIKEOUT)
            .then_some(StrikethroughStyle {
                thickness: px(LINE_THICKNESS),
                color: Some(color),
            }),
    }
}

/// The foreground and background a style draws with: inverse swaps them, dim fades the text
/// toward the background, and hidden text takes the background color.
pub(crate) fn cell_colors(style: &Style, palette: &Palette) -> (Rgb, Rgb) {
    let resolve = |color: Color| palette.resolve(color);
    let (mut foreground, mut background) = (resolve(style.fg), resolve(style.bg));
    if style.attrs.contains(Attrs::INVERSE) {
        std::mem::swap(&mut foreground, &mut background);
    }
    if style.attrs.contains(Attrs::DIM) {
        foreground = palette.dim(foreground);
    }
    if style.attrs.contains(Attrs::HIDDEN) {
        foreground = background;
    }
    (foreground, background)
}

fn paint_selection(
    snapshot: &SnapshotState,
    origin: Point<Pixels>,
    frame: &Frame,
    window: &mut Window,
) {
    let Some(range) = snapshot.selection() else {
        return;
    };
    let cell = frame.cell;
    let spans = selection_spans(range, snapshot.viewport_top(), snapshot.size());
    for span in spans {
        let start = origin
            + point(
                cell.width * f32::from(span.start_col),
                cell.height * f32::from(span.row),
            );
        let columns = span.end_col.saturating_sub(span.start_col);
        let extent = size(cell.width * f32::from(columns), cell.height);
        window.paint_quad(fill(Bounds::new(start, extent), frame.selection));
    }
}

fn paint_find_matches(
    matches: &[SearchMatch],
    current: Option<SearchMatch>,
    snapshot: &SnapshotState,
    origin: Point<Pixels>,
    frame: &Frame,
    window: &mut Window,
) {
    let cell = frame.cell;
    for found in matches {
        let Some(span) = match_span(*found, snapshot.viewport_top(), snapshot.size()) else {
            continue;
        };
        let alpha = if current == Some(*found) {
            FIND_CURRENT_ALPHA
        } else {
            FIND_MATCH_ALPHA
        };
        let start = origin
            + point(
                cell.width * f32::from(span.start_col),
                cell.height * f32::from(span.row),
            );
        let columns = span.end_col - span.start_col;
        let extent = size(cell.width * f32::from(columns), cell.height);
        window.paint_quad(fill(Bounds::new(start, extent), frame.find.opacity(alpha)));
    }
}

/// The cells of a Find match when it is on screen. A match is one row long.
pub(crate) fn match_span(
    found: SearchMatch,
    top: RowId,
    size: oneterm_vt::Size,
) -> Option<SelectionSpan> {
    let row = i64::try_from(found.row.0).unwrap_or(i64::MAX) - top_id(top);
    if !(0..i64::from(size.rows)).contains(&row) {
        return None;
    }
    let start_col = u16::try_from(found.start_col).unwrap_or(u16::MAX);
    let end_col = u16::try_from(found.end_col)
        .unwrap_or(u16::MAX)
        .min(size.cols);
    (start_col < end_col).then_some(SelectionSpan {
        row: u16::try_from(row).unwrap_or(u16::MAX),
        start_col,
        end_col,
    })
}

/// One highlighted run of cells on a viewport row; `end_col` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SelectionSpan {
    pub(crate) row: u16,
    pub(crate) start_col: u16,
    pub(crate) end_col: u16,
}

/// The cells a selection covers on the visible rows. The range is inclusive at both ends and in
/// row ids; a stream selection takes the rest of its first row, whole rows between, and the start
/// of its last row, and a block takes the same columns on every row.
pub(crate) fn selection_spans(
    range: SelectionRange,
    top: RowId,
    size: oneterm_vt::Size,
) -> Vec<SelectionSpan> {
    let first = i64::try_from(range.start.row.0).unwrap_or(i64::MAX) - top_id(top);
    let last = i64::try_from(range.end.row.0).unwrap_or(i64::MAX) - top_id(top);
    let (left, right) = (
        range.start.col.min(range.end.col),
        range.start.col.max(range.end.col),
    );
    let mut spans = Vec::new();
    for row in first.max(0)..=last.min(i64::from(size.rows) - 1) {
        let (start_col, end_col) = if range.is_block {
            (left, right.saturating_add(1))
        } else if first == last {
            (range.start.col, range.end.col.saturating_add(1))
        } else if row == first {
            (range.start.col, size.cols)
        } else if row == last {
            (0, range.end.col.saturating_add(1))
        } else {
            (0, size.cols)
        };
        let end_col = end_col.min(size.cols);
        if start_col < end_col {
            spans.push(SelectionSpan {
                row: u16::try_from(row).unwrap_or(u16::MAX),
                start_col,
                end_col,
            });
        }
    }
    spans
}

fn top_id(top: RowId) -> i64 {
    i64::try_from(top.0).unwrap_or(i64::MAX)
}

fn paint_cursor(
    snapshot: &SnapshotState,
    shape: CursorShape,
    origin: Point<Pixels>,
    cell: Size<Pixels>,
    palette: &Palette,
    window: &mut Window,
) {
    let cursor = snapshot.cursor();
    let Some(row) = cursor.row.filter(|_| cursor.visible) else {
        return;
    };
    if cursor.col >= snapshot.size().cols {
        return;
    }
    let corner = origin
        + point(
            cell.width * f32::from(cursor.col),
            cell.height * f32::from(row),
        );
    let color = hsla_of(palette.cursor);
    let area = Bounds::new(corner, cell);
    match shape {
        CursorShape::Hidden => {}
        CursorShape::Block => window.paint_quad(fill(area, color.opacity(BLOCK_CURSOR_ALPHA))),
        CursorShape::HollowBlock => window.paint_quad(outline(area, color, BorderStyle::Solid)),
        CursorShape::Beam => {
            let beam = Bounds::new(corner, size(px(BAR_CURSOR_THICKNESS), cell.height));
            window.paint_quad(fill(beam, color));
        }
        CursorShape::Underline => {
            let at = corner + point(px(0.), cell.height - px(BAR_CURSOR_THICKNESS));
            let line = Bounds::new(at, size(cell.width, px(BAR_CURSOR_THICKNESS)));
            window.paint_quad(fill(line, color));
        }
    }
}

/// An engine color as a theme-independent GPUI color; its source is a theme token.
fn hsla_of(color: Rgb) -> Hsla {
    Hsla::from(Rgba {
        r: f32::from(color.r) / 255.,
        g: f32::from(color.g) / 255.,
        b: f32::from(color.b) / 255.,
        a: 1.,
    })
}

#[cfg(test)]
#[path = "terminal_element_tests.rs"]
mod terminal_element_tests;
