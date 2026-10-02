use super::*;

/// A clipboard in memory: nothing here reaches the system clipboard.
#[derive(Default)]
struct FakeClipboard {
    text: Option<String>,
    fail_writes: bool,
    fail_reads: bool,
    fail_clears: bool,
    private_writes: usize,
    clears: usize,
}

impl ClipboardPort for FakeClipboard {
    fn read_text(&mut self) -> Result<Option<Zeroizing<String>>, ClipboardWriteError> {
        if self.fail_reads {
            return Err(ClipboardWriteError::Unavailable);
        }
        Ok(self.text.clone().map(Zeroizing::new))
    }

    fn write_private(&mut self, text: &str) -> Result<(), ClipboardWriteError> {
        if self.fail_writes {
            return Err(ClipboardWriteError::Unavailable);
        }
        self.private_writes += 1;
        self.text = Some(text.to_owned());
        Ok(())
    }

    fn clear(&mut self) -> Result<(), ClipboardWriteError> {
        if self.fail_clears {
            return Err(ClipboardWriteError::Unavailable);
        }
        self.clears += 1;
        self.text = None;
        Ok(())
    }
}

const FIXTURE_VALUE: &str = "fixture-value-0016";

fn written(clipboard: &mut FakeClipboard) -> ClipboardMark {
    write_marked(clipboard, FIXTURE_VALUE).expect("the fake write succeeds")
}

#[test]
fn mark_matches_same_text_only() {
    let mark = ClipboardMark::of(FIXTURE_VALUE);
    assert!(mark.matches(FIXTURE_VALUE));
    assert!(!mark.matches("another fixture"));
    assert!(!mark.matches(""));
}

#[test]
fn marks_use_distinct_keys() {
    let first = ClipboardMark::of(FIXTURE_VALUE);
    let second = ClipboardMark::of(FIXTURE_VALUE);
    assert_ne!(first.hash, second.hash);
}

#[test]
fn write_returns_a_mark_of_the_written_text() {
    let mut clipboard = FakeClipboard::default();
    let mark = written(&mut clipboard);
    assert_eq!(clipboard.private_writes, 1);
    assert!(mark.matches(FIXTURE_VALUE));
}

#[test]
fn failed_private_write_copies_nothing_and_does_not_fall_back() {
    let mut clipboard = FakeClipboard {
        fail_writes: true,
        ..Default::default()
    };
    let result = write_marked(&mut clipboard, FIXTURE_VALUE);
    assert!(matches!(result, Err(ClipboardWriteError::Unavailable)));
    assert_eq!(clipboard.text, None);
}

#[test]
fn write_error_has_a_fixed_text() {
    assert_eq!(
        ClipboardWriteError::Unavailable.to_string(),
        "The clipboard is busy; nothing was copied."
    );
}

#[test]
fn clear_skipped_when_clipboard_changed() {
    let mut clipboard = FakeClipboard::default();
    let mark = written(&mut clipboard);
    // The user copied something else within 30 s.
    clipboard.text = Some("other text".to_owned());
    assert_eq!(clear_marked(&mut clipboard, &mark), ClearOutcome::Kept);
    assert_eq!(clipboard.clears, 0);
    assert_eq!(clipboard.text.as_deref(), Some("other text"));
}

#[test]
fn clear_runs_when_clipboard_unchanged() {
    let mut clipboard = FakeClipboard::default();
    let mark = written(&mut clipboard);
    assert_eq!(clear_marked(&mut clipboard, &mark), ClearOutcome::Cleared);
    assert_eq!(clipboard.clears, 1);
    assert_eq!(clipboard.text, None);
}

#[test]
fn clear_skipped_when_clipboard_holds_no_text() {
    let mut clipboard = FakeClipboard::default();
    let mark = ClipboardMark::of(FIXTURE_VALUE);
    assert_eq!(clear_marked(&mut clipboard, &mark), ClearOutcome::Kept);
    assert_eq!(clipboard.clears, 0);
}

#[test]
fn clear_retries_when_clipboard_unreadable() {
    let mut clipboard = FakeClipboard::default();
    let mark = written(&mut clipboard);
    clipboard.fail_reads = true;
    assert_eq!(clear_marked(&mut clipboard, &mark), ClearOutcome::Retry);
    // Nothing was touched: the value is still there for the retry.
    assert_eq!(clipboard.text.as_deref(), Some(FIXTURE_VALUE));
    clipboard.fail_reads = false;
    assert_eq!(clear_marked(&mut clipboard, &mark), ClearOutcome::Cleared);
}

#[test]
fn clear_retries_when_the_clipboard_cannot_be_emptied() {
    let mut clipboard = FakeClipboard::default();
    let mark = written(&mut clipboard);
    clipboard.fail_clears = true;
    assert_eq!(clear_marked(&mut clipboard, &mark), ClearOutcome::Retry);
    assert_eq!(clipboard.text.as_deref(), Some(FIXTURE_VALUE));
}

#[test]
fn retry_waits_one_second_and_gives_up_after_five_tries() {
    let again = ClearStep::RetryIn(Duration::from_secs(1));
    for attempt in 0..CLEAR_MAX_TRIES - 1 {
        assert_eq!(next_clear_step(ClearOutcome::Retry, attempt), again);
    }
    assert_eq!(
        next_clear_step(ClearOutcome::Retry, CLEAR_MAX_TRIES - 1),
        ClearStep::Done
    );
    assert_eq!(CLEAR_MAX_TRIES, 5);
}

#[test]
fn finished_clears_do_not_retry() {
    assert_eq!(next_clear_step(ClearOutcome::Cleared, 0), ClearStep::Done);
    assert_eq!(next_clear_step(ClearOutcome::Kept, 0), ClearStep::Done);
}

#[test]
fn clear_delay_is_thirty_seconds() {
    assert_eq!(CLIPBOARD_CLEAR_DELAY, Duration::from_secs(30));
}

#[test]
fn exclusion_formats_are_the_three_documented_ones() {
    let names: Vec<&str> = EXCLUSION_FORMATS.iter().map(|format| format.name).collect();
    assert_eq!(
        names,
        [
            "ExcludeClipboardContentFromMonitorProcessing",
            "CanIncludeInClipboardHistory",
            "CanUploadToCloudClipboard"
        ]
    );
}

#[test]
fn exclusion_format_payloads() {
    // The two DWORD formats say no (0); the first accepts any payload but needs one.
    assert_eq!(EXCLUSION_FORMATS[0].data.len(), 1);
    assert_eq!(EXCLUSION_FORMATS[1].data, [0, 0, 0, 0]);
    assert_eq!(EXCLUSION_FORMATS[2].data, [0, 0, 0, 0]);
}

#[test]
fn unicode_text_is_utf16_with_a_trailing_nul() {
    let wide = utf16_with_nul("a\u{e9}\u{1f512}");
    assert_eq!(*wide, [0x61, 0xe9, 0xd83d, 0xdd12, 0]);
}

#[test]
fn empty_text_is_just_the_nul() {
    assert_eq!(*utf16_with_nul(""), [0]);
}

#[test]
fn utf16_block_stops_at_the_first_nul() {
    let block = [0x61, 0x62, 0, 0x63, 0x64];
    assert_eq!(&**text_from_utf16_block(&block), "ab");
}

#[test]
fn utf16_block_without_nul_reads_to_the_end() {
    let block = [0x61, 0xe9, 0xd83d, 0xdd12];
    assert_eq!(&**text_from_utf16_block(&block), "a\u{e9}\u{1f512}");
    assert_eq!(&**text_from_utf16_block(&[]), "");
}

#[test]
fn utf16_block_lone_surrogate_becomes_the_replacement_character() {
    let block = [0x61, 0xd83d, 0x62, 0];
    assert_eq!(&**text_from_utf16_block(&block), "a\u{fffd}b");
}

#[test]
fn utf16_block_never_outgrows_its_capacity() {
    // The worst case is three bytes per unit: BMP characters of three bytes each.
    let text = "\u{20ac}".repeat(100);
    let block: Vec<u16> = text.encode_utf16().collect();
    let decoded = text_from_utf16_block(&block);
    assert_eq!(&**decoded, text);
    assert_eq!(decoded.capacity(), block.len() * 3);
}
