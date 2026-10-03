use gpui_kit::component::ThemeColor;
use oneterm_vt::{Attrs, CellWidth, Color, SelectionKind, SnapshotContent, SnapshotRow};

use super::*;

const SIZE: GridSize = GridSize { cols: 20, rows: 5 };

fn now() -> Instant {
    Instant::now()
}

/// A session fed `bytes`, painted with the light theme.
fn fed(bytes: &[u8]) -> TerminalSession {
    let mut session = TerminalSession::new(SIZE);
    session.set_palette(terminal_palette(&ThemeColor::light()));
    session.feed(bytes, now());
    session
}

fn row_text(row: &SnapshotRow) -> String {
    let mut text = String::new();
    for cell in &row.cells {
        if cell.width == CellWidth::WideSpacer {
            continue;
        }
        match cell.content {
            SnapshotContent::Scalar(scalar) => text.push(scalar),
            SnapshotContent::Cluster { start, len } => text.extend(row.cluster(start, len)),
        }
    }
    text.trim_end().to_owned()
}

fn screen_text(session: &mut TerminalSession) -> Vec<String> {
    session
        .snapshot(now())
        .rows()
        .iter()
        .map(row_text)
        .collect()
}

fn light_theme_reply(key: &str, end: &str) -> Vec<u8> {
    let theme = ThemeColor::light();
    let color = rgb_of(theme.background);
    format!(
        "\x1b]{key};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}{end}",
        r = color.r,
        g = color.g,
        b = color.b
    )
    .into_bytes()
}

#[test]
fn fed_text_appears_in_the_snapshot() {
    let mut session = fed(b"hello\r\nworld");
    let rows = screen_text(&mut session);
    assert_eq!(rows[0], "hello");
    assert_eq!(rows[1], "world");
    assert_eq!(rows.len(), usize::from(SIZE.rows));
}

#[test]
fn sgr_colors_map_to_theme_tokens() {
    let theme = ThemeColor::light();
    let mut session = fed(b"\x1b[31mred");
    let snapshot = session.snapshot(now());
    let row = &snapshot.rows()[0];
    let style = row.style_of(&row.cells[0]);
    assert_eq!(style.fg, Color::Rgb(rgb_of(theme.red)));
    // The default background resolves to the theme background, not a hard-coded black.
    assert_eq!(style.bg, Color::Rgb(rgb_of(theme.background)));
}

#[test]
fn a_new_palette_recolors_rows_already_drawn() {
    let mut session = fed(b"\x1b[31mred");
    session.snapshot(now());
    let dark = ThemeColor::dark();
    session.set_palette(terminal_palette(&dark));
    let snapshot = session.snapshot(now());
    let row = &snapshot.rows()[0];
    assert_eq!(row.style_of(&row.cells[0]).fg, Color::Rgb(rgb_of(dark.red)));
}

#[test]
fn an_unchanged_palette_does_not_repaint() {
    let mut session = fed(b"hello");
    session.snapshot(now());
    let copied = session.snapshot.rows_copied();
    session.set_palette(terminal_palette(&ThemeColor::light()));
    session.snapshot(now());
    assert_eq!(session.snapshot.rows_copied(), copied);
}

#[test]
fn device_attributes_query_is_answered_through_the_outbox() {
    let mut session = fed(b"\x1b[c");
    let reply = session.take_outbox();
    assert!(!reply.is_empty());
    assert!(
        session.take_outbox().is_empty(),
        "the outbox is emptied by the call"
    );
}

#[test]
fn a_flood_of_queries_cannot_grow_the_outbox_past_its_bound() {
    let mut session = fed(b"");
    // Each `ESC [ c` owes a reply; 40 000 of them would be several hundred KiB.
    let queries = b"\x1b[c".repeat(40_000);
    session.feed(&queries, now());
    let owed = session.take_outbox();
    assert!(!owed.is_empty());
    assert!(owed.len() <= MAX_OUTBOX_BYTES, "{} bytes", owed.len());
    // Draining resets the bound: the next query is answered again.
    session.feed(b"\x1b[c", now());
    assert!(!session.take_outbox().is_empty());
}

/// A one-pixel Sixel: DCS q, one band, ST.
const SIXEL: &[u8] = b"\x1bPq#0;2;100;0;0#0~\x1b\\";

#[test]
fn the_engine_keeps_a_decoded_image_until_it_is_taken() {
    // Guards the test below: this sequence really is decoded into an image.
    let mut session = TerminalSession::new(SIZE);
    session.term.feed(SIXEL, &mut session.batch, now());
    assert_eq!(session.term.take_graphics().len(), 1);
}

#[test]
fn sixel_images_are_not_kept() {
    let mut session = TerminalSession::new(SIZE);
    for _ in 0..50 {
        session.feed(SIXEL, now());
    }
    assert!(session.term.take_graphics().is_empty());
}

#[test]
fn decrqcra_is_not_answered() {
    let mut session = fed(b"secret\x1b[1;1;1;1;2;2*y");
    assert!(session.take_outbox().is_empty());
}

#[test]
fn color_query_is_answered_with_its_terminator() {
    let mut session = fed(b"\x1b]11;?\x07");
    assert_eq!(session.take_outbox(), light_theme_reply("11", "\x07"));
    session.feed(b"\x1b]11;?\x1b\\", now());
    assert_eq!(session.take_outbox(), light_theme_reply("11", "\x1b\\"));
}

#[test]
fn color_query_reads_an_override_the_program_set() {
    let mut session = fed(b"\x1b]11;rgb:10/20/30\x07\x1b]11;?\x07");
    assert_eq!(
        session.take_outbox(),
        b"\x1b]11;rgb:1010/2020/3030\x07".to_vec()
    );
}

#[test]
fn osc52_store_and_load_are_ignored() {
    // A store of "hello" and a read request: neither is answered or acted on.
    let mut session = fed(b"\x1b]52;c;aGVsbG8=\x07\x1b]52;c;?\x07\x1b]52;c;?\x1b\\");
    assert!(session.take_outbox().is_empty());
}

#[test]
fn osc_7770_sets_the_resolved_shell() {
    let mut session = fed(b"");
    assert_eq!(session.shell(), ResolvedShell::Auto);
    session.feed(b"\x1b]7770;bash\x07", now());
    assert_eq!(session.shell(), ResolvedShell::Bash);
    session.feed(b"\x1b]7770;ash\x1b\\", now());
    assert_eq!(session.shell(), ResolvedShell::Ash);
    session.feed(b"\x1b]7770;sh\x07", now());
    assert_eq!(session.shell(), ResolvedShell::Sh);
    // Text that is no shell name leaves the pick alone.
    session.feed(b"\x1b]7770;evil\x07", now());
    assert_eq!(session.shell(), ResolvedShell::Sh);
}

#[test]
fn resolved_shell_labels_are_the_header_names() {
    let labels = [
        (ResolvedShell::Auto, "Auto"),
        (ResolvedShell::Bash, "bash"),
        (ResolvedShell::Ash, "ash"),
        (ResolvedShell::Sh, "sh"),
    ];
    for (shell, label) in labels {
        assert_eq!(shell.label(), label);
    }
}

#[test]
fn osc_7770_with_any_other_text_leaves_the_shell_alone() {
    let mut session = fed(b"\x1b]7770;evil\x07");
    assert_eq!(session.shell(), ResolvedShell::Auto);
    session.feed(b"\x1b]7770\x07\x1b]7770;BASH\x07", now());
    assert_eq!(session.shell(), ResolvedShell::Auto);
    assert!(session.take_outbox().is_empty());
}

#[test]
fn other_osc_numbers_do_not_name_a_shell() {
    let session = fed(b"\x1b]7771;bash\x07\x1b]0;bash\x07");
    assert_eq!(session.shell(), ResolvedShell::Auto);
}

#[test]
fn note_strips_control_characters() {
    let mut session = fed(b"");
    session.note("a\x1bb");
    let snapshot = session.snapshot(now());
    let note = snapshot
        .rows()
        .iter()
        .find(|row| row_text(row) == "ab")
        .expect("the note is on screen without its escape");
    let style = note.style_of(&note.cells[0]);
    assert!(style.attrs.contains(Attrs::DIM));
}

#[test]
fn scrollback_is_capped() {
    let mut session = TerminalSession::new(GridSize { cols: 80, rows: 24 });
    let lines: String = (0..6_000).map(|line| format!("line {line}\r\n")).collect();
    session.feed(lines.as_bytes(), now());
    assert_eq!(session.term.screen().history_len(), SCROLLBACK_LINES);
}

#[test]
fn resize_reports_only_real_changes() {
    let mut session = fed(b"");
    assert!(!session.resize(SIZE));
    let wider = GridSize { cols: 30, rows: 5 };
    assert!(session.resize(wider));
    assert!(!session.resize(wider));
    assert_eq!(session.snapshot(now()).size().cols, 30);
}

#[test]
fn hostile_stream_never_panics() {
    let mut session = fed(b"");
    // xorshift64 with a fixed seed: the same hostile bytes on every run.
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut bytes = Vec::with_capacity(1 << 20);
    while bytes.len() < 1 << 20 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    for chunk in bytes.chunks(4096) {
        session.feed(chunk, now());
        session.take_outbox();
    }
    let rows = session.snapshot(now()).rows().len();
    assert_eq!(rows, usize::from(session.term.size().rows));
}

#[test]
fn terminal_palette_uses_theme_tokens_only() {
    for theme in [ThemeColor::light(), ThemeColor::dark()] {
        let palette = terminal_palette(&theme);
        let expected = [
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
        for (index, color) in expected.into_iter().enumerate() {
            assert_eq!(palette.indexed[index], rgb_of(color), "ANSI color {index}");
        }
        assert_eq!(palette.foreground, rgb_of(theme.foreground));
        assert_eq!(palette.background, rgb_of(theme.background));
        assert_eq!(palette.cursor, rgb_of(theme.caret));
        // The cube and the greys are the engine's defaults.
        assert_eq!(palette.indexed[16..], Palette::new().indexed[16..]);
    }
}

// ---- find, selection, and scrolling (step 3b) ----

/// Ten lines of which three hold `api`, on a screen of five rows, so the oldest match scrolls
/// out of view.
fn session_with_three_api_lines() -> TerminalSession {
    let mut text = String::new();
    for line in [
        "api one",
        "x",
        "x",
        "x",
        "x",
        "x",
        "api two",
        "x",
        "x",
        "api three",
    ] {
        text.push_str(line);
        text.push_str("\r\n");
    }
    fed(text.as_bytes())
}

#[test]
fn find_reports_match_count_and_order() {
    let mut session = session_with_three_api_lines();
    assert_eq!(session.find("API"), 3, "ASCII case is ignored");
    // The newest match is the first one the user is on.
    assert_eq!(session.find_status(), Some((1, 3)));
    let rows: Vec<u64> = session.find_matches().iter().map(|m| m.row.0).collect();
    assert!(
        rows[0] > rows[1] && rows[1] > rows[2],
        "newest first: {rows:?}"
    );
    session.find_next();
    assert_eq!(session.find_status(), Some((2, 3)));
    session.find_next();
    session.find_next();
    assert_eq!(
        session.find_status(),
        Some((1, 3)),
        "next wraps to the newest"
    );
    session.find_previous();
    assert_eq!(
        session.find_status(),
        Some((3, 3)),
        "previous wraps to the oldest"
    );
}

#[test]
fn find_scrolls_the_current_match_into_view() {
    let mut session = session_with_three_api_lines();
    session.find("api");
    let oldest_is_visible = |session: &mut TerminalSession| {
        let top = session.snapshot(now()).viewport_top();
        let found = session.current_find_match().expect("a current match");
        let rows = i64::from(SIZE.rows);
        let row = i64::try_from(found.row.0).unwrap() - i64::try_from(top.0).unwrap();
        (0..rows).contains(&row)
    };
    assert!(oldest_is_visible(&mut session), "the newest match is shown");
    session.find_next();
    session.find_next();
    assert_eq!(session.find_status(), Some((3, 3)));
    assert!(
        oldest_is_visible(&mut session),
        "the oldest match scrolled into view"
    );
}

#[test]
fn an_empty_or_unmatched_query_leaves_no_matches() {
    let mut session = session_with_three_api_lines();
    assert_eq!(session.find("nothing here"), 0);
    assert_eq!(session.find_status(), None);
    session.find("api");
    assert_eq!(session.find(""), 0);
    assert!(session.find_matches().is_empty());
    session.find("api");
    session.clear_find();
    assert_eq!(session.find_status(), None);
}

#[test]
fn a_selection_reads_back_the_text_under_it() {
    let mut session = fed(b"hello world\r\nsecond line");
    assert_eq!(session.selected_text(), None);
    session.begin_selection(0., 0., SelectionKind::Simple);
    session.extend_selection(0., 4.5);
    assert_eq!(session.selected_text().as_deref(), Some("hello"));
    session.clear_selection();
    assert_eq!(session.selected_text(), None);
    // A double click selects the word, a triple click the line.
    session.begin_selection(0., 7., SelectionKind::Semantic);
    assert_eq!(session.selected_text().as_deref(), Some("world"));
    session.begin_selection(1., 2., SelectionKind::Lines);
    // A line selection keeps its line end.
    assert_eq!(session.selected_text().as_deref(), Some("second line\n"));
}

#[test]
fn scrolling_shows_history_and_any_key_returns_to_the_bottom() {
    let mut text = String::new();
    for n in 0..30 {
        text.push_str(&format!("line {n}\r\n"));
    }
    let mut session = fed(text.as_bytes());
    let bottom = screen_text(&mut session);
    session.scroll_lines(-3);
    let scrolled = screen_text(&mut session);
    assert_ne!(bottom, scrolled, "the view moved into history");
    session.scroll_to_bottom();
    assert_eq!(screen_text(&mut session), bottom);
}

#[test]
fn modes_report_bracketed_paste() {
    let mut session = fed(b"");
    assert!(!session.modes().bracketed_paste);
    session.feed(b"\x1b[?2004h", now());
    assert!(session.modes().bracketed_paste);
}
