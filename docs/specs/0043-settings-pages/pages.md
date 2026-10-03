# 0043 · General, Appearance density, Logs, Terminal & Shell

[Back to index](README.md) · Steps 1–2 · Module: `settings_window.rs` (+ readers in [settings-model.md](settings-model.md)). Wireframes: W2 nav, Tokens "Chữ và mật độ" (28 / 36 px), W8 and W8b log toolbar (Timestamps, Wrap, JSON), 0036 shell tab picker.

## Page order (`PAGES`)

W2 nav order, pages with content only (0025 rule). Metrics and Extensions stay out (backlog).

| W2 nav | After 0043 | Step |
|---|---|---|
| General | **new**, first | 1 |
| Clusters | unchanged, still the page `Ctrl ,` opens | — |
| Appearance | + Row density | 1 |
| Keyboard Shortcuts, Safety | unchanged | — |
| Terminal & Shell | **new** | 2 |
| Logs | **new** | 2 |
| About | unchanged | — |

`SettingsPage` gains `General`, `TerminalAndShell` (title `Terminal & Shell`), `Logs`; `pages_follow_w2_order` lists all eight. Every page is `.resettable(false)` (0025 decision 6): each dropdown lists its default value, so going back is one pick; the default option's label ends with ` (default)`.

All fields bind `AppSettings::get` / `AppSettings::update` (0025 rule). Dropdowns use the 0025 one-table pattern: `const X_OPTIONS: [(value, &str); n]`, the label is the key, `x_label` / `x_from_label` next to the type (unknown label → default). A stored value that is not an option shows its own label (`"{n} lines"`), so a hand edit is visible and not silently replaced.

## General (step 1)

| Group · item | Control | Text |
|---|---|---|
| Files · Export folder | `SettingItem::render`: mono path (or `Home folder` muted) + ghost `Choose…` (`prompt_for_paths`, directories only, one) + ghost `Use home folder` (shown when set; stores `None`) | description `Where Export dialogs start. Updated after each export.` |
| Issues · Watch TLS Secrets for expiry | switch | `Lists and watches Secrets of type kubernetes.io/tls; this shows in API audit logs. Applies when the cluster is opened again.` (0020 decision 11) |

- Export folder write after a save: `start_export_with` sends `path.parent()` with `AppSettings::update` in its `finish` step (only after `Saved`, never on cancel or failure). The picker writes the same key.
- TLS off: `condition_plan(scope, access, CertificateWatch::Skip)` returns `FeedPlan::Off("off in Settings")` for `ResourceKind::Secrets`; coverage names it (`Not checked: certificates (off in Settings).`). `CertificateWatch { Watch, Skip }` is an enum, not a `bool` (design.md). The Ingress drawer's on-demand TLS companion watch (0016) is not affected.

## Appearance · Row density (step 1)

| Item | Control | Effect |
|---|---|---|
| Row density | dropdown `Compact (28 px)`, `Comfortable (36 px)` | every `DataTable` gets `.with_size(Size::Size(px(density.row_height())))`; live |

```rust
impl RowDensity { pub(crate) fn row_height(self) -> f32 } // Compact 28., Comfortable 36.
```

- **Live**: `workspace.rs` has no settings observer today; it gains `_settings_observer: cx.observe_global::<AppSettings>(|_, cx| cx.notify())` so the tables re-render on a change.
- `with_size` sets the kit's `table_row_height` for the **header row too** (kit `state.rs` uses the same size for the header), so the header is 28 / 36 px as well; accepted (Tokens show one row height).
- **Default Compact 28 px** (coordinator decision 2026-10-03, Tokens "default for power users"; today the kit's 32 px). Cell padding stays the kit default for `Size::Size` (4 px top and bottom), leaving 20 px of content. The default is **conditional on the ui-verifier 28 px check** of status pills, usage bars, and the selection checkbox: anything that clips is fixed in its cell renderer, never by changing the default (0024 "row-height audit").
- Lists that are not `DataTable` (dock, drawer sections, switcher) are out of scope.

## Logs (step 2)

Group "New log tabs", description `Each tab can still change these from its toolbar.`

| Item | Control | Options |
|---|---|---|
| Lines loaded at open | dropdown | `100`, `500`, `1,000`, `5,000`, `10,000` lines |
| Timestamps | switch | — |
| Wrap long lines | switch | — |
| Show JSON as message and fields | switch | — |

- `LogTab::new` reads the three switches into `shows_timestamps`, `wraps_lines`, `shows_json`. A tab toggle never writes back (one source of truth per tab).
- `tail_lines()` replaces `POD_TAIL_LINES` at both sites; `LATE_JOIN_TAIL_LINES` (50) stays. Workload tabs open up to `MAX_WORKLOAD_STREAMS` (20) streams with this tail; the 8 MiB staging cap and the 10,000-line buffer still bound memory. The log request is the existing `pods/{name}/log` GET with a different `tailLines`: no new call.

## Terminal & Shell (step 2)

| Group · item | Control | Options |
|---|---|---|
| Shell · Default shell | dropdown | `Auto (bash, ash, sh)`, `bash`, `sh` (the 0036 picker set) |
| Terminal · Scrollback | dropdown | `1,000`, `5,000`, `10,000` lines (cap and memory: [settings-model.md](settings-model.md)) |
| Terminal · Font size | dropdown | `Theme default`, `12`, `13`, `14`, `16`, `18` px |

- Default shell: the three `ShellCommand::Auto` literals in `shell_open.rs::start_shell` and `ShellTab::new`'s initial `command` read `terminal.default_shell`. The intent, confirm dialog, and audit line already name the command (0036), so a fixed `bash` shows there. Node shell and debug containers (0037) keep their own command.
- Scrollback: `TerminalSession::new(size: GridSize, scrollback_lines: u32)`; `SCROLLBACK_LINES` becomes the `TerminalSettings` default. Existing tabs keep theirs.
- Font size: `measure` uses `AppSettings::get(cx).terminal.font_size()` (or `theme.mono_font_size`); cell metrics are recomputed each frame already, so the resize path sends the new grid.
- Description of the page: `Applies to new shell tabs; font size applies at once.`

## Not on these pages (lean on purpose)

Secret clipboard clear time (fixed 30 s, 0016 decision 26), log buffer size, late-join tail, Previous by default, cursor style, copy on select, bell, paste confirmation (a safety check, never a toggle), font family, "reopen last cluster", language, updates, telemetry. Each is speculative or a safety rule; add it when a user asks.
