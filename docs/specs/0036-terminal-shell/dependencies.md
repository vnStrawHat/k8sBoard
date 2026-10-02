# 0036 · Dependencies: `oneterm-vt` and kube `ws`

[Back to index](README.md) · Steps 1–2. Decisions 1–5. C3 (exec, project rule) was approved by the user on 2026-10-02 (one approval for all mutating specs); the dependency changes themselves (C6) still need the user's approval.

## `oneterm-vt` (terminal engine)

| Item | Value (probed 2026-10-02 from a shallow clone in `.tmp/oneterm-src`) |
|---|---|
| Repository | `https://github.com/vnStrawHat/OneTerm`, crate `crates/vt`, package `oneterm-vt` 0.6.7 |
| Pin | `rev = "e2f9c24bc747b26dbfe0e338e66f1c41a3ac30d8"`: the repository HEAD and the last commit that touched `crates/vt` at probe time |
| Features | `default-features = false` drops `pty` (ConPTY/openpty, `polling`, `windows-sys`, `libc`); the engine alone has six leaf deps: `bitflags`, `log`, `memchr`, `rustc-hash`, `unicode-segmentation`, `unicode-width`, all already in our `Cargo.lock` |
| Toolchain | MSRV 1.96.0, edition 2024: equal to ours. `cargo check -p oneterm-vt` passed in a scratch copy of the workspace |
| Licence | Apache-2.0 (`LICENSE`, `NOTICE` in the repository): compatible; the app's About page and third-party notices list it |
| Publication | not on crates.io (owner ruling in its `Cargo.toml`); tags do not carry it yet, so a `rev` pin is the only reproducible form |

Root `Cargo.toml`: the commented line becomes
`oneterm-vt = { git = "https://github.com/vnStrawHat/OneTerm", rev = "e2f9c24bc747b26dbfe0e338e66f1c41a3ac30d8", default-features = false }`;
`crates/app/Cargo.toml` adds `oneterm-vt.workspace = true`. The git checkout lands in `.cargo-home/git` (agent rule). The first fetch clones the whole repository (tens of MB).

### API used (verified in the probe; names from `crates/vt/src/lib.rs`)

| Need | API |
|---|---|
| Engine | `Terminal::new(Size { rows, cols }, Config { scrollback_limit, product_name, .. })`, `feed(&[u8], &mut EventBatch, Instant) -> FeedStats` (never panics on input) |
| Events | `EventBatch::iter()` → `VtEvent::{Reply, Title, Bell, ClipboardStore, ClipboardLoad, ColorQuery, Osc, …}`; payloads via `batch.bytes(span)` / `batch.str(span)` |
| Drawing | `snapshot_update(&mut SnapshotState, Instant)`; `SnapshotState::{rows, changed, cursor, selection, modes, scroll_offset, map_colors(&Palette)}`; rows carry cells and `StyleRun`s |
| Colours | `Palette { indexed: [Rgb; 256], foreground, background, cursor, .. }`, `Palette::new()` for xterm defaults |
| Input | `encode_key(&KeySpec, KeyMods) -> Option<Vec<u8>>` (`KeySpec::{Character(String), Named(NamedKey)}`, `KeyMods { shift, ctrl, alt }`), `mode_snapshot().bracketed_paste`, `input::encode_wheel_event` |
| Selection | `hit_test(row, col) -> (Pos, Side)`, `selection_start/update/clear`, `selection_text() -> Option<String>`, `select_all` |
| Geometry | `resize(Size, ResizePolicy::BottomAnchor)`, `set_cell_pixels`, `screen().scroll_viewport(i32)`, `scroll_to_bottom()` (through `grid_mut()`; coder confirms the accessor path) |
| Search | `search::{GridText::from_terminal, search_grid_text, SearchPattern::Literal, SearchOptions}`, `SearchMatch { row, start_col, end_col }` |

To confirm in step 2: the exact accessor path to `scroll_viewport` (`Terminal::grid_mut()` then the screen). `ColorQuery { key, terminator }` is answered by the embedder from `terminal_palette` with the query's terminator ([terminal-view.md](terminal-view.md)). `Config::allow_screen_readback` exists and defaults to `false`; 0036 pins it. The guide (`crates/vt/docs/guide/`, chapters 2, 3, 4, 6, 7, 9) is the reference; `cargo doc -p oneterm-vt` renders it offline.

OneTerm's own GPUI renderer (`crates/terminal-view`, same `gpui-pre` 0.3 / `gpui-component` 0.7 as us) is a **reading reference only**: it is an application crate tied to its workspace, so we do not depend on it. Any code copied from it keeps its Apache-2.0 header and goes into our third-party notices.

## kube `ws` feature (exec transport)

- kube 4.2 has it: `kube-client` feature `ws = ["client", "tokio-tungstenite", "kube-core/ws", "tokio/macros"]`, giving `Api<Pod>::exec(name, command, &AttachParams) -> AttachedProcess` with `stdin()`, `stdout()`, `terminal_size() -> Option<mpsc::Sender<TerminalSize>>` (futures channel), `take_status()`, `abort()`.
- Enabled **only in the cluster crate**: `crates/cluster/Cargo.toml` `kube = { workspace = true, features = ["ws"] }` (the root stays as is; the app has no kube dependency).
- `Cargo.lock` gains exactly five packages (probe: resolved both changes in a scratch copy): `tokio-tungstenite` 0.29.0, `tungstenite` 0.29.0, `sha1` 0.10.7, `data-encoding` 2.11.1, `oneterm-vt` 0.6.7 (git). No version changes to existing packages. AC 2 checks this list.
