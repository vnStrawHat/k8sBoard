//! The clipboard side of Copy for Secret values: a write that clipboard history and cloud sync skip
//! where the OS allows it, and a clear that runs 30 s later when the clipboard still holds what
//! was written.
//!
//! The decisions (what to write, when to clear, when to try again) are plain code behind
//! `ClipboardPort`, so every test runs on every OS with a fake port and never touches the system
//! clipboard. Only the Windows calls are `#[cfg(windows)]`, in `windows_clipboard`: the one module
//! of the workspace that may use `unsafe`. Nothing here logs a value or keeps one: the clear
//! compares a keyed hash.
//!
//! Opening the Windows clipboard waits at most 4 x 20 ms (80 ms) for another process to let go. A
//! clear opens it twice (read, then empty), so one attempt stalls the main thread for about 160 ms
//! at worst, and 5 attempts for about 800 ms. That is bounded by design: a copy is a click, and the
//! clear runs once. An unreadable clipboard at clear time is retried (`next_clear_step`), never
//! given up. The read and the empty are separate sessions, so something copied between them is
//! cleared too: a window of milliseconds, accepted.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::BuildHasher as _;
use std::time::Duration;

use gpui_kit::App;
use zeroize::Zeroizing;

/// How long a copied value may stay on the clipboard.
pub(crate) const CLIPBOARD_CLEAR_DELAY: Duration = Duration::from_secs(30);
/// How long to wait before trying a clear again when the clipboard could not be read or emptied.
pub(crate) const CLEAR_RETRY_DELAY: Duration = Duration::from_secs(1);
/// How many clear attempts a copy gets in all.
pub(crate) const CLEAR_MAX_TRIES: u32 = 5;

/// What was written, without keeping it: a hash keyed with a random per-copy key. It tells the
/// clear whether the clipboard still holds the copied text; the key never leaves this value.
#[derive(Clone)]
pub(crate) struct ClipboardMark {
    hasher: RandomState,
    hash: u64,
}

impl ClipboardMark {
    pub(crate) fn of(text: &str) -> Self {
        let hasher = RandomState::new();
        let hash = hasher.hash_one(text);
        Self { hasher, hash }
    }

    pub(crate) fn matches(&self, text: &str) -> bool {
        self.hasher.hash_one(text) == self.hash
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipboardWriteError {
    /// Another process holds the clipboard, or an exclusion format could not be set.
    Unavailable,
}

impl fmt::Display for ClipboardWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("The clipboard is busy; nothing was copied.")
    }
}

/// What a clear did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClearOutcome {
    Cleared,
    /// The clipboard holds something else, or no text: it is left alone.
    Kept,
    /// The clipboard could not be read or emptied just now, so the value may still be there.
    Retry,
}

/// What the caller does after a clear attempt number `attempt` (counted from 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClearStep {
    Done,
    RetryIn(Duration),
}

/// `Retry` tries again after `CLEAR_RETRY_DELAY`, until `CLEAR_MAX_TRIES` attempts were made.
pub(crate) fn next_clear_step(outcome: ClearOutcome, attempt: u32) -> ClearStep {
    match outcome {
        ClearOutcome::Retry if attempt + 1 < CLEAR_MAX_TRIES => {
            ClearStep::RetryIn(CLEAR_RETRY_DELAY)
        }
        ClearOutcome::Cleared | ClearOutcome::Kept | ClearOutcome::Retry => ClearStep::Done,
    }
}

/// The three operations the logic needs from a clipboard.
trait ClipboardPort {
    /// The text on the clipboard: `Ok(None)` when it holds no text, `Err` when it could not be
    /// read at all.
    fn read_text(&mut self) -> Result<Option<Zeroizing<String>>, ClipboardWriteError>;
    /// Writes `text` so that history and cloud sync skip it where the OS allows. Fails closed:
    /// on an error nothing of `text` stays on the clipboard.
    fn write_private(&mut self, text: &str) -> Result<(), ClipboardWriteError>;
    fn clear(&mut self) -> Result<(), ClipboardWriteError>;
}

fn write_marked(
    port: &mut impl ClipboardPort,
    text: &str,
) -> Result<ClipboardMark, ClipboardWriteError> {
    port.write_private(text)?;
    Ok(ClipboardMark::of(text))
}

fn clear_marked(port: &mut impl ClipboardPort, mark: &ClipboardMark) -> ClearOutcome {
    let current = match port.read_text() {
        Ok(current) => current,
        Err(_) => return ClearOutcome::Retry,
    };
    if !current.is_some_and(|text| mark.matches(&text)) {
        return ClearOutcome::Kept;
    }
    match port.clear() {
        Ok(()) => ClearOutcome::Cleared,
        Err(_) => ClearOutcome::Retry,
    }
}

/// Writes `text` privately and returns the mark the clear needs. There is no fallback to a plain
/// write: it could land in clipboard history.
pub(crate) fn write_private_text(
    text: &str,
    cx: &mut App,
) -> Result<ClipboardMark, ClipboardWriteError> {
    write_marked(&mut system_clipboard(cx), text)
}

/// Clears the clipboard when it still holds the marked text; anything else is left alone.
pub(crate) fn clear_if_unchanged(mark: &ClipboardMark, cx: &mut App) -> ClearOutcome {
    clear_marked(&mut system_clipboard(cx), mark)
}

/// Windows: the clipboard through the Win32 calls below, which GPUI cannot make private.
#[cfg(windows)]
fn system_clipboard(_cx: &mut App) -> impl ClipboardPort {
    WindowsClipboard
}

#[cfg(windows)]
struct WindowsClipboard;

#[cfg(windows)]
impl ClipboardPort for WindowsClipboard {
    fn read_text(&mut self) -> Result<Option<Zeroizing<String>>, ClipboardWriteError> {
        windows_clipboard::read_text()
    }

    fn write_private(&mut self, text: &str) -> Result<(), ClipboardWriteError> {
        windows_clipboard::write_excluded_text(text)
    }

    fn clear(&mut self) -> Result<(), ClipboardWriteError> {
        windows_clipboard::empty_clipboard()
    }
}

/// Elsewhere: the clipboard through GPUI. There is no history or cloud sync to exclude from, and
/// clipboard managers stay a ceiling.
#[cfg(not(windows))]
fn system_clipboard(cx: &mut App) -> impl ClipboardPort + '_ {
    GpuiClipboard(cx)
}

#[cfg(not(windows))]
struct GpuiClipboard<'a>(&'a mut App);

#[cfg(not(windows))]
impl ClipboardPort for GpuiClipboard<'_> {
    fn read_text(&mut self) -> Result<Option<Zeroizing<String>>, ClipboardWriteError> {
        Ok(self
            .0
            .read_from_clipboard()
            .and_then(|item| item.text())
            .map(Zeroizing::new))
    }

    fn write_private(&mut self, text: &str) -> Result<(), ClipboardWriteError> {
        self.0
            .write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.to_owned()));
        Ok(())
    }

    fn clear(&mut self) -> Result<(), ClipboardWriteError> {
        self.0
            .write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new()));
        Ok(())
    }
}

/// One extra format of the private write: a registered clipboard format and its payload.
#[cfg(any(windows, test))]
struct ExclusionFormat {
    name: &'static str,
    data: &'static [u8],
}

/// The documented formats that keep clipboard monitors, Win+V history, and the cloud clipboard
/// away from the text. The first needs any payload; the other two need a DWORD 0.
#[cfg(any(windows, test))]
const EXCLUSION_FORMATS: [ExclusionFormat; 3] = [
    ExclusionFormat {
        name: "ExcludeClipboardContentFromMonitorProcessing",
        data: &[1],
    },
    ExclusionFormat {
        name: "CanIncludeInClipboardHistory",
        data: &[0, 0, 0, 0],
    },
    ExclusionFormat {
        name: "CanUploadToCloudClipboard",
        data: &[0, 0, 0, 0],
    },
];

/// `text` as UTF-16 with the trailing NUL that `CF_UNICODETEXT` needs, wiped on drop.
#[cfg(any(windows, test))]
/// The buffer is sized up front (UTF-16 never has more units than the UTF-8 has bytes), so it never
/// reallocates and leaves no unwiped fragment behind.
fn utf16_with_nul(text: &str) -> Zeroizing<Vec<u16>> {
    let mut wide = Zeroizing::new(Vec::with_capacity(text.len() + 1));
    wide.extend(text.encode_utf16());
    wide.push(0);
    wide
}

/// The text of a `CF_UNICODETEXT` block: up to the first NUL, or all of `words` without one. A lone
/// surrogate becomes U+FFFD. The string is sized up front (a UTF-16 unit is at most 3 UTF-8 bytes),
/// so it never reallocates and leaves no unwiped fragment behind.
#[cfg(any(windows, test))]
fn text_from_utf16_block(words: &[u16]) -> Zeroizing<String> {
    let end = words
        .iter()
        .position(|&word| word == 0)
        .unwrap_or(words.len());
    let mut text = Zeroizing::new(String::with_capacity(end * 3));
    text.extend(
        char::decode_utf16(words[..end].iter().copied())
            .map(|decoded| decoded.unwrap_or(char::REPLACEMENT_CHARACTER)),
    );
    text
}

#[cfg(windows)]
// SAFETY (module): every `unsafe` block below calls a Win32 clipboard or global-memory function
// with arguments this module built itself; each block states its own invariants.
#[allow(unsafe_code)]
mod windows_clipboard {
    use std::thread::sleep;
    use std::time::Duration;

    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    use windows::core::PCWSTR;
    use zeroize::Zeroizing;

    use super::{ClipboardWriteError, EXCLUSION_FORMATS, text_from_utf16_block, utf16_with_nul};

    /// Another process may hold the clipboard for a moment.
    const OPEN_ATTEMPTS: u32 = 5;
    const OPEN_RETRY_DELAY: Duration = Duration::from_millis(20);

    /// An open clipboard session. `Drop` closes it, so every path (including an early return)
    /// ends the session.
    struct OpenClipboardGuard;

    impl OpenClipboardGuard {
        fn open() -> Option<Self> {
            for attempt in 0..OPEN_ATTEMPTS {
                // SAFETY: a null owner window is valid (`None`): the session then belongs to the
                // current task. No pointer is passed. A successful open is paired with the
                // `CloseClipboard` in `Drop`; a failed one is retried and never closed.
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Some(Self);
                }
                if attempt + 1 < OPEN_ATTEMPTS {
                    sleep(OPEN_RETRY_DELAY);
                }
            }
            None
        }
    }

    impl Drop for OpenClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: a guard exists only after a successful `OpenClipboard` on this thread and
            // is dropped once, so the clipboard is open and owned by this thread here.
            let _ = unsafe { CloseClipboard() };
        }
    }

    /// The three exclusion formats, then the text, in one clipboard session, so no monitor ever
    /// sees the text without them. The text goes last on purpose: once its block belongs to the
    /// system nothing else can fail, so the fail-closed `EmptyClipboard` below never has to free
    /// an unwiped text block (the formats hold no secret). After any error the clipboard is
    /// emptied again.
    pub(super) fn write_excluded_text(text: &str) -> Result<(), ClipboardWriteError> {
        let Some(_session) = OpenClipboardGuard::open() else {
            return Err(ClipboardWriteError::Unavailable);
        };
        // SAFETY: the session is open on this thread (guard alive); `EmptyClipboard` takes no
        // arguments and takes ownership of the clipboard for this session.
        if unsafe { EmptyClipboard() }.is_err() {
            return Err(ClipboardWriteError::Unavailable);
        }
        let result = set_exclusion_formats().and_then(|()| set_text(text));
        if result.is_err() {
            // SAFETY: as above, the session is still open.
            let _ = unsafe { EmptyClipboard() };
        }
        result
    }

    /// Empties the clipboard in its own session, leaving no empty text entry in history.
    pub(super) fn empty_clipboard() -> Result<(), ClipboardWriteError> {
        let Some(_session) = OpenClipboardGuard::open() else {
            return Err(ClipboardWriteError::Unavailable);
        };
        // SAFETY: the session is open on this thread (guard alive).
        unsafe { EmptyClipboard() }.map_err(|_| ClipboardWriteError::Unavailable)
    }

    /// The text on the clipboard, read in its own session: `Ok(None)` when there is no text.
    pub(super) fn read_text() -> Result<Option<Zeroizing<String>>, ClipboardWriteError> {
        let Some(_session) = OpenClipboardGuard::open() else {
            return Err(ClipboardWriteError::Unavailable);
        };
        let format = u32::from(CF_UNICODETEXT.0);
        // SAFETY: a plain query of a format id inside an open session.
        if unsafe { IsClipboardFormatAvailable(format) }.is_err() {
            return Ok(None);
        }
        // SAFETY: the session is open. The handle stays owned by the clipboard: it is only
        // locked, read, and unlocked below, never freed or stored.
        let handle =
            unsafe { GetClipboardData(format) }.map_err(|_| ClipboardWriteError::Unavailable)?;
        let block = HGLOBAL(handle.0);
        // SAFETY: `block` is the live global block the clipboard holds for `CF_UNICODETEXT`; a
        // null result means the lock failed and nothing is read.
        let source = unsafe { GlobalLock(block) }.cast::<u16>();
        if source.is_null() {
            return Err(ClipboardWriteError::Unavailable);
        }
        // SAFETY: while locked, `source` is valid for `GlobalSize` bytes, which is `size / 2`
        // whole `u16` words; global blocks are aligned for `u16`. The slice dies before the
        // unlock, and the text is copied out of it.
        let text = unsafe {
            let words = GlobalSize(block) / 2;
            let slice = std::slice::from_raw_parts(source, words);
            let text = text_from_utf16_block(slice);
            // `Err` is the expected result when the lock count drops to 0; the clipboard owns the
            // block either way.
            let _ = GlobalUnlock(block);
            text
        };
        Ok(Some(text))
    }

    fn set_text(text: &str) -> Result<(), ClipboardWriteError> {
        let wide = utf16_with_nul(text);
        set_global_bytes(u32::from(CF_UNICODETEXT.0), utf16_bytes(&wide))
    }

    fn set_exclusion_formats() -> Result<(), ClipboardWriteError> {
        for format in &EXCLUSION_FORMATS {
            let name = utf16_with_nul(format.name);
            // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call.
            let id = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
            if id == 0 {
                return Err(ClipboardWriteError::Unavailable);
            }
            set_global_bytes(id, format.data)?;
        }
        Ok(())
    }

    /// Copies `bytes` into a movable global block and hands it to the open clipboard. On success
    /// the system owns the block; on failure it is wiped and freed here.
    fn set_global_bytes(format: u32, bytes: &[u8]) -> Result<(), ClipboardWriteError> {
        // SAFETY: `GlobalAlloc` with `GMEM_MOVEABLE` and a non-zero size has no pointer inputs.
        let block = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }
            .map_err(|_| ClipboardWriteError::Unavailable)?;
        if write_block(block, bytes).is_err() {
            free_block(block);
            return Err(ClipboardWriteError::Unavailable);
        }
        // SAFETY: the clipboard is open (the caller holds the session), `format` is a clipboard
        // format id, and `block` is a live movable global block that is not locked. On success the
        // system takes ownership of it, so it is not freed again; on failure we still own it.
        match unsafe { SetClipboardData(format, Some(HANDLE(block.0))) } {
            Ok(_) => Ok(()),
            Err(_) => {
                wipe_block(block, bytes.len());
                free_block(block);
                Err(ClipboardWriteError::Unavailable)
            }
        }
    }

    fn write_block(block: HGLOBAL, bytes: &[u8]) -> Result<(), ()> {
        // SAFETY: `block` is a live movable block from `GlobalAlloc` of at least `bytes.len()`
        // bytes (min 1). A null return means the lock failed and nothing is written.
        let target = unsafe { GlobalLock(block) }.cast::<u8>();
        if target.is_null() {
            return Err(());
        }
        // SAFETY: `target` is valid for `bytes.len()` writes (the block size) and cannot overlap
        // `bytes`, which lives in our heap. The lock taken above is released right after.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
            // `Err` is the expected result when the lock count drops to 0, so it is ignored; the
            // caller owns the block either way.
            let _ = GlobalUnlock(block);
        }
        Ok(())
    }

    /// Zeroes the first `len` bytes of a block this module still owns.
    fn wipe_block(block: HGLOBAL, len: usize) {
        // SAFETY: `block` is a live movable block of at least `len` bytes (min 1) that the system
        // did not take; a null lock result writes nothing.
        let target = unsafe { GlobalLock(block) }.cast::<u8>();
        if target.is_null() {
            return;
        }
        // SAFETY: `target` is valid for `len` writes (the block size), locked until the unlock.
        unsafe {
            std::ptr::write_bytes(target, 0, len);
            // `Err` is the expected result when the lock count drops to 0.
            let _ = GlobalUnlock(block);
        }
    }

    fn free_block(block: HGLOBAL) {
        // SAFETY: `block` came from `GlobalAlloc`, is not locked, and was not accepted by the
        // clipboard (callers reach here only on failure paths), so this module still owns it and
        // frees it exactly once.
        let _ = unsafe { GlobalFree(Some(block)) };
    }

    /// The bytes of a UTF-16 buffer, for a global block.
    fn utf16_bytes(wide: &Zeroizing<Vec<u16>>) -> &[u8] {
        // SAFETY: a `u16` slice is `2 * len` initialized bytes with no padding, and `u8` has
        // alignment 1, so the byte view of the same memory and lifetime is valid.
        unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) }
    }
}

#[cfg(test)]
#[path = "secret_clipboard_tests.rs"]
mod secret_clipboard_tests;
