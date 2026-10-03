//! The small subsequence scorer behind the command palette. The candidate set is a few thousand
//! short names, so a greedy alignment is fast enough and needs no matcher dependency.

/// Every matched character earns this.
const MATCH: u32 = 1;
/// A matched character that starts a word: index 0, or after `-` `/` `.` `_` or a space.
const WORD_START: u32 = 6;
/// A matched character right after the previous match: a typed run of the name.
const CONSECUTIVE: u32 = 16;
/// The first match is at index 0. Index 0 also counts as a word start, so this is the only
/// extra credit for being first.
const PREFIX_BONUS: u32 = 10;
/// The needle equals the haystack.
const EXACT_BONUS: u32 = 20;
/// A skipped character between two matches costs this, up to `MAX_GAP_PENALTY` per gap.
const GAP_PENALTY: u32 = 1;
const MAX_GAP_PENALTY: u32 = 3;

/// One unit of text the scorer compares: a byte of an ASCII text, or a `char` of any other.
trait Unit: Copy {
    /// ASCII case-insensitive; other characters compare exactly.
    fn same(self, other: Self) -> bool;
    fn is_separator(self) -> bool;
}

impl Unit for u8 {
    fn same(self, other: Self) -> bool {
        self.eq_ignore_ascii_case(&other)
    }

    fn is_separator(self) -> bool {
        matches!(self, b'-' | b'/' | b'.' | b'_' | b' ')
    }
}

impl Unit for char {
    fn same(self, other: Self) -> bool {
        self.eq_ignore_ascii_case(&other)
    }

    fn is_separator(self) -> bool {
        matches!(self, '-' | '/' | '.' | '_' | ' ')
    }
}

/// `None` when `needle` is not a subsequence of `haystack`, or when it only matches scattered
/// inside words: the match must be a contiguous run of `haystack`, or start on a word start. An
/// empty needle matches with 0. Kubernetes names are ASCII, so the common case compares bytes
/// and allocates nothing; any other text falls back to `char`s.
///
/// ponytail: greedy alignment, not optimal (no DP); switch to a Smith-Waterman pass or nucleo if
/// users report misses.
pub(crate) fn fuzzy_score(needle: &str, haystack: &str) -> Option<u32> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.is_ascii() && haystack.is_ascii() {
        return score(needle.as_bytes(), haystack.as_bytes());
    }
    let haystack: Vec<char> = haystack.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    score(&needle, &haystack)
}

fn score<T: Unit>(needle: &[T], haystack: &[T]) -> Option<u32> {
    // A word start picked ahead of a nearer hit can use up the later characters, so the plain
    // leftmost alignment is the fallback that always finds a subsequence.
    let found = align(needle, haystack, true).or_else(|| align(needle, haystack, false))?;
    // "rest" is a subsequence of "previous dock tab", but nobody means it: a scattered hit that
    // starts mid-word and is no run of the text is noise.
    let is_plausible = found.starts_at_word_start
        || found.is_run
        || haystack.windows(needle.len()).any(|window| {
            window
                .iter()
                .zip(needle)
                .all(|(found, wanted)| wanted.same(*found))
        });
    is_plausible.then_some(found.score)
}

struct Alignment {
    score: u32,
    starts_at_word_start: bool,
    /// Every needle unit matched right after the previous one.
    is_run: bool,
}

fn align<T: Unit>(needle: &[T], haystack: &[T], prefers_word_starts: bool) -> Option<Alignment> {
    let mut score = 0_u32;
    let mut from = 0_usize;
    let mut previous: Option<usize> = None;
    let mut starts_at_word_start = false;
    let mut is_run = true;
    for wanted in needle {
        let found = next_hit(*wanted, haystack, from, prefers_word_starts)?;
        score = score.saturating_add(MATCH);
        if is_word_start(haystack, found) {
            score = score.saturating_add(WORD_START);
        }
        match previous {
            Some(last) if found == last + 1 => score = score.saturating_add(CONSECUTIVE),
            Some(last) => {
                is_run = false;
                let gap = u32::try_from(found - last - 1).unwrap_or(u32::MAX);
                score = score.saturating_sub(gap.saturating_mul(GAP_PENALTY).min(MAX_GAP_PENALTY));
            }
            None => {
                starts_at_word_start = is_word_start(haystack, found);
                if found == 0 {
                    score = score.saturating_add(PREFIX_BONUS);
                }
            }
        }
        previous = Some(found);
        from = found + 1;
    }
    // A run that is as long as the text starts at 0 and equals it: the needle is the whole text.
    if is_run && needle.len() == haystack.len() {
        score = score.saturating_add(EXACT_BONUS);
    }
    Some(Alignment {
        score,
        starts_at_word_start,
        is_run,
    })
}

/// The next position of `wanted` at or after `from`. A preferring alignment takes a later word
/// start over a nearer mid-word hit, except where the hit continues the run of the previous match
/// (`from > 0` and the hit is at `from`): a typed run beats a word start further on.
fn next_hit<T: Unit>(
    wanted: T,
    haystack: &[T],
    from: usize,
    prefers_word_starts: bool,
) -> Option<usize> {
    let mut hits = (from..haystack.len()).filter(|&index| wanted.same(haystack[index]));
    let first = hits.next()?;
    let continues_run = from > 0 && first == from;
    if !prefers_word_starts || continues_run || is_word_start(haystack, first) {
        return Some(first);
    }
    Some(
        hits.find(|&index| is_word_start(haystack, index))
            .unwrap_or(first),
    )
}

fn is_word_start<T: Unit>(haystack: &[T], index: usize) -> bool {
    index == 0 || haystack[index - 1].is_separator()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_score_rejects_a_non_subsequence() {
        assert_eq!(fuzzy_score("xyz", "payments-api"), None);
        // Order matters: the letters exist but not in this order.
        assert_eq!(fuzzy_score("ipa", "api"), None);
    }

    #[test]
    fn fuzzy_score_ignores_ascii_case() {
        assert_eq!(
            fuzzy_score("PAY", "payments-api"),
            fuzzy_score("pay", "payments-api")
        );
        assert!(fuzzy_score("pay", "payments-api").is_some());
        assert!(fuzzy_score("pay", "PAYMENTS-API").is_some());
    }

    #[test]
    fn fuzzy_score_prefers_word_starts() {
        assert!(fuzzy_score("pa", "payments-api") > fuzzy_score("pa", "deploy-pa"));
        assert!(fuzzy_score("dp", "deploy-pa") > fuzzy_score("dp", "adpx"));
    }

    #[test]
    fn fuzzy_score_counts_index_zero_once() {
        let first = fuzzy_score("p", "p-q");
        let after_separator = fuzzy_score("q", "p-q");
        assert_eq!(
            first
                .zip(after_separator)
                .map(|(first, second)| first - second),
            Some(PREFIX_BONUS)
        );
    }

    #[test]
    fn fuzzy_score_prefers_consecutive_matches() {
        assert!(fuzzy_score("api", "payments-api") > fuzzy_score("api", "a-p-i"));
    }

    #[test]
    fn fuzzy_score_ranks_exact_then_prefix_highest() {
        let exact = fuzzy_score("pods", "pods");
        let prefix = fuzzy_score("pods", "podsx");
        let scattered = fuzzy_score("pods", "prods");
        assert!(exact > prefix);
        assert!(prefix > scattered);
        assert!(scattered.is_some());
    }

    #[test]
    fn fuzzy_score_empty_needle_matches_with_zero() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert_eq!(fuzzy_score("", ""), Some(0));
    }

    #[test]
    fn fuzzy_score_falls_back_when_a_word_start_would_use_up_the_text() {
        // The word start of the second `c` leaves nothing for `b`; the leftmost alignment works.
        assert!(fuzzy_score("cb", "xcb-c").is_some());
    }

    #[test]
    fn fuzzy_score_rejects_scattered_mid_word_hits() {
        // A subsequence, but spread over three words and starting mid-word.
        assert_eq!(fuzzy_score("rest", "previous dock tab"), None);
        assert_eq!(fuzzy_score("po", "deployments"), None);
        // A run inside a word, and a scattered match that starts on a word start, both stay.
        assert!(fuzzy_score("dock", "previous dock tab").is_some());
        assert!(fuzzy_score("ock", "previous dock tab").is_some());
        assert!(fuzzy_score("rest", "restart rollout").is_some());
        assert!(fuzzy_score("rr", "restart rollout").is_some());
    }

    #[test]
    fn fuzzy_score_keeps_a_later_run_when_the_first_hit_is_scattered() {
        // The leftmost alignment of `ab` is scattered, but the text holds the run `ab`.
        assert!(fuzzy_score("ab", "xaxbxab").is_some());
    }

    #[test]
    fn fuzzy_score_compares_non_ascii_text_by_character() {
        assert!(fuzzy_score("é", "café").is_some());
        assert!(fuzzy_score("É", "café").is_none());
        assert!(fuzzy_score("caf", "café").is_some());
    }
}
