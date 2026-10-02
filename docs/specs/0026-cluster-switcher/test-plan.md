# 0026 · Test plan

[Back to index](README.md). Unit tests are offline.

**Headless fixture** (`app_shell_tests.rs`, shared helper): build a `tokio::runtime::Runtime` (`new_multi_thread`, 1 worker, `enable_all`) and keep it alive for the whole test (a local bound before `cx.update`, dropped at the end); `cx.set_global(ClusterRuntime::new(runtime.handle().clone()))`; `AppSettings::install` with `WriteMode::Disabled`; a `ClusterCatalog` global over fixture kubeconfigs in `std::env::temp_dir()` that point at `https://127.0.0.1:1` (connect fails fast, no network). `gpui_kit::init`, `bind_keys`, then `open_window` as today.

## Step 1

| File | Test | Checks |
|---|---|---|
| `cluster_registry` tests | `start_scope_prefers_memory` | memory `kube-system` beats default namespace |
| | `start_scope_uses_default_namespace` | no memory → `Named(default)` |
| | `start_scope_is_none_without_memory_or_default` | |
| | `remember_scope_replaces_previous_value` | |
| `app_shell_tests.rs` | `switch_releases_the_old_session` | after the switch, `run_until_parked`, and two draws, the old weak handle cannot upgrade |
| | `old_session_is_gone_before_the_new_connect` | test hook in `connect_active` records `old_weak.upgrade().is_none()` = true (AC 3) |
| | `switch_closes_drawer_and_log_tabs` | |
| | `switch_keeps_screen_and_resets_filters` | Deployments screen stays; filter text and chips cleared |
| | `switch_to_active_target_is_noop` | session entity id unchanged |
| | `second_switch_wins_over_pending_connect` | switch A → B → C without parking between: the old weak handles cannot upgrade; the only session's `context()` is C |
| | `failed_connect_offers_back_to_previous` | fixture fails; view has "Back to {label}" |
| | `back_is_hidden_when_previous_is_gone` | `previous` no longer in the catalog |
| | `missing_target_shows_notice` | decision 20 |
| `menu` tests | `pod_menu_holds_a_weak_session` / `kind_menu_holds_a_weak_session` | after the switch, menu builders keep no strong handle (covered by the release test; compile-level: closures capture `WeakEntity`) |
| `status_bar` tests | `status_bar_shows_version_and_latency` | `API v1.29.5 · 38 ms` |
| | `latency_below_one_ms_shows_one` | |

`last_used` on Live is tested in 0024 step 3 (`last_used_is_written_on_live`, `last_used_is_not_written_on_failure`).

## Step 2 — `cluster_switcher_rows_tests.rs` and `cluster_switcher.rs`

| Test | Checks |
|---|---|
| `sections_follow_env_groups` | Production, Staging, Development · Local; counts |
| `shortcuts_number_the_first_nine_rows` | 10 rows → 1…9, tenth none |
| `active_row_is_marked` | |
| `filter_matches_label_context_badge_and_file` | `prod`, `PROD`, `config`, context-only match |
| `filter_drops_empty_sections` | |
| `connected_segment_keeps_live_and_reachable` | |
| `nth_cluster_ignores_the_filter` | |
| `highlight_moves_and_wraps` | |
| `highlight_resets_to_first_visible_on_edit` | |
| `open_focuses_the_filter` (headless) | after open and one draw, the filter's focus handle is focused (kit `track_focus`) |
| `close_restores_previous_focus` (headless) | focus in the `/` filter before open → back there after Esc |
| `footer_dispatches_open_settings` (headless) | |

## Step 3 — keys (headless)

`ctrl_shift_c_toggles`, `ctrl_digit_switches_from_inside_quick_filter`, `ctrl_digit_beyond_rows_does_nothing`, `enter_in_filter_switches_to_highlight`, `enter_on_row_switches` (focus on a row button, context `ClusterSwitcher`), `escape_in_switcher_filter_closes` (path `Popover > ClusterSwitcher > Input`), `arrows_move_highlight_in_filter`, `space_stays_unbound` (0028 `RESERVED_KEYS` updated when its code exists), `kbd_hint_resolves_in_app_shell_context`.

## Step 4 — `cluster_health_tests.rs`

| Test | Checks |
|---|---|
| `exec_and_auth_provider_are_not_auto_probed` | `is_probed_automatically` table |
| `due_skips_active_running_and_fresh` | `PROBE_TTL` with injected `Instant` |
| `stale_entry_is_due_again` | 61 s later |
| `record_clears_running` | |
| `clear_running_makes_aborted_rows_due` | M5 |
| `retry_on_running_row_is_noop` | |
| `row_health_maps_results` | Reachable(ms), Unreachable, Checking, NotChecked |
| `probe_all_limits_concurrency` | tokio test with a counting fake probe: in-flight ≤ 4 |
| `probe_timeout_is_unreachable` | `tokio::time::pause` + 5 s advance |
| `retry_on_active_failed_row_retries_the_session` | headless: phase leaves Failed |
| `closing_switcher_drops_probes` | headless: `probes` empty after close |

## Live checks (coder-lite, read-only, UAT)

`--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0026`.

1. Status bar reads `API v1.29.5 · {n} ms`; record n.
2. Seed `registry.kubeconfigs` with a fixture in `.tmp/` (token auth, `https://127.0.0.1:1`); open the switcher: that row turns `Unreachable` within 5 s; Retry re-probes.
3. Switch to it → failure view with Retry and `Back to readonly@Monitor`; Back reconnects UAT and restores the scope chosen before.
4. Code review instead of request logs (debug logging can expose headers): `probe_one` calls only `ClusterConnection::open` and `server_version`. Never print the kubeconfig.

## ui-verifier (screenshot build; writes are off)

| Screen | Seed | Expect (W1) |
|---|---|---|
| `switcher`, light and dark | fixture second cluster (`environment: production`, unreachable) + UAT | popover under the trigger; filter focused; segment `All 2 / Connected 1`; groups Production and Staging; UAT row current with check and `Live · n ms`; fixture row `Unreachable` + Retry; `Ctrl+1`/`Ctrl+2` hints; footer `Manage clusters… opens Settings Ctrl+,` |
| `pods`, seeded `last_used` | `last_used` = UAT, no `--context` | starts on UAT |

Report misaligned rows, clipped labels, a missing current mark, and color literals as defects. Checkboxes and "View N clusters" are expected to be absent (0027).
