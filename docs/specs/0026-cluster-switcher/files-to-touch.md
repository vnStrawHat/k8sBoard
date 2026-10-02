# 0026 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step (dead-code rule). `crates/cluster` is not touched. `Cargo.toml`/`Cargo.lock` unchanged (`buffer_unordered` is in the existing `futures` dependency).

## Prerequisites (merged first)

| Needs | Why |
|---|---|
| 0024, all steps, with its amendments: `ClusterRef` derives `Hash`; `last_used` written on Live (decision 22) | map keys; no `start_session` write to undo |
| 0025 step 1 | `Kubeconfig::connection_info` → `AuthKind` (decision 9) |
| 0025 step 2 | `ClusterCatalog`, `OpenSettings` |
| 0025 step 3 **or** the fallback | `cluster_groups`. If 0025 step 3 has not merged when 0026 step 2 starts, 0026 step 2 adds `cluster_groups` (with `ClusterGroup`) to `cluster_registry.rs` and 0025 step 3 imports it from there instead of defining it in `cluster_form.rs` |
| 0028 (code optional) | `keymap.rs` contexts; [keys.md](keys.md) covers both cases |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/cluster_registry.rs` (+ tests) | `ScopeMemory`, `remember_scope`, `start_scope` (needs `ClusterRef: Hash`, 0024) |
| 1 | `src/app_shell.rs` (+ tests) | `switch_cluster` (teardown, bookkeeping, deferred `connect_active`), `previous`, `scope_memory`, `back_to_previous`; test hook for AC 3 under `#[cfg(test)]` |
| 1 | `src/pod_drawer.rs` (line 111), `src/kind_drawer.rs` (line 143) | menu closures hold `WeakEntity<ClusterSession>` (`session.downgrade()`), upgrade on click |
| 1 | `src/cluster_session.rs` (+ tests) | `Connected.api_latency`, `LiveCluster.api_latency`; `connect_cluster` times `server_version` |
| 1 | `src/workspace.rs` | failure view title with label, optional "Back to …" action; busy view label |
| 1 | `src/status_bar.rs` (+ tests) | `API {version} · {ms} ms` |
| 2 | `src/cluster_switcher_rows.rs` (new) + `cluster_switcher_rows_tests.rs` | `SwitcherSegment`, `SwitcherRow`, `SwitcherSection`, `switcher_sections`, `visible_sections`, `nth_cluster`, `HighlightStep`, `move_highlight` (+ `cluster_groups` fallback in `cluster_registry.rs`) |
| 2 | `src/cluster_switcher.rs` (new, tests in module) | `ClusterSwitcherState`, open/close, `switcher_content` render fn with `track_focus`, footer |
| 2 | `src/title_bar.rs` | trigger inside the `Popover`; the old `DropdownMenu` and its "Manage clusters…" item removed |
| 2 | `src/app_shell.rs`, `src/main.rs` | `switcher` field; click handlers; `mod cluster_switcher; mod cluster_switcher_rows;` |
| 3 | `src/cluster_switcher.rs` | actions `OpenClusterSwitcher`, `SwitchToCluster1`…`9`, `SwitcherNext`, `SwitcherPrevious`, `SwitcherConfirm`, `CloseClusterSwitcher`; `bind_keys` (`ClusterSwitcher`, `ClusterSwitcher > Input`) |
| 3 | `src/keymap.rs` (if 0028 code merged) or `src/app_shell.rs` `bind_keys`; `src/main.rs` | `secondary-shift-c`, `secondary-1`…`9`; `RESERVED_KEYS` and sheet rows; `cluster_switcher::bind_keys(cx)` after `gpui_kit::init` |
| 4 | `src/cluster_health.rs` (new) + `cluster_health_tests.rs` | `HealthBoard` (`clear_running`, `is_running`), `ProbeEntry`, `ProbeResult`, `RowHealth`, `ProbeCandidate`, `ProbeTarget`, `is_probed_automatically`, `probe_stream`, constants |
| 4 | `src/cluster_switcher.rs` | health lines, Retry/Check (active row → `ClusterSession::retry`), probe subscriptions |
| 4 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` (+ tests) | `--screen switcher` (popover open after the catalog loads and probes settle) |

## Follow-ups for the orchestrator (specs not owned here)

- **0028** (committed): drop the 0026 rows from the `keymap.md` reserved table and `RESERVED_KEYS`; add sheet rows. `escape_stays_with_other_inputs` must use a context path **without** `ClusterSwitcher` (for example the namespace picker's `Popover > Input`), because 0026 binds `escape` in `ClusterSwitcher > Input`; 0026's own test `escape_in_switcher_filter_closes` covers `Popover > ClusterSwitcher > Input`. The same applies to any 0028 test that resolves `up`/`down`/`enter` under a popover input.
- **0028**: its Space note should point to 0027 (kit `Popover` binds `space → Confirm`; keys.md note).
- **0029**: the palette's `@` cluster scope should reuse `cluster_switcher_rows` (`switcher_sections`, `search_text`) and `switch_cluster`, and must not bind `secondary-shift-c` or `secondary-1…9`.
- **0025** (being amended by another architect): if 0026 step 2 lands first, `cluster_groups` lives in `cluster_registry.rs` (prerequisites above).

## Docs (after merge)

- `docs/roadmap/inventory-shell.md`: T2 → Done (single cluster); keyboard rows Ctrl Shift C and Ctrl 1–9 → Done.
- `docs/roadmap/cross-cutting.md` C4: probing only while the switcher is open; exec auth on click.
