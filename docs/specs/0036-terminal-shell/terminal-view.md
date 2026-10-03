# 0036 · Terminal core and GPUI view

[Back to index](README.md) · Steps 2–3 · Modules: `terminal_session.rs` (new; tests `terminal_session_tests.rs`), `terminal_input.rs` (new; tests in module), `terminal_element.rs` (new). Decisions 14–22.

## Session core (`terminal_session.rs`, no GPUI types except `App` for the theme)

```rust
pub(crate) struct TerminalSession { term: Terminal, batch: EventBatch, snapshot: SnapshotState,
    palette: Palette, outbox: Vec<u8> }
impl TerminalSession {
    pub(crate) fn new(size: GridSize) -> Self;          // Config { scrollback_limit: SCROLLBACK_LINES, product_name: Some("k8sBoard"), allow_screen_readback: false, osc_routes: (7770 → Forward), ..Config::default() }
    /// Feeds remote bytes; queues the engine's replies (DA, DSR) in `outbox`; applies the event policy.
    pub(crate) fn feed(&mut self, bytes: &[u8], now: Instant);
    pub(crate) fn take_outbox(&mut self) -> Vec<u8>;    // sent as `ShellInput::Bytes`
    pub(crate) fn note(&mut self, text: &str);          // a local dim line: banner, exit, errors
    pub(crate) fn resize(&mut self, size: GridSize) -> bool; // true when the size changed
    pub(crate) fn snapshot(&mut self, now: Instant) -> &SnapshotState; // update + map_colors
    pub(crate) fn set_palette(&mut self, palette: Palette);
}
pub(crate) const SCROLLBACK_LINES: u32 = 5_000;
```

- One `Terminal` per Shell tab, owned by the tab entity on the GPUI thread; parsing stays on the main thread, with no lock (vt guide ch. 3 allows any owner). Each update is at most one 64 KiB read, and `feed` costs about 12 ns/byte in release builds (vt README), so one update costs about 1 ms or less: that is the budget, checked by a bulk-output timing in the allowed-path live check (`cat` of a 50 MB file, trace of feed time per update). Debug builds are 10–30× slower; ui-verifier runs (debug, `screenshot` feature) should expect sluggish bulk output.
- `// ponytail: parse on the UI thread; if bulk output stalls frames, move feed to a reader thread with the Terminal behind a mutex (vt guide ch. 3 two-phase hand-off).`
- `allow_screen_readback` is pinned to `false` (DECRQCRA would let a program read back screen text it did not write, such as a password prompt); test `decrqcra_is_not_answered`.
- Event policy (C1, "not a policy" engine): `Reply` → outbox; `ClipboardStore` (OSC 52 write) ignored; `ClipboardLoad` never answered (the remote never reads the local clipboard); `ColorQuery { key, terminator }` answered from `terminal_palette` with the query's own terminator (OSC 10/11/12/4 reply format of vt guide ch. 4); `Osc` 7770 (routed `Forward` in `Config::osc_routes`) sets the resolved shell name when it is `bash`, `ash`, or `sh` ([shell-tab.md](shell-tab.md)); every other `Osc`, and `Title`, `Bell`, `Cwd`, `Notification`, `Progress`, `ShellMark`, are ignored.
- `note` feeds `\r\n\x1b[2m{text}\x1b[0m\r\n` with `text` stripped of control characters. The first note is the W8b banner `# exec -n {ns} {pod} -c {container} -- {shell} ({context})`.
- Nothing from the session is logged, traced, persisted, or kept after the tab closes (C1).
- Limits (W2 security review, 2026-10-03): `feed` discards every decoded Sixel image (`take_graphics`), because 0036 draws none and the engine keeps them until taken; the outbox holds at most 64 KiB of replies until drained, and a reply past that is dropped; `log_filter.rs` pins the `tungstenite` and `tokio_tungstenite` targets at `info`, because they trace every WebSocket frame payload at `trace`, which would put keystrokes in the log.

## Palette from the kit theme (`terminal_palette(theme: &Theme) -> Palette`, pure)

| Index | Theme token | Index | Theme token |
|---|---|---|---|
| 0 | `muted` | 8 | `muted_foreground` |
| 1–6 | `red`, `green`, `yellow`, `blue`, `magenta`, `cyan` | 9–14 | the `*_light` variants |
| 7 | `muted_foreground` | 15 | `foreground` |
| fg / bg / cursor | `foreground` / `background` / `caret` (or `primary`) | 16–255 | `Palette::new()` cube and greys |

Recomputed when the theme changes (observe the theme global); `set_palette` forces a full repaint (vt palette epoch). No hard-coded colour (0003 colour grep).

## Element (`terminal_element.rs`)

- A GPUI `Element` sized by its parent (flex fill). Font: `theme.mono_font_family`, `theme.mono_font_size`. Cell width = advance of `M`; cell height = `round(font_size × 1.3)`; both cached per font and size, passed to `set_cell_pixels`.
- `fn grid_size(bounds: Size<Pixels>, cell: Size<Pixels>) -> GridSize` (pure): floor division, minimum 2 × 1. Prepaint compares it with the session size; a change calls `resize(.., BottomAnchor)` and queues `ShellInput::Resize` (the SIGWINCH path). Resizes are coalesced: at most one per frame, the latest wins.
- Paint per visible row: background quads for runs whose bg is not the default; one `shape_line` per row with a `TextRun` per `StyleRun` (fg, bold, italic, underline, strikethrough, dim via the palette); wide cells use `SnapshotRow::cluster`. Then the selection (theme `selection`), Find matches (theme `warning` at 40 % alpha), and the cursor (`SnapshotCursor`: block, bar, underline; hidden when `show_cursor` is off; no blink).
- `// ponytail: reshapes every visible row each frame; cache per row on `changed()` if a profile shows frame time > 4 ms at 200 × 60.`

## Input (`terminal_input.rs`, pure mapping + element listeners)

- Focus: the element has the tab's `FocusHandle` and `key_context("Terminal")`; a click focuses it.
- `fn key_to_vt(keystroke: &Keystroke) -> Option<(KeySpec, KeyMods)>`: named keys (enter, tab, backspace, escape, arrows, home, end, pageup, pagedown, insert, delete, f1–f12) → `NamedKey`; else `key_char` (or `key` for ctrl chords) → `Character`. `platform` (⌘/Win) chords → `None` (never sent). Result → `encode_key` → `ShellInput::Bytes`; any key also calls `scroll_to_bottom`.
- Key routing with 0028 ([shell-tab.md](shell-tab.md) "Keys"): bytes come from `on_key_down`, which only sees keys no action consumed.
- Paste (`TerminalPaste`), `fn sanitize_paste(text: &str, bracketed: bool) -> String` (pure): drop every C0 control except `\t`, `\r`, `\n`, and drop every C1 control (U+0080–U+009F), so no escape sequence or `ETX` survives. Not bracketed: `\r\n` → `\r`, then bare `\n` → `\r`. Bracketed: newlines pass unchanged and the result is wrapped in `\x1b[200~ … \x1b[201~`. Fixture: `a\x03b\u{9b}c\nd` → `abc\rd` (not bracketed).
- Multi-line paste (every environment, no tier logic): when not bracketed and the sanitized text, after trimming one trailing newline, still contains a newline, a kit `Dialog` "Paste N lines?" shows a preview (first 5 lines, mono) and Enter or `Paste` sends it; Esc cancels. Bracketed mode skips the dialog (the shell holds the text until Enter).
- Mouse: left drag → `hit_test` + `selection_start/update` (double-click: word selection kind, triple: line); wheel → `scroll_viewport` on the primary screen, `encode_wheel_event` when the program enabled mouse reporting or alternate scroll on the alternate screen. Click reporting to programs: not in 0036 (open item).

## Copy (C1)

- `TerminalCopy` with a selection → `secret_clipboard::write_private_text(text)` (0016: Windows private formats, out of clipboard history and cloud sync), because shell output may hold secrets. No 30 s auto-clear: the private write is enough (user decision, decision 20). `ClipboardWriteError::Unavailable` shows 0016's fixed notice "The clipboard is busy; nothing was copied."; the text is never put in the notice. Without a selection the chord does nothing.
- No "copy on select", no clipboard reads except an explicit paste.

## Find

Header "Find" opens a kit `Input` in the tab header. Each query change: `GridText::from_terminal` → `search_grid_text(Literal, case-insensitive)`; Enter / Shift+Enter move to the next / previous match and scroll it into view; the header shows `3 of 12`; Esc closes Find and refocuses the terminal. Matches are recomputed on query change only (user-initiated; vt guide ch. 7).
