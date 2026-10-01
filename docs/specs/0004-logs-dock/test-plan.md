# 0004 · Test plan

[Back to index](README.md)

Rules:

- No network and no cluster.
- Crate stream tests drive `log_updates` with fakes: `std::future::ready(Ok(stream))` for the open step, and `futures::stream::iter` or `futures::channel::mpsc::unbounded` for the chunks. They run under `#[tokio::test(start_paused = true)]`, as in 0002.
- App tests cover pure logic only. Layout is checked by the ui-verifier.

## `crates/cluster/src/pod_log_tests.rs`

| Test | Verifies |
|---|---|
| `log_params_current_follows_with_tail_and_timestamps` | follow, tail 1000, timestamps, container, `previous` false |
| `log_params_previous_does_not_follow` | `previous` true, follow false, tail 1000 |
| `splitter_joins_line_split_across_chunks` | `"ab"` + `"c\nd"` → `abc`, with `d` kept as the partial line |
| `splitter_strips_carriage_return` | `"x\r\n"` → `x` |
| `splitter_replaces_invalid_utf8` | `[0x66, 0xff, b'\n']` → `"f\u{FFFD}"` |
| `splitter_truncates_overlong_line_and_resumes_at_newline` | 20 KiB, then `\n`, then `ok\n` → 16 KiB + ` … [truncated]`, then `ok` |
| `splitter_finish_flushes_partial_last_line` | |
| `splitter_finish_skips_empty_partial` | |
| `parse_line_splits_rfc3339_nano_prefix` | `2024-05-01T10:47:58.902345678Z hello` |
| `parse_line_without_timestamp_keeps_whole_text` | `hello world` → `None`, `hello world` |
| `open_failure_emits_only_failed_then_ends` | a `Forbidden` open → `[Failed]` |
| `open_success_emits_started_first` | |
| `burst_within_window_emits_one_batch` | 50 lines with no time gap → one `Lines` holding all 50, in order |
| `separate_windows_emit_separate_batches` | 150 ms advanced between chunks |
| `chunk_without_newline_emits_nothing_until_end` | |
| `io_error_flushes_lines_then_fails_and_ends` | `Lines`, then `Failed(Unreachable)`, then `None` |
| `source_end_flushes_partial_line_and_ends` | |
| `dropping_stream_drops_source` | with an mpsc fake, `sender.is_closed()` after the drop |

## `crates/app/src/log_buffer_tests.rs`

| Test | Verifies |
|---|---|
| `push_appends_in_order` | |
| `push_evicts_oldest_over_line_cap` | `has_dropped` is true |
| `push_evicts_oldest_over_byte_cap` | lines of 1 MiB each |
| `change_counts_match_visible_len_without_filter` | the invariant from log-buffer.md |
| `change_counts_match_visible_len_with_filter` | |
| `batch_larger_than_cap_keeps_newest` | a batch of `MAX_LINES + 5` lines: `added_visible` equals `MAX_LINES`, and the first 5 are gone |
| `filter_keeps_only_matching_lines_ignoring_ascii_case` | `ERROR` matches `error` |
| `empty_needle_removes_filter` | also whitespace only |
| `filter_applies_to_new_lines` | |
| `eviction_removes_evicted_matches` | `visible_line` never points at an evicted line |
| `clear_keeps_filter_and_resets_lines_and_dropped` | |
| `visible_text_joins_lines_with_optional_timestamps` | |
| `find_matches_returns_non_overlapping_ranges` | `aaa` with needle `aa` gives one range |
| `find_matches_on_non_ascii_text_keeps_char_boundaries` | `"Ärger error"` with needle `error` |
| `format_log_time_is_utc_with_millis` | `10:47:58.902` |

## `log_tab.rs` and `log_dock.rs` (inline `mod tests`)

| Test | Verifies |
|---|---|
| `stream_state_started_moves_connecting_to_streaming` | |
| `stream_state_close_while_streaming_is_ended` | |
| `stream_state_close_after_failure_stays_failed` | |
| `log_target_of_pod_picks_default_container` | the first not-ready Main container; `containers` holds the summaries |
| `dock_max_height_is_sixty_percent_of_workspace` | 1000 → 600 |
| `dock_max_height_never_below_minimum` | 150 → 120 |
| `dock_max_height_unbounded_before_first_layout` | 0 → `Pixels::MAX` |
| `closing_active_tab_selects_right_then_left_neighbor` | |
| `closing_tab_before_active_shifts_active` | |
| `closing_last_tab_leaves_no_active` | `None` |

## Updated existing tests

| File | Test |
|---|---|
| `resource_actions_tests.rs` | `logs_allowed_is_enabled` replaces `logs_allowed_disabled_with_later_version_reason` |
| `resource_actions_tests.rs` | `logs_denied_reason_names_access_check`: "Not permitted: get pods/log" |
| `launch_options_tests.rs` | `parses_logs_screens`: `logs-dock` and `logs-zoomed` |
| `screenshot.rs` | `logs_screen_waits_for_log_stream`: not settled while Connecting; settled once Streaming with 0 lines, or Ended or Failed |
| `screenshot.rs` | `logs_pod_prefers_running_default_container`: falls back to `pick_drawer_pod` |

## Live checks (coder-lite)

1. Probe:
   ```bash
   cargo run -p k8sboard-cluster --example probe -- --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --logs-seconds 5
   ```
   Report the `logs …` line verbatim, then run the 0001 AC7 credential script on the output.
2. Read-only guard: run the 0001 AC4 grep on `crates/cluster/{src,examples}`, and check that `ws` is absent from the `kube` features.
3. Tracing guard: run a scoped grep for `tracing::` in `crates/cluster/src/pod_log.rs` and `crates/app/src/log_*.rs`. Report each hit. None may take `text`, a line, or a chunk.

## ui-verifier checklist

1. Capture `logs-dock` and `logs-zoomed` in light and dark (`.tmp/ui-shots/0004-<screen>-<theme>.png`). Re-capture `pods` to confirm that 0003 is unchanged when there are no tabs.
2. Compare with W8 and W8b:
   - the dock sits below the table, and the table is shorter, not covered;
   - the kit resize handle sits on the dock's top edge;
   - the tab shows the label `{pod}/{container}`, a status dot, and ✕;
   - the zoom and minimize icons are on the right;
   - the toolbar has the container picker, the filter, Previous, Timestamps, Wrap, Copy, and the status text;
   - rows are mono, with a muted time column, and there is no gap between rows;
   - zoomed covers the header, the table, and the drawer, while the title bar, sidebar, and status bar stay visible.
3. Resize, minimize, Wrap, and the filter cannot be captured headlessly. The user spot-checks them on UAT (README AC6).
