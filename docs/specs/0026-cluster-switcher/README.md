# 0026 — Cluster switcher (W1, single cluster)

Status: amended after the advisor review (M1–M5, S1–S8, N1–N5), HEAD `9d5af01`. Crate: `crates/app` only (uses the 0025 step 1 cluster-crate `connection_info`). Local plus read-only: the only new cluster call is `GET /version` for health probes. Prerequisites: [files-to-touch.md](files-to-touch.md). Applies C4, C5, C13. Wireframe: W1 (pins 1–5; pins 4 and 6 are 0027), keyboard map rows Ctrl Shift C and Ctrl 1–9.

## Goal

- The title-bar switcher becomes a **popover**: "Filter clusters…" input, All / Connected segment, env groups (Production, Staging, Development · Local) with badges, a current mark, a health line per row, `Ctrl 1…9` hints, Retry, and "Manage clusters… opens Settings Ctrl ,".
- **Switching** tears the old session down completely (tasks, watches, log streams) before the new one connects; the namespace scope is remembered per cluster for the app session.
- **Settings**: `last_used` is written when a session goes live (0024 amended); the start namespace follows memory, then the 0024 default namespace.
- **Errors**: a failed connect shows the workspace error view with Retry and "Back to {previous}".
- **Keys**: `Ctrl Shift C` toggles the switcher, `Ctrl 1…9` switch directly; arrows, Enter, Esc inside the switcher.
- Status bar: `API {version} · {n} ms`.

## Non-goals

- Several live clusters, checkboxes, Space, "View N clusters", `+N`, riskiest-env border, Cluster column (0027). The model leaves room ([switch-lifecycle.md](switch-lifecycle.md) "Room for 0027").
- Issue counts per row (0020), background polling while the switcher is closed, probing exec/auth-provider clusters without a click.
- Reordering clusters (0025 later work); persisting per-cluster scope across app runs.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Switch lifecycle: teardown, deferred connect, weak menu closures, `start_scope`/`remember_scope`, failure view with Back, status bar latency | 1–5 |
| 2 | Switcher popover: `cluster_switcher_rows.rs` model, `cluster_switcher.rs` state and render, groups, filter, segment, current mark, footer, focus | 1, 2, 6, 7 |
| 3 | Keys: Ctrl Shift C, Ctrl 1–9, in-popover arrows/Enter/Esc; 0028 reserved list | 1, 2, 8, 9 |
| 4 | Health probes, Retry and Check, cache; `--screen switcher`; ui-verifier run | 1, 2, 10–12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [switch-lifecycle.md](switch-lifecycle.md) | teardown and deferred connect, holders, pure helpers, failure UI, status bar, room for 0027 |
| [switcher-ui.md](switcher-ui.md) | popover, focus, row model, layout, interaction, empty states |
| [keys.md](keys.md) | Ctrl Shift C, Ctrl 1–9, popover keys and contexts, 0028 reserved list |
| [health-probes.md](health-probes.md) | which clusters are probed, when, concurrency, timeout, cache, cost |
| [files-to-touch.md](files-to-touch.md) | prerequisites, files per step, follow-ups |
| [test-plan.md](test-plan.md) | headless fixture, unit, headless, live, and ui-verifier checks |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. `Cargo.lock` unchanged.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [x] 3. After a switch and `run_until_parked` (two draws), the old session's weak handle cannot upgrade, and that is already true when the deferred connect of the new session starts (asserted inside `connect_active` in a test hook).
- [x] 4. Switching A → B → A restores A's namespace scope; a new cluster opens on its 0024 default namespace, else today's default (`start_scope` tests).
- [x] 5. `last_used` changes only when a session becomes Live; a failed connect shows "Cannot connect to {label}" with Retry, and "Back to {previous label}" when `previous` resolves.
- [x] 6. The popover shows W1's parts: filter input, All/Connected segment with counts, env groups with counts, badges, current mark, health line, `Ctrl n` hints for the first nine rows, footer link to Settings.
- [x] 7. Typing filters by label, context, env badge, and file name (case-insensitive); the filter has focus on open; closing restores the previous focus.
- [x] 8. `Ctrl Shift C` toggles the switcher; `Ctrl n` switches to row n of the unfiltered list from anywhere in the main window, text fields included; ↓↑, Enter, Esc work in the filter and on a row.
- [x] 9. 0028's reserved-key list no longer has `secondary-shift-c` or `secondary-1…9`; `space` was reserved for 0027. (Done by 0028: the 11 chords moved from `app_shell::bind_keys` into `keymap.rs`, and the sheet has the rows "Open cluster switcher" and "Switch to cluster 1–9". Done by 0027: `RESERVED_KEYS` dropped `space`, which `cluster_switcher::bind_keys` binds to `ToggleClusterTick`; test `space_ticks_a_row_and_never_confirms`.)
- [x] 10. Opening the switcher probes only in-process-auth clusters, at most 4 at a time, 5 s each, cached 60 s; closing aborts probes and those rows are probed again on the next open.
- [x] 11. On UAT the switcher shows `readonly@Monitor` as current with `Live · {n} ms`; the probe code calls only `open` + `server_version` (`GET /version`); the 0001 read-only grep is unchanged.
- [x] 12. Screenshot `switcher` (light, dark): no high-severity defect against W1 (ignoring 0027 parts).

## Open items

1. Exec-auth clusters are probed only on Check (decision 9); an opt-in auto-probe setting needs a user decision.
2. Issue counts per row arrive with 0020 (live session only, C4).
3. Ctrl 1–9 follow display order (env group, then registry order), so numbers shift when a cluster is imported, removed, or its environment changes; fixed numbering waits for 0025 drag reorder.
4. No warm sessions: returning to a cluster always shows "Connecting…"; 0027 may revisit.
5. Non-US layouts: Ctrl 1–9 bind the digit keys; on AZERTY the digit row needs Shift for digits, so the chord is the physical key without Shift. Rebinding is out of scope (0028 open item 2).
