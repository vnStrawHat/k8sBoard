//! One terminal: the `oneterm-vt` engine, its snapshot, and the policy for what a remote program
//! may ask of it (spec 0036). No GPUI types except the kit theme colors in `terminal_palette`.
//!
//! The engine runs on the GPUI thread with no lock: every update is at most one 64 KiB read, so
//! one `feed` is about a millisecond in a release build.
//!
//! Event policy ("not a policy" engine, decision 17): replies and color queries are answered, the
//! private OSC 7770 names the shell, and everything else a program can ask for (clipboard
//! access, title, bell, notifications, working directory, progress) is ignored. Nothing from
//! the session is logged, traced, or kept after the tab closes (C1).

use std::time::Instant;

use cluster::GridSize;
use gpui_kit::Rgba;
use gpui_kit::component::ThemeColor;
use oneterm_vt::input::{KeyMods, KeySpec};
use oneterm_vt::search::{GridText, SearchMatch, SearchOptions, SearchPattern, search_grid_text};
use oneterm_vt::{
    ColorKey, Config, CursorShape, EventBatch, ModeSnapshot, OscRoute, OscRoutes, Palette,
    ParamSpans, ResizePolicy, Rgb, SelectionKind, Size, SnapshotState, StringTerm, Terminal,
    VtEvent,
};

/// The most reply bytes held before the owner drains the outbox. A program can ask for a reply
/// with every few bytes it prints (`ESC [ c`), so a bulk print would otherwise queue megabytes;
/// a reply past the bound is dropped, which a program that floods queries cannot tell from a
/// slow terminal.
const MAX_OUTBOX_BYTES: usize = 64 * 1024;
/// The private OSC the `Auto` shell script uses to name the shell it picked.
const SHELL_OSC: u32 = 7770;

/// The shell the container runs, as far as the session knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedShell {
    /// Not reported yet: `Auto` has not picked one, or the script said something else.
    Auto,
    Bash,
    Ash,
    Sh,
}

impl ResolvedShell {
    /// The name the tab header shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Bash => "bash",
            Self::Ash => "ash",
            Self::Sh => "sh",
        }
    }

    /// Only the three shells the script can pick; any other text is not a shell name.
    fn named(name: &[u8]) -> Option<Self> {
        match name {
            b"bash" => Some(Self::Bash),
            b"ash" => Some(Self::Ash),
            b"sh" => Some(Self::Sh),
            _ => None,
        }
    }
}

pub(crate) struct TerminalSession {
    term: Terminal,
    batch: EventBatch,
    snapshot: SnapshotState,
    palette: Palette,
    /// Bytes owed to the remote program: terminal replies and color answers, at most
    /// `MAX_OUTBOX_BYTES` until the owner takes them.
    outbox: Vec<u8>,
    shell: ResolvedShell,
    cell_pixels: (u16, u16),
    find: FindState,
}

/// The matches of the last Find query, newest first, and the one the user is on.
#[derive(Default)]
struct FindState {
    query: String,
    matches: Vec<SearchMatch>,
    current: Option<usize>,
}

impl TerminalSession {
    /// `scrollback_lines` is the rows of history the terminal keeps (the saved setting, already clamped).
    pub(crate) fn new(size: GridSize, scrollback_lines: u32) -> Self {
        let mut osc_routes = OscRoutes::new();
        osc_routes.route(SHELL_OSC, OscRoute::Forward);
        let config = Config {
            scrollback_limit: scrollback_lines,
            osc_routes,
            product_name: Some("k8sBoard".into()),
            // DECRQCRA would let a program read back screen text it did not write, such as a
            // password prompt.
            allow_screen_readback: false,
            ..Config::default()
        };
        Self {
            term: Terminal::new(vt_size(size), config),
            batch: EventBatch::new(),
            snapshot: SnapshotState::new(),
            palette: Palette::new(),
            outbox: Vec::new(),
            shell: ResolvedShell::Auto,
            cell_pixels: (0, 0),
            find: FindState::default(),
        }
    }

    /// Feeds remote bytes, queues the engine's replies in the outbox, and applies the event
    /// policy. Never panics on input: the engine drops what it cannot parse.
    // ponytail: parse on the UI thread; if bulk output stalls frames, move feed to a reader
    // thread with the Terminal behind a mutex (vt guide ch. 3 two-phase hand-off).
    pub(crate) fn feed(&mut self, bytes: &[u8], now: Instant) {
        self.term.feed(bytes, &mut self.batch, now);
        self.apply_events();
        // 0036 draws no images, and the engine keeps every decoded one until it is taken, so a
        // program printing Sixel in a loop would grow without limit.
        drop(self.term.take_graphics());
    }

    /// The bytes to send to the remote program, emptied by the call.
    pub(crate) fn take_outbox(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.outbox)
    }

    /// A local dim line (banner, exit, errors). Control characters are stripped, so remote text
    /// that reaches a note cannot smuggle an escape sequence in.
    pub(crate) fn note(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        self.feed(
            format!("\r\n\x1b[2m{clean}\x1b[0m\r\n").as_bytes(),
            Instant::now(),
        );
    }

    /// Whether the size changed. A resize reflows every row, so callers ask only when the
    /// element's grid moved.
    pub(crate) fn resize(&mut self, size: GridSize) -> bool {
        let size = vt_size(size);
        if size == self.term.size() {
            return false;
        }
        self.term.resize(size, ResizePolicy::BottomAnchor);
        // A reflow gives rows new identities, which the matches name, so they are found again.
        if !self.find.query.is_empty() {
            let (query, current) = (self.find.query.clone(), self.find.current);
            self.find(&query);
            if let Some(last) = self.find.matches.len().checked_sub(1) {
                self.find.current = current.map(|current| current.min(last));
            }
        }
        true
    }

    /// The current view of the grid, colors resolved against the palette. Cheap when nothing
    /// changed: only the rows that moved since the last call are copied.
    #[cfg(test)]
    pub(crate) fn snapshot(&mut self, now: Instant) -> &SnapshotState {
        self.refresh(now);
        &self.snapshot
    }

    /// Brings the snapshot up to date. A painter calls this, then reads `view` together with other
    /// parts of the session (the Find matches) without copying them.
    pub(crate) fn refresh(&mut self, now: Instant) {
        self.term.snapshot_update(&mut self.snapshot, now);
        self.snapshot.map_colors(&self.palette);
    }

    /// The snapshot of the last `refresh`.
    pub(crate) fn view(&self) -> &SnapshotState {
        &self.snapshot
    }

    /// A new palette repaints every row: the engine counts it as a new palette epoch. The element
    /// offers the theme's palette every frame, so an equal one must cost nothing.
    pub(crate) fn set_palette(&mut self, palette: Palette) {
        if self.palette == palette {
            return;
        }
        self.term.set_theme_colors(&palette);
        self.palette = palette;
    }

    /// The size the engine is at now, which is what a new exec starts with.
    pub(crate) fn grid_size(&self) -> GridSize {
        let size = self.term.size();
        GridSize {
            cols: size.cols,
            rows: size.rows,
        }
    }

    pub(crate) fn shell(&self) -> ResolvedShell {
        self.shell
    }

    pub(crate) fn cursor_shape(&self) -> CursorShape {
        self.term.cursor_style().shape
    }

    pub(crate) fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The cell size in pixels, which only the renderer knows; it is what `CSI 14 t` reports.
    pub(crate) fn set_cell_pixels(&mut self, width: u16, height: u16) {
        if self.cell_pixels != (width, height) {
            self.cell_pixels = (width, height);
            self.term.set_cell_pixels(width, height);
        }
    }

    /// The modes a program set: bracketed paste, mouse reporting, alternate screen.
    pub(crate) fn modes(&self) -> ModeSnapshot {
        self.term.mode_snapshot()
    }

    /// The bytes a key sends, in the modes the program set. `None` for a chord with no encoding.
    pub(crate) fn encode_key(&self, key: &KeySpec, mods: KeyMods) -> Option<Vec<u8>> {
        self.term.encode_key(key, mods)
    }

    /// Scrolls the view by `lines`; a negative count goes towards history.
    pub(crate) fn scroll_lines(&mut self, lines: i32) {
        self.term.grid_mut().screen_mut().scroll_viewport(lines);
    }

    /// Returns the view to the live screen, where new output lands.
    pub(crate) fn scroll_to_bottom(&mut self) {
        self.term.grid_mut().screen_mut().scroll_to_bottom();
    }

    /// Starts a selection at a pointer position given in fractional cells of the view.
    pub(crate) fn begin_selection(&mut self, row: f32, col: f32, kind: SelectionKind) {
        let (pos, side) = self.term.hit_test(row, col);
        self.term.selection_start(pos, side, kind);
    }

    /// Moves the far end of the selection to the pointer.
    pub(crate) fn extend_selection(&mut self, row: f32, col: f32) {
        let (pos, side) = self.term.hit_test(row, col);
        self.term.selection_update(pos, side);
    }

    pub(crate) fn clear_selection(&mut self) {
        self.term.selection_clear();
    }

    /// The selected text, `None` when nothing is selected. Shell output can hold secrets, so the
    /// caller hands it to the private clipboard write and keeps no copy.
    pub(crate) fn selected_text(&self) -> Option<String> {
        self.term.selection_text().filter(|text| !text.is_empty())
    }

    /// Searches the whole grid, history included, for `query` ignoring ASCII case. The newest match
    /// becomes current and is scrolled into view. An empty query clears the highlights. Returns the
    /// match count.
    pub(crate) fn find(&mut self, query: &str) -> usize {
        self.find = FindState::default();
        if query.is_empty() {
            return 0;
        }
        self.find.query = query.to_owned();
        let text = GridText::from_terminal(&self.term);
        let mut matches = search_grid_text(
            &text,
            SearchPattern::Literal(query),
            SearchOptions::default(),
        );
        // The engine lists oldest first; the user starts from the bottom of the screen.
        matches.reverse();
        self.find.current = (!matches.is_empty()).then_some(0);
        self.find.matches = matches;
        self.reveal_current_match();
        self.find.matches.len()
    }

    /// Moves to the next older match, wrapping to the newest.
    pub(crate) fn find_next(&mut self) {
        self.step_match(true);
    }

    /// Moves to the next newer match, wrapping to the oldest.
    pub(crate) fn find_previous(&mut self) {
        self.step_match(false);
    }

    pub(crate) fn clear_find(&mut self) {
        self.find = FindState::default();
    }

    /// `(position, total)`, the position counted from 1 over the matches newest first.
    pub(crate) fn find_status(&self) -> Option<(usize, usize)> {
        Some((self.find.current? + 1, self.find.matches.len()))
    }

    pub(crate) fn find_matches(&self) -> &[SearchMatch] {
        &self.find.matches
    }

    pub(crate) fn current_find_match(&self) -> Option<SearchMatch> {
        self.find.matches.get(self.find.current?).copied()
    }

    fn step_match(&mut self, is_older: bool) {
        let total = self.find.matches.len();
        let Some(current) = self.find.current else {
            return;
        };
        self.find.current = Some(if is_older {
            (current + 1) % total
        } else {
            (current + total - 1) % total
        });
        self.reveal_current_match();
    }

    /// Scrolls so the current match is on screen, near the middle when it was not.
    fn reveal_current_match(&mut self) {
        let Some(found) = self.current_find_match() else {
            return;
        };
        let view = self.term.viewport();
        let row = i64::try_from(found.row.0).unwrap_or(i64::MAX);
        let top = i64::try_from(view.top.0).unwrap_or(i64::MAX);
        if (top..top + i64::from(view.rows)).contains(&row) {
            return;
        }
        let wanted_top = row - i64::from(view.rows / 2);
        let delta = (wanted_top - top).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        self.scroll_lines(i32::try_from(delta).unwrap_or(0));
    }

    fn apply_events(&mut self) {
        // Disjoint fields: the batch is read while the outbox and the shell name are written.
        let Self {
            term,
            batch,
            palette,
            outbox,
            shell,
            ..
        } = self;
        for event in batch.iter() {
            match event {
                VtEvent::Reply(span) => queue_reply(outbox, batch.bytes(*span)),
                VtEvent::ColorQuery { key, terminator } => {
                    let color = term.color(*key).or_else(|| palette_color(palette, *key));
                    if let Some(color) = color {
                        queue_reply(outbox, &color_reply(*key, *terminator, color));
                    }
                }
                VtEvent::Osc {
                    code: SHELL_OSC,
                    params,
                    truncated: false,
                    ..
                } => {
                    if let Some(name) = shell_name(batch, *params) {
                        *shell = name;
                    }
                }
                // ClipboardStore and ClipboardLoad are ignored on purpose: a container neither
                // writes nor reads the local clipboard. Title, Bell, Cwd, Notification, Progress,
                // ShellMark, and every other event carry nothing the tab shows.
                _ => {}
            }
        }
    }
}

/// Queues a reply unless it would take the outbox past its bound.
fn queue_reply(outbox: &mut Vec<u8>, reply: &[u8]) {
    if outbox.len() + reply.len() <= MAX_OUTBOX_BYTES {
        outbox.extend_from_slice(reply);
    }
}

fn vt_size(size: GridSize) -> Size {
    Size {
        rows: size.rows,
        cols: size.cols,
    }
    .clamped()
}

/// The payload of an OSC 7770 when it names a known shell.
fn shell_name(batch: &EventBatch, params: ParamSpans) -> Option<ResolvedShell> {
    // Parameter 0 is the OSC number itself.
    let name = batch.params(params).nth(1)?;
    ResolvedShell::named(name)
}

/// The palette's answer to a query, or `None` for a color the palette does not hold (the two
/// selection colors).
fn palette_color(palette: &Palette, key: ColorKey) -> Option<Rgb> {
    Some(match key {
        ColorKey::Palette(index) => palette.indexed[usize::from(index)],
        ColorKey::Foreground => palette.foreground,
        ColorKey::Background => palette.background,
        ColorKey::Cursor => palette.cursor,
        ColorKey::BrightForeground => palette.bright_foreground.unwrap_or(palette.foreground),
        ColorKey::DimForeground => palette
            .dim_foreground
            .unwrap_or_else(|| palette.dim(palette.foreground)),
        ColorKey::Dim(index) => palette.dim(palette.indexed[usize::from(index & 7)]),
        ColorKey::SelectionBackground | ColorKey::SelectionForeground => return None,
    })
}

/// xterm's reply form, each channel doubled to 16 bits, ended the way the question ended.
fn color_reply(key: ColorKey, terminator: StringTerm, color: Rgb) -> Vec<u8> {
    let end = match terminator {
        StringTerm::Bel => "\x07",
        StringTerm::St => "\x1b\\",
    };
    format!(
        "\x1b]{};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}{end}",
        key.query_prefix(),
        color.r,
        color.r,
        color.g,
        color.g,
        color.b,
        color.b
    )
    .into_bytes()
}

/// The ANSI table from kit theme tokens: black is `muted`, white and bright black are
/// `muted_foreground`, bright white is `foreground`. The cube and greys (16 to 255) stay the
/// xterm defaults. Recomputed when the theme changes.
pub(crate) fn terminal_palette(theme: &ThemeColor) -> Palette {
    let mut palette = Palette::new();
    let ansi = [
        theme.muted,
        theme.red,
        theme.green,
        theme.yellow,
        theme.blue,
        theme.magenta,
        theme.cyan,
        theme.muted_foreground,
        theme.muted_foreground,
        theme.red_light,
        theme.green_light,
        theme.yellow_light,
        theme.blue_light,
        theme.magenta_light,
        theme.cyan_light,
        theme.foreground,
    ];
    for (slot, color) in palette.indexed.iter_mut().zip(ansi) {
        *slot = rgb_of(color);
    }
    palette.foreground = rgb_of(theme.foreground);
    palette.background = rgb_of(theme.background);
    palette.cursor = rgb_of(theme.caret);
    palette
}

/// A theme color as the engine's 8-bit channels; the theme's alpha is not used.
pub(crate) fn rgb_of(color: gpui_kit::Hsla) -> Rgb {
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    Rgb {
        r: channel(rgba.r),
        g: channel(rgba.g),
        b: channel(rgba.b),
    }
}

#[cfg(test)]
#[path = "terminal_session_tests.rs"]
mod terminal_session_tests;
