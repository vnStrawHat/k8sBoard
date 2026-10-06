//! Log severity: heuristic detection from one line of text, and the set the chips hide.
//! No GPUI types.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub(crate) const ALL: [Self; 4] = [Self::Error, Self::Warn, Self::Info, Self::Debug];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
            Self::Info => 2,
            Self::Debug => 3,
        }
    }
}

/// The levels the chips hide; the default hides none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LevelSet {
    hidden: [bool; 4],
}

impl LevelSet {
    pub(crate) fn is_hidden(self, level: LogLevel) -> bool {
        self.hidden[level.index()]
    }

    pub(crate) fn toggled(mut self, level: LogLevel) -> Self {
        let slot = &mut self.hidden[level.index()];
        *slot = !*slot;
        self
    }

    /// The Alt-click: only `level` stays visible; when it already is the only one, every level
    /// comes back.
    pub(crate) fn toggled_only(self, level: LogLevel) -> Self {
        let mut only = Self { hidden: [true; 4] };
        only.hidden[level.index()] = false;
        if self == only { Self::default() } else { only }
    }

    pub(crate) fn hides_none(self) -> bool {
        self.hidden == [false; 4]
    }
}

/// The level a line carries, or `None` when nothing in it names one.
pub(crate) fn detect_level(text: &str) -> Option<LogLevel> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        // A JSON object that parses decides alone; text that only looks like one falls through.
        if let Ok(object) =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(trimmed)
        {
            return json_level(&object);
        }
    }
    logfmt_level(text)
        .or_else(|| klog_level(text))
        .or_else(|| keyword_level(text))
}

const JSON_LEVEL_KEYS: [&str; 4] = ["level", "severity", "lvl", "log.level"];
const LOGFMT_LEVEL_KEYS: [&str; 3] = ["level=", "lvl=", "severity="];
/// A level keyword sits in the prefix of a line; later words are message text.
const KEYWORD_WINDOW_BYTES: usize = 64;

fn json_level(object: &serde_json::Map<String, serde_json::Value>) -> Option<LogLevel> {
    let value = JSON_LEVEL_KEYS.iter().find_map(|key| object.get(*key))?;
    match value {
        serde_json::Value::String(word) => level_of_word(word),
        serde_json::Value::Number(number) => level_of_number(number.as_u64()?),
        _ => None,
    }
}

fn logfmt_level(text: &str) -> Option<LogLevel> {
    let value = text.split(' ').find_map(|token| {
        LOGFMT_LEVEL_KEYS
            .iter()
            .find_map(|key| token.strip_prefix(key))
    })?;
    level_of_word(value.trim_matches('"'))
}

/// `E0501 10:47:58.902 …`: a severity letter, four date digits, then a space.
fn klog_level(text: &str) -> Option<LogLevel> {
    let bytes = text.as_bytes();
    let head = bytes.get(..6)?;
    let is_klog = head[1..5].iter().all(u8::is_ascii_digit) && head[5] == b' ';
    if !is_klog {
        return None;
    }
    match head[0] {
        b'I' => Some(LogLevel::Info),
        b'W' => Some(LogLevel::Warn),
        b'E' | b'F' => Some(LogLevel::Error),
        _ => None,
    }
}

fn keyword_level(text: &str) -> Option<LogLevel> {
    let mut end = text.len().min(KEYWORD_WINDOW_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = &text[..end];
    let bytes = head.as_bytes();
    let is_word_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut start = 0;
    while start < bytes.len() {
        if !is_word_byte(bytes[start]) {
            start += 1;
            continue;
        }
        let mut word_end = start;
        while word_end < bytes.len() && is_word_byte(bytes[word_end]) {
            word_end += 1;
        }
        let word = &head[start..word_end];
        let before = start.checked_sub(1).map(|index| bytes[index]);
        let after = bytes.get(word_end).copied();
        let is_marked = !word.bytes().any(|byte| byte.is_ascii_lowercase())
            || (before == Some(b'[') && after == Some(b']'))
            || (before == Some(b'\t') && after == Some(b'\t'));
        if let (true, Some(level)) = (is_marked, level_of_word(word)) {
            return Some(level);
        }
        start = word_end;
    }
    None
}

fn level_of_word(word: &str) -> Option<LogLevel> {
    const WORDS: [(&str, LogLevel); 14] = [
        ("error", LogLevel::Error),
        ("err", LogLevel::Error),
        ("fatal", LogLevel::Error),
        ("panic", LogLevel::Error),
        ("crit", LogLevel::Error),
        ("critical", LogLevel::Error),
        ("alert", LogLevel::Error),
        ("emerg", LogLevel::Error),
        ("warn", LogLevel::Warn),
        ("warning", LogLevel::Warn),
        ("info", LogLevel::Info),
        ("notice", LogLevel::Info),
        ("debug", LogLevel::Debug),
        ("trace", LogLevel::Debug),
    ];
    WORDS
        .iter()
        .find(|(name, _)| word.eq_ignore_ascii_case(name))
        .map(|(_, level)| *level)
}

/// The pino and bunyan scale.
fn level_of_number(value: u64) -> Option<LogLevel> {
    match value {
        50 | 60 => Some(LogLevel::Error),
        40 => Some(LogLevel::Warn),
        30 => Some(LogLevel::Info),
        10 | 20 => Some(LogLevel::Debug),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_level_field_is_detected() {
        assert_eq!(
            detect_level(r#"{"level":"ERROR","msg":"x"}"#),
            Some(LogLevel::Error)
        );
        assert_eq!(
            detect_level(r#"{"severity":"warning"}"#),
            Some(LogLevel::Warn)
        );
        assert_eq!(detect_level(r#"{"lvl":"debug"}"#), Some(LogLevel::Debug));
        assert_eq!(
            detect_level(r#"{"log.level":"info"}"#),
            Some(LogLevel::Info)
        );
    }

    #[test]
    fn json_numeric_level_maps_pino_scale() {
        let level = |value: &str| detect_level(&format!(r#"{{"level":{value}}}"#));
        assert_eq!(level("10"), Some(LogLevel::Debug));
        assert_eq!(level("20"), Some(LogLevel::Debug));
        assert_eq!(level("30"), Some(LogLevel::Info));
        assert_eq!(level("40"), Some(LogLevel::Warn));
        assert_eq!(level("50"), Some(LogLevel::Error));
        assert_eq!(level("60"), Some(LogLevel::Error));
        assert_eq!(level("35"), None);
        assert_eq!(level("true"), None);
    }

    #[test]
    fn logfmt_level_is_detected() {
        assert_eq!(detect_level("ts=1 level=warn msg=x"), Some(LogLevel::Warn));
        assert_eq!(detect_level(r#"lvl="info" a=b"#), Some(LogLevel::Info));
    }

    #[test]
    fn klog_prefix_is_detected() {
        assert_eq!(
            detect_level("E0501 10:47:58.902 1 x.go:12] boom"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            detect_level("I0501 10:47:58.902 1 x.go:12] ok"),
            Some(LogLevel::Info)
        );
        assert_eq!(detect_level("W0501 10:47:58.902 x"), Some(LogLevel::Warn));
        assert_eq!(detect_level("F0501 10:47:58.902 x"), Some(LogLevel::Error));
        assert_eq!(detect_level("X0501 10:47:58.902 x"), None);
    }

    #[test]
    fn uppercase_keyword_in_head_is_detected() {
        assert_eq!(
            detect_level("2024 ERROR [main] boom"),
            Some(LogLevel::Error)
        );
    }

    #[test]
    fn lowercase_word_needs_brackets_or_tabs() {
        assert_eq!(detect_level("[error] x"), Some(LogLevel::Error));
        assert_eq!(detect_level("\tinfo\tx"), Some(LogLevel::Info));
        assert_eq!(detect_level("no error here"), None);
    }

    #[test]
    fn underscore_joined_word_is_not_a_level() {
        assert_eq!(detect_level("ERROR_COUNT=3"), None);
        assert_eq!(detect_level("no_error"), None);
    }

    #[test]
    fn keyword_after_64_bytes_is_ignored() {
        let padding = "x ".repeat(32);
        assert_eq!(detect_level(&format!("{padding}ERROR")), None);
        assert_eq!(
            detect_level(&format!("{}ERROR", "x ".repeat(10))),
            Some(LogLevel::Error)
        );
    }

    #[test]
    fn keyword_window_cuts_at_char_boundary() {
        let text = format!("{}é rest", "a".repeat(63));
        assert_eq!(detect_level(&text), None);
    }

    #[test]
    fn level_set_toggles_and_reports_hidden() {
        let set = LevelSet::default();
        assert!(set.hides_none());
        let set = set.toggled(LogLevel::Info);
        assert!(set.is_hidden(LogLevel::Info));
        assert!(!set.is_hidden(LogLevel::Error));
        assert!(!set.hides_none());
        assert!(set.toggled(LogLevel::Info).hides_none());
    }

    #[test]
    fn alt_click_shows_only_that_level_and_a_second_one_restores_all() {
        let only_error = LevelSet::default().toggled_only(LogLevel::Error);
        assert!(!only_error.is_hidden(LogLevel::Error));
        assert!(
            LogLevel::ALL[1..]
                .iter()
                .all(|level| only_error.is_hidden(*level))
        );
        assert!(only_error.toggled_only(LogLevel::Error).hides_none());
    }

    #[test]
    fn alt_click_on_another_level_switches_the_only_level() {
        let set = LevelSet::default()
            .toggled_only(LogLevel::Error)
            .toggled_only(LogLevel::Warn);
        assert!(!set.is_hidden(LogLevel::Warn));
        assert!(set.is_hidden(LogLevel::Error));
    }

    #[test]
    fn alt_click_after_a_plain_toggle_still_selects_only() {
        let set = LevelSet::default()
            .toggled(LogLevel::Debug)
            .toggled_only(LogLevel::Debug);
        assert!(!set.is_hidden(LogLevel::Debug));
        assert!(set.is_hidden(LogLevel::Info));
    }
}
