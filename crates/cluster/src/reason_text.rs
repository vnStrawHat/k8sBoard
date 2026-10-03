//! The one sanitizer for text that came from the server or a socket and reaches the UI.

pub(crate) const MAX_REASON_CHARS: usize = 200;

/// Control characters stripped and at most 200 characters, so a reason can neither carry an
/// escape sequence nor flood a status line.
pub(crate) fn reason_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .take(MAX_REASON_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_stripped_and_the_length_is_capped() {
        assert_eq!(reason_text("a\x1b[31mb\r\nc\x07"), "a[31mbc");
        assert_eq!(reason_text("plain text"), "plain text");
        assert_eq!(
            reason_text(&"x".repeat(500)).chars().count(),
            MAX_REASON_CHARS
        );
    }

    #[test]
    fn the_cap_counts_characters_not_bytes() {
        let wide = "é".repeat(300);
        assert_eq!(reason_text(&wide).chars().count(), MAX_REASON_CHARS);
    }
}
