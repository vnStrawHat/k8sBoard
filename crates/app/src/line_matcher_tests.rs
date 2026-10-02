use super::*;

fn parsed(text: &str, mode: FilterMode) -> LineMatcher {
    LineMatcher::parse(text, mode)
        .expect("valid pattern")
        .expect("a matcher")
}

#[test]
fn plain_matcher_matches_ignoring_ascii_case() {
    let matcher = parsed("Error", FilterMode::Plain);
    assert!(matcher.is_match("an ERROR happened"));
    assert!(!matcher.is_match("fine"));
    let ranges = matcher.ranges("an ERROR");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges.first(), Some(&(3..8)));
    assert_eq!(matcher.pattern(), "Error");
}

#[test]
fn regex_matcher_matches_alternation_case_insensitively() {
    let matcher = parsed("error|timeout", FilterMode::Regex);
    assert!(matcher.is_match("Upstream TIMEOUT"));
    assert!(!matcher.is_match("all good"));
    assert_eq!(matcher.ranges("Error then timeout"), [0..5, 11..18]);
    assert_eq!(matcher.pattern(), "error|timeout");
}

#[test]
fn invalid_regex_is_an_error() {
    assert!(matches!(
        LineMatcher::parse("a|(", FilterMode::Regex),
        Err(InvalidRegex)
    ));
    assert_eq!(InvalidRegex.to_string(), "Invalid regex");
}

#[test]
fn blank_text_parses_to_no_matcher() {
    for mode in [FilterMode::Plain, FilterMode::Regex] {
        assert!(matches!(LineMatcher::parse("", mode), Ok(None)));
        assert!(matches!(LineMatcher::parse("  \t", mode), Ok(None)));
    }
}

#[test]
fn regex_ranges_skip_empty_matches() {
    let matcher = parsed("x?", FilterMode::Regex);
    assert!(matcher.is_match("abc"));
    assert!(matcher.ranges("abc").is_empty());
}

#[test]
fn oversized_regex_is_rejected() {
    assert!(matches!(
        LineMatcher::parse(r"(\w{1000}){1000}", FilterMode::Regex),
        Err(InvalidRegex)
    ));
}

#[test]
fn find_matches_returns_non_overlapping_ranges() {
    let ranges = find_matches("aaa", "aa");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges.first(), Some(&(0..2)));
    assert_eq!(find_matches("a-A-a", "a"), [0..1, 2..3, 4..5]);
    assert!(find_matches("abc", "").is_empty());
}

#[test]
fn find_matches_on_non_ascii_text_keeps_char_boundaries() {
    let text = "Ärger error";
    let ranges = find_matches(text, "error");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges.first(), Some(&(7..12)));
    assert!(
        ranges
            .iter()
            .all(|range| text.is_char_boundary(range.start))
    );
    assert!(find_matches("Ärger", "ärger").is_empty());
}
