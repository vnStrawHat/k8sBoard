use cluster::GridSize;

use super::*;
use crate::terminal_session::TerminalSession;

fn typed(key: &str) -> Option<(KeySpec, KeyMods)> {
    key_to_vt(&Keystroke::parse(key).expect("a valid keystroke"))
}

/// The bytes the key sends to a fresh terminal.
fn sent(key: &str) -> Option<Vec<u8>> {
    let session = TerminalSession::new(GridSize { cols: 80, rows: 24 }, 5_000);
    let (spec, mods) = typed(key)?;
    session.encode_key(&spec, mods)
}

#[test]
fn key_to_vt_maps_named_keys() {
    for (key, named) in [
        ("enter", NamedKey::Enter),
        ("tab", NamedKey::Tab),
        ("backspace", NamedKey::Backspace),
        ("escape", NamedKey::Escape),
        ("up", NamedKey::ArrowUp),
        ("left", NamedKey::ArrowLeft),
        ("pagedown", NamedKey::PageDown),
        ("delete", NamedKey::Delete),
        ("f5", NamedKey::F5),
    ] {
        let (spec, _) = typed(key).unwrap_or_else(|| panic!("{key} has a mapping"));
        assert_eq!(spec, KeySpec::Named(named), "{key}");
    }
}

#[test]
fn key_to_vt_sends_plain_text_and_the_shift_of_a_chord() {
    assert_eq!(sent("l"), Some(b"l".to_vec()));
    assert_eq!(sent("space"), Some(b" ".to_vec()));
    assert_eq!(sent("enter"), Some(b"\r".to_vec()));
    assert_eq!(sent("escape"), Some(b"\x1b".to_vec()));
    assert_eq!(sent("up"), Some(b"\x1b[A".to_vec()));
}

#[test]
fn key_to_vt_sends_ctrl_chords_and_drops_platform_chords() {
    assert_eq!(sent("ctrl-c"), Some(vec![0x03]));
    assert_eq!(sent("ctrl-k"), Some(vec![0x0b]));
    assert_eq!(sent("ctrl-w"), Some(vec![0x17]));
    assert_eq!(sent("alt-b"), Some(b"\x1bb".to_vec()));
    assert!(typed("cmd-k").is_none());
    assert!(typed("cmd-c").is_none());
}

#[test]
fn key_to_vt_ignores_keys_with_no_encoding() {
    assert!(typed("shift").is_none());
    assert!(typed("capslock").is_none());
}

#[test]
fn sanitize_paste_strips_c0_and_c1_controls() {
    assert_eq!(sanitize_paste("a\x03b\u{9b}c\nd", false), "abc\rd");
    // An escape cannot start a sequence, and the bracket end cannot be forged.
    assert_eq!(sanitize_paste("x\x1b[201~y", false), "x[201~y");
    assert_eq!(sanitize_paste("tab\there", false), "tab\there");
}

#[test]
fn sanitize_paste_maps_line_ends_to_cr_when_not_bracketed() {
    assert_eq!(sanitize_paste("a\r\nb\nc\rd", false), "a\rb\rc\rd");
}

#[test]
fn sanitize_paste_keeps_newlines_when_bracketed() {
    assert_eq!(sanitize_paste("a\r\nb", true), "\x1b[200~a\r\nb\x1b[201~");
    // The wrapper is the only escape in the result.
    assert_eq!(
        sanitize_paste("a\x1b[201~b", true),
        "\x1b[200~a[201~b\x1b[201~"
    );
}

#[test]
fn a_single_line_paste_goes_straight_in() {
    assert_eq!(
        decide_paste("ls -la", false),
        PasteDecision::Send(b"ls -la".to_vec())
    );
    // The trailing newline is the Enter of one pasted line.
    assert_eq!(
        decide_paste("ls\n", false),
        PasteDecision::Send(b"ls\r".to_vec())
    );
}

#[test]
fn multi_line_paste_asks_first_unless_bracketed() {
    let PasteDecision::Ask(ask) = decide_paste("a\nb\n", false) else {
        panic!("two lines must ask");
    };
    assert_eq!(ask.lines, 2);
    assert_eq!(ask.preview, ["a", "b"]);
    assert_eq!(ask.bytes, b"a\rb\r");
    // One line, with the Enter that ends it, does not; bracketed mode never does.
    assert!(matches!(
        decide_paste("ls\n", false),
        PasteDecision::Send(_)
    ));
    assert!(matches!(decide_paste("a\nb", true), PasteDecision::Send(_)));
}

#[test]
fn the_preview_shows_the_first_five_lines_of_many() {
    let text = (1..=9)
        .map(|n| format!("l{n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let PasteDecision::Ask(ask) = decide_paste(&text, false) else {
        panic!("nine lines must ask");
    };
    assert_eq!(ask.lines, 9);
    assert_eq!(ask.preview, ["l1", "l2", "l3", "l4", "l5"]);
}

#[test]
fn a_bracketed_paste_never_asks() {
    let PasteDecision::Send(bytes) = decide_paste("a\nb\nc", true) else {
        panic!("bracketed mode holds the text in the shell");
    };
    assert_eq!(bytes, b"\x1b[200~a\nb\nc\x1b[201~");
}

#[test]
fn a_paste_of_only_controls_sends_nothing() {
    assert_eq!(decide_paste("\x03\x1b", false), PasteDecision::Nothing);
    assert_eq!(decide_paste("", true), PasteDecision::Nothing);
}

#[test]
fn an_oversized_paste_is_refused_whole() {
    let text = "x".repeat(MAX_PASTE_BYTES + 1);
    assert_eq!(decide_paste(&text, false), PasteDecision::TooLarge);
    assert_eq!(decide_paste(&text, true), PasteDecision::TooLarge);
}

fn altgr(key: &str, key_char: Option<&str>) -> Keystroke {
    let mut keystroke = Keystroke::parse(key).expect("a valid keystroke");
    keystroke.key_char = key_char.map(str::to_owned);
    keystroke
}

#[test]
fn altgr_types_the_character_the_layout_produced() {
    // A German layout types @ with AltGr+Q: Windows reports Ctrl+Alt, and the character with it.
    for (key, text) in [
        ("q", "@"),
        ("7", "{"),
        ("0", "}"),
        ("8", "["),
        ("ß", "\\"),
        ("<", "|"),
    ] {
        let keystroke = altgr(&format!("ctrl-alt-{key}"), Some(text));
        let (spec, mods) =
            key_to_vt_as(&keystroke, CtrlAltMeans::AltGr).expect("AltGr sends its text");
        assert_eq!(spec, KeySpec::Character(text.to_owned()), "{key}");
        assert_eq!(
            mods,
            KeyMods::default(),
            "{key}: no modifiers, no ESC prefix"
        );
    }
}

#[test]
fn a_real_ctrl_alt_chord_stays_a_chord() {
    // No printable character came with it, or the platform does not read it as AltGr.
    let chord = altgr("ctrl-alt-b", None);
    let (spec, mods) = key_to_vt_as(&chord, CtrlAltMeans::AltGr).expect("a chord");
    assert_eq!(spec, KeySpec::Character("b".to_owned()));
    assert!(mods.ctrl && mods.alt);
    let control = altgr("ctrl-alt-b", Some("\u{2}"));
    let (_, mods) = key_to_vt_as(&control, CtrlAltMeans::AltGr).expect("a chord");
    assert!(mods.ctrl && mods.alt);
    let typed = altgr("ctrl-alt-q", Some("@"));
    let (spec, mods) = key_to_vt_as(&typed, CtrlAltMeans::Chord).expect("a chord");
    assert_eq!(spec, KeySpec::Character("q".to_owned()));
    assert!(mods.ctrl && mods.alt);
}

#[test]
fn altgr_with_the_platform_key_is_still_the_apps() {
    let keystroke = altgr("ctrl-alt-cmd-q", Some("@"));
    assert!(key_to_vt_as(&keystroke, CtrlAltMeans::AltGr).is_none());
}

#[test]
fn paste_debug_prints_sizes_and_never_the_text() {
    let ask = decide_paste("secret one\nsecret two\n", false);
    let shown = format!("{ask:?}");
    assert!(!shown.contains("secret"), "{shown}");
    assert!(shown.contains("bytes"), "{shown}");
    let sent = format!("{:?}", decide_paste("secret", false));
    assert_eq!(sent, "Send(6 bytes)");
}
