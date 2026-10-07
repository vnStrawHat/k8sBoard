//! Text whose counts read in semibold: `Delete **3** pods · **1** cannot move`. The kit alert and
//! the dialog lines take a plain string, so the weight is a highlight on one text element, which
//! keeps wrapping and the color of the line.

use std::ops::Range;

use gpui_kit::{FontWeight, HighlightStyle, SharedString, StyledText};

/// `text` with every count in semibold.
pub(crate) fn counted_text(text: impl Into<SharedString>) -> StyledText {
    let text = text.into();
    let semibold = HighlightStyle {
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    let highlights = count_ranges(&text)
        .into_iter()
        .map(|range| (range, semibold))
        .collect::<Vec<_>>();
    StyledText::new(text).with_highlights(highlights)
}

/// The byte ranges of the counts in `text`: a run of digits that stands alone as a word. Digits
/// inside a name (`wk-04`, `api-7d9f8c`), a unit (`5m`), a fraction (`1/1`), or a clock (`3:12`)
/// are not counts. Pure.
fn count_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (ch.is_ascii_digit(), start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                start = None;
                let before = text[..from].chars().next_back();
                let is_word_start =
                    before.is_none_or(|c| c.is_whitespace() || c == '(' || c == '+');
                let is_word_end = text[index..]
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_whitespace() || c == ',' || c == ')');
                if is_word_start && is_word_end {
                    ranges.push(from..index);
                }
            }
            _ => {}
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(text: &str) -> Vec<&str> {
        count_ranges(text)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    #[test]
    fn the_counts_of_a_removal_line_are_found() {
        assert_eq!(counts("Delete 3 pods · 1 cannot move"), ["3", "1"]);
        assert_eq!(counts("Server dry-run passed for 12 of 12"), ["12", "12"]);
        assert_eq!(
            counts("Cancelled · 12 of 23 evicted on wk-04 (2 pending)"),
            ["12", "23", "2"]
        );
        assert_eq!(counts("Pods to evict · 23 · 1 cannot move"), ["23", "1"]);
        assert_eq!(counts("+3 more"), ["3"]);
        assert_eq!(counts("4"), ["4"]);
    }

    #[test]
    fn digits_in_names_units_fractions_and_clocks_are_not_counts() {
        assert_eq!(counts("Drain wk-04 · Timeout in 3:12"), Vec::<&str>::new());
        assert_eq!(counts("api-7d9f8c 5m 1/1 21 of 23 evictions"), ["21", "23"]);
        assert_eq!(counts("passed · 412 ms"), ["412"]);
        assert_eq!(counts("v1.29.5 on 10.0.0.1"), Vec::<&str>::new());
    }

    #[test]
    fn multibyte_text_keeps_the_ranges_on_char_boundaries() {
        // The ellipsis is three bytes, so the count after it is not at its character index.
        assert_eq!(counts("Evict… · 3 pods"), ["3"]);
    }
}
