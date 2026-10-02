# 0016 · App: private clipboard write and auto-clear

[Back to index](README.md) · Step 3 · Modules: `secret_clipboard.rs` (new) + `secret_clipboard_tests.rs`, `secret_values.rs`, `app_shell.rs`. Settles C1 "auto-clear the clipboard?" (decision 26, coordinator decision M2).

## Cargo (step 3)

```toml
# crates/app/Cargo.toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.62", features = ["Win32_Foundation", "Win32_System_DataExchange", "Win32_System_Memory", "Win32_System_Ole"] }
```

`windows` 0.62.2 is already locked (gpui, wgpu-hal): no new package (AC 4 for step 3: `Cargo.lock` unchanged).

## API (`secret_clipboard.rs`)

```rust
pub(crate) const CLIPBOARD_CLEAR_DELAY: Duration = Duration::from_secs(30);
/// What we wrote, without keeping it: a hash keyed with a per-copy random key.
pub(crate) struct ClipboardMark { hasher: std::hash::RandomState, hash: u64 }
impl ClipboardMark { pub(crate) fn of(text: &str) -> Self; pub(crate) fn matches(&self, text: &str) -> bool; }
pub(crate) enum ClipboardWriteError { Unavailable } // fixed text "The clipboard is busy; nothing was copied."
/// Writes `text` so that clipboard history and cloud sync skip it where the OS allows.
pub(crate) fn write_private_text(text: &str, cx: &mut App) -> Result<ClipboardMark, ClipboardWriteError>;
/// Clears the clipboard when it still holds the marked text; leaves anything else alone.
pub(crate) fn clear_if_unchanged(mark: &ClipboardMark, cx: &mut App);
```

- Decisions stay cross-platform (`ClipboardMark`, `clear_if_unchanged`'s compare); `#[cfg(windows)]` sits only on the FFI functions (structure rules). Non-Windows: `cx.write_to_clipboard(ClipboardItem::new_string(..))` and, for clearing, an empty string.
- `clear_if_unchanged` reads `cx.read_from_clipboard()` text into `Zeroizing<String>`, compares with `mark.matches`, clears on match.

## Windows FFI (`#[cfg(windows)] mod windows_clipboard` inside `secret_clipboard.rs`)

```rust
#[cfg(windows)] fn write_excluded_text(text: &str) -> Result<(), ClipboardWriteError>;
#[cfg(windows)] fn empty_clipboard();
```

One clipboard session per write, so monitors never see the text without the exclusion formats:

1. `OpenClipboard(None)` (retry 5 × 20 ms while another process holds it; then `Unavailable`), `EmptyClipboard`.
2. `CF_UNICODETEXT`: UTF-16 with a trailing NUL in a `GlobalAlloc(GMEM_MOVEABLE)` block → `SetClipboardData`.
3. `RegisterClipboardFormatW` for `ExcludeClipboardContentFromMonitorProcessing` (any 1-byte data), `CanIncludeInClipboardHistory` (DWORD 0), `CanUploadToCloudClipboard` (DWORD 0) → `SetClipboardData` each.
4. `CloseClipboard` on every path (a guard type whose `Drop` closes it). A failed `SetClipboardData` frees its block; on success the system owns it.

- `empty_clipboard`: open, `EmptyClipboard`, close (no empty text entry in history).
- **Fail closed**: if the private write fails, nothing is copied and the row shows the error; there is no fallback to `cx.write_to_clipboard` (it would land in history).
- `unsafe`: the workspace denies `unsafe_code`; this module is the **only** allowed exception: `#[allow(unsafe_code)]` on `mod windows_clipboard` with a `// SAFETY:` comment on each block (handle ownership, NUL-terminated buffer, lock/unlock pairing). AC 1 names it.

## Auto-clear (`app_shell.rs`)

```rust
clipboard_clear: Option<(ClipboardMark, Task<()>)>,   // AppShell field
fn arm_clipboard_clear(&mut self, mark: ClipboardMark, cx: &mut Context<Self>);
```

- `SecretValuesView` emits `SecretCopied(ClipboardMark)` (`EventEmitter`); `sync_secret_values` subscribes when it creates the view. The shell owns the timer, so it survives the drawer closing and a context switch.
- `arm_clipboard_clear` replaces any armed clear (a new copy restarts the 30 s; the old mark no longer matters because the clipboard now holds the new text). After `CLIPBOARD_CLEAR_DELAY`: `clear_if_unchanged(&mark, cx)`, then `clipboard_clear = None`.
- App quit: `cx.on_app_quit` runs `clear_if_unchanged` for an armed mark (best effort).
- Copying something else inside the app or another program within 30 s leaves the clipboard untouched (hash mismatch).

## Ceilings

- **macOS / Linux**: clipboard managers and history tools (Maccy, Klipper, GPaste, …) are not controllable from GPUI; they may keep the value. The 30 s clear still runs. The macOS `org.nspasteboard.ConcealedType` convention would need AppKit FFI: not done.
- Windows: a program already monitoring the clipboard may ignore the exclusion formats; the clear removes the value, it cannot wipe another process's copy.
- The hash is in-process, keyed per copy, never logged, and dropped after the clear. Someone who can read process memory could brute-force a short value from hash and key, but could equally read the clipboard itself.
