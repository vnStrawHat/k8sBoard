# 0059 · Test plan

[Back to index](README.md) · All offline; the shell tests use the fake API of `app_shell_edit_tests.rs` (`t.puts()` records every `PUT`).

## `crates/app/src/yaml_edit_tests.rs` (pure, `#[test]`)

| Test | Checks |
|---|---|
| `local_diff_of_the_base_text_has_no_rows` | `local_diff_rows(base, base) == Ok(vec![])` for a serializer-form base (`format_yaml` of a sample) |
| `a_whitespace_or_key_order_change_has_no_rows` | the same object re-indented, with extra blank lines and keys in another order → `Ok(vec![])` |
| `a_header_comment_change_has_no_rows` | the leading `#` line removed or edited → `Ok(vec![])` |
| `a_value_change_has_a_removed_and_an_added_row` | `replicas: 3` → `5`: exactly one `Removed` and one `Added`, texts `replicas: 3` / `replicas: 5` |
| `a_text_that_does_not_parse_is_an_error` | `"a: ["` → `Err(EditError::Syntax { .. })`; `"- a"` → `Err(NotAnObject)` |
| `footer_follows_the_preview_and_the_text` (changed) | drops the `rows` field of `PassedPreview`; assertions unchanged |

## `crates/app/src/app_shell_edit_tests.rs` (`#[gpui_kit::test]`)

Helpers: `t.dry_run(cx)` (`view.dry_run(cx)`), `t.wait_for_local_diff(cx)`, `t.local_rows(cx) -> Option<Result<Vec<DiffRow>, SharedString>>` (from `local_diff_for_test`), `t.apply_reason(cx)` (`apply_block_reason(&text)`).

### New

| Test | Steps → expected |
|---|---|
| `restoring_the_base_text_after_a_dry_run_leaves_the_diff_empty` | **stale-rows regression.** change `replicas: 3`→`5`, `dry_run`, wait, `is_passed`; `set_text(base_text)`; show Diff, wait → `local_rows == Some(Ok([]))`; `apply_reason == Some("No changes")` |
| `showing_the_diff_never_sends_a_request` | change, show Diff, `run_until_parked` → `t.puts()` empty; rows hold an `Added` `replicas: 5`; no row text contains `s3cr3t-env`; preview still `NotChecked` |
| `the_dry_run_button_sends_one_dry_run_and_keeps_the_tab` | change, `dry_run`, wait → one `PUT`, `dryRun=All`, `fieldManager=k8sboard`, path `PATH`; tab `Editor`; passed changes `["spec.replicas"]` 3 → 5; checks start with `kubectl apply users:` (absorbs `ctrl_s_runs_the_preview_and_shows_diff`) |
| `apply_is_off_until_the_dry_run_passes` | change → `apply_reason == Some("Run the dry-run first")`; `t.apply` → no dialog, no `PUT`; `dry_run`, wait → `apply_reason == None`; `t.apply` → dialog `Apply changes` |
| `ctrl_s_does_nothing_before_the_dry_run` | change, `press("ctrl-s")`, `run_until_parked` → no `PUT`, no dialog, tab `Editor`, preview `NotChecked` |
| `a_format_only_difference_shows_no_changes` | text = base text re-indented / keys reordered; show Diff, wait → `Some(Ok([]))` |
| `a_parse_error_shows_in_the_diff_tab` | text `"a: ["`; show Diff, wait → `Some(Err(message))`, message starts `YAML error at line`; no `PUT` |
| `the_diff_follows_the_text_while_it_is_shown` | change, show Diff, wait → rows; `set_text(base_text)` while on Diff, wait → `Some(Ok([]))` (the `refresh_dirty` trigger) |
| `the_dry_run_is_off_while_clean_or_running` | clean → `dry_run_block_reason == Some("No changes")`; change, `dry_run` → `Some("Dry-run running…")`; a second `dry_run` before the answer sends nothing (one `PUT`) |

### Changed

| Test | Change |
|---|---|
| every test that starts the check with `t.apply` before `wait_for_preview` | that first call becomes `t.dry_run(cx)`; a second `t.apply` stays (it opens the dialog) |
| `second_ctrl_s_opens_the_confirm_dialog` | → `apply_after_a_passed_dry_run_opens_the_confirm`: `dry_run`, wait, `apply` → dialog; two `PUT`s (the check and the dialog's) |
| `stale_preview_needs_a_new_check` | after the second change: `apply_reason == Some("Run the dry-run first")`, `t.apply` opens nothing and sends nothing (1 `PUT`); `dry_run` → 2 `PUT`s, passed |
| `reload_and_keep_my_changes_rebases_and_checks_again` | → `reload_and_keep_my_changes_rebases_without_a_check`: after the rebase no `PUT` and preview `NotChecked`; then `dry_run` → one `PUT` whose body has `resourceVersion` `200` and label `tier: backend` |
| `a_held_ctrl_s_never_opens_the_confirm_dialog` | the first fresh press becomes `dry_run`; held press → no dialog; fresh press → dialog |
| `apply_is_off_after_a_secret_value_is_refused_and_on_again_after_an_edit` | reasons via `apply_reason`: after the change `Some("Run the dry-run first")` (was `None`); after the refusal the Secret reason, also from `dry_run_block_reason`; after the next change `Some("Run the dry-run first")` |
| `apply_stays_enabled_when_quota_exceeds`, `quota_feed_off_says_not_checked_and_adds_no_warning` | `apply_block_reason(&text)` signature |
| `local_error_stays_on_editor_tab` | started by `dry_run`; still `Editor`, no `PUT` |

### Deleted

`ctrl_s_while_running_does_nothing` (covered by `the_dry_run_is_off_while_clean_or_running`), `opening_the_diff_runs_the_dry_run_without_ctrl_s`, `the_diff_asks_again_only_for_a_text_no_check_covers`, `ctrl_s_after_the_diff_opened_goes_straight_to_the_confirm` (decision 31 and 35 reverse them; the new tests above cover the replacement behavior).

## `crates/cluster/src/edit_preview_tests.rs`

| Test | Change |
|---|---|
| `preview_masks_both_sides` | → `preview_changes_are_masked`: no `FieldChange` old/new contains `s3cr3t-env` or `replaced-literal`; the changed env value reads `<hidden, changed>` |
| `moved_placeholders_are_marked_and_checked` | the `<hidden, moved>` assertion reads the change of the moved path instead of `preview.after` |
| `debug_shows_counts_only` | the literal loses `before` / `after` |

## Manual and UI checks

- `--screen edit-yaml-diff` (screenshot build): Diff rows `replicas 3 → 5` and `memory 512Mi → 1Gi` only, side panel `2 changes`, `Server dry-run passed`, quota line; footer shows `Dry-run` and `Apply…`, both disabled (fixture). ui-verifier against W10.
- No live cluster run is needed: the change sends fewer requests than before (the Diff tab sends none). The UAT denied path is unchanged.
