//! What the user sends to a shell (spec 0036): keystrokes mapped to the engine's key model, and
//! pasted text cleaned of control characters. Pure: no session, no clipboard, no GPUI view.

use gpui_kit::Keystroke;
use oneterm_vt::input::{KeyMods, KeySpec, NamedKey};

/// The most one paste may carry. The transport buffers a paste whole, so an unbounded one could
/// hold the whole clipboard in the input queue.
pub(crate) const MAX_PASTE_BYTES: usize = 128 * 1024;
/// How many lines the multi-line paste dialog previews.
const PREVIEW_LINES: usize = 5;

const BRACKET_START: &str = "\x1b[200~";
const BRACKET_END: &str = "\x1b[201~";

/// The engine's name of a key, or `None` for a chord the terminal never sends: the platform key
/// (Cmd or Win) is the app's, and a bare modifier or an unknown key has no bytes.
pub(crate) fn key_to_vt(keystroke: &Keystroke) -> Option<(KeySpec, KeyMods)> {
    let modifiers = keystroke.modifiers;
    if modifiers.platform {
        return None;
    }
    let mods = KeyMods {
        shift: modifiers.shift,
        ctrl: modifiers.control,
        alt: modifiers.alt,
    };
    if let Some(named) = named_key(&keystroke.key) {
        return Some((KeySpec::Named(named), mods));
    }
    // A chord names its base key: with Ctrl or Alt held the platform may report a control or
    // dead-key character instead of the letter.
    let has_chord = modifiers.control || modifiers.alt;
    let text = if has_chord {
        base_key(&keystroke.key)?
    } else {
        keystroke
            .key_char
            .clone()
            .or_else(|| base_key(&keystroke.key))?
    };
    Some((KeySpec::Character(text), mods))
}

/// The text of a key that is one character; `space` is the character itself.
fn base_key(key: &str) -> Option<String> {
    if key == "space" {
        return Some(" ".to_owned());
    }
    (key.chars().count() == 1).then(|| key.to_owned())
}

fn named_key(key: &str) -> Option<NamedKey> {
    Some(match key {
        "enter" => NamedKey::Enter,
        "tab" => NamedKey::Tab,
        "backspace" => NamedKey::Backspace,
        "escape" => NamedKey::Escape,
        "up" => NamedKey::ArrowUp,
        "down" => NamedKey::ArrowDown,
        "left" => NamedKey::ArrowLeft,
        "right" => NamedKey::ArrowRight,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        "insert" => NamedKey::Insert,
        "delete" => NamedKey::Delete,
        "f1" => NamedKey::F1,
        "f2" => NamedKey::F2,
        "f3" => NamedKey::F3,
        "f4" => NamedKey::F4,
        "f5" => NamedKey::F5,
        "f6" => NamedKey::F6,
        "f7" => NamedKey::F7,
        "f8" => NamedKey::F8,
        "f9" => NamedKey::F9,
        "f10" => NamedKey::F10,
        "f11" => NamedKey::F11,
        "f12" => NamedKey::F12,
        _ => return None,
    })
}

/// Drops every C0 control except tab, CR, and LF, and every C1 control, so no escape sequence
/// and no `ETX` survives a paste. Not bracketed: line ends become CR, which is what Enter sends.
/// Bracketed: the text passes as is, wrapped so the shell holds it until Enter.
pub(crate) fn sanitize_paste(text: &str, is_bracketed: bool) -> String {
    let kept: String = text
        .chars()
        .filter(|character| matches!(character, '\t' | '\r' | '\n') || !character.is_control())
        .collect();
    if is_bracketed {
        return format!("{BRACKET_START}{kept}{BRACKET_END}");
    }
    kept.replace("\r\n", "\r").replace('\n', "\r")
}

/// What a paste does.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PasteDecision {
    /// Nothing is left after cleaning.
    Nothing,
    /// More than `MAX_PASTE_BYTES`: refused whole, because a cut paste is a different command.
    TooLarge,
    Send(Vec<u8>),
    /// Several lines into a program that did not ask for bracketed paste: each line would run as it
    /// arrives, so the user sees the text first.
    Ask(PasteAsk),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PasteAsk {
    pub(crate) lines: usize,
    /// The first lines, for the dialog.
    pub(crate) preview: Vec<String>,
    /// What `Paste` sends.
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn decide_paste(text: &str, is_bracketed: bool) -> PasteDecision {
    if text.len() > MAX_PASTE_BYTES {
        return PasteDecision::TooLarge;
    }
    let plain = sanitize_paste(text, false);
    if plain.is_empty() {
        return PasteDecision::Nothing;
    }
    if is_bracketed {
        return PasteDecision::Send(sanitize_paste(text, true).into_bytes());
    }
    let bytes = plain.into_bytes();
    // One trailing line end is the Enter of a single pasted line.
    let body = bytes.strip_suffix(b"\r").unwrap_or(&bytes);
    if !body.contains(&b'\r') {
        return PasteDecision::Send(bytes);
    }
    let lines: Vec<String> = String::from_utf8_lossy(body)
        .split('\r')
        .map(str::to_owned)
        .collect();
    PasteDecision::Ask(PasteAsk {
        lines: lines.len(),
        preview: lines.into_iter().take(PREVIEW_LINES).collect(),
        bytes,
    })
}

#[cfg(test)]
#[path = "terminal_input_tests.rs"]
mod terminal_input_tests;
