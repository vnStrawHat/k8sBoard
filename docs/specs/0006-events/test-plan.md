# 0006 · Test plan

[Back to index](README.md). **S** is the step that adds the test. Tests are deterministic and offline; fixtures are k8s-openapi structs with `..Default::default()`; each test checks one behavior.

## `crates/cluster` (step 1)

| File | Test |
|---|---|
| `event.rs` | `legacy_event_reads_count_and_last_timestamp` |
| `event.rs` | `event_series_overrides_count_and_last_seen` |
| `event.rs` | `new_style_singleton_counts_one_and_uses_event_time` (count 0, only `eventTime`) |
| `event.rs` | `first_seen_falls_back_to_event_time_then_creation` |
| `event.rs` | `last_seen_falls_back_to_first_seen` |
| `event.rs` | `only_warning_type_is_warning` (`Warning`, `Normal`, empty, missing) |
| `event.rs` | `event_source_picks_component_and_host_per_field` (source only, reporting only, component from source with host from `reportingInstance`, host alone, none) |
| `event.rs` | `involved_object_empty_namespace_is_none` |
| `event.rs` | `message_is_trimmed_and_keeps_inner_newlines` |
| `event.rs` | `message_truncates_on_a_char_boundary` (a multi-byte char straddles 1,024; ends with `…`; ≤ 1,024 bytes before it) |
| `event.rs` | `short_message_is_unchanged` |
| `event.rs` | `event_summary_drops_labels_and_annotations`: distinctive label and annotation values are absent from `format!("{summary:?}")` |
| `event.rs` | `warnings_only_selects_warning_type` (`All` → `None`) |
| `event.rs` | `object_selector_names_kind_and_name` |
| `event.rs` | `cluster_scoped_object_events_read_default_namespace` (pure `fn object_events_namespace(object) -> &str`: Pod → its namespace, Node → `default`) |
| `resource_watch_tests.rs` | `store_limit_evicts_oldest_on_apply` |
| `resource_watch_tests.rs` | `store_limit_unknown_recency_is_evicted_first` |
| `resource_watch_tests.rs` | `store_limit_new_item_older_than_all_is_not_a_change` |
| `resource_watch_tests.rs` | `store_limit_keeps_newest_after_relist` |
| `resource_watch_tests.rs` | `store_limit_trims_relist_buffer_at_twice_the_limit` |
| `resource_watch_tests.rs` | `limited_watch_snapshot_holds_at_most_limit` (`batch_updates` with a fake source, paused time) |

## `crates/app`

| S | File | Test |
|---|---|---|
| 2 | `resource_kind.rs` | `object_kinds_round_trip` (every kind; `"Pod"` → `None`) |
| 2 | `resource_kind.rs` | `every_kind_ends_with_a_right_aligned_age_column` (updated: `Age`, or `Last seen` for Events) |
| 2 | `resource_kind.rs` | `only_events_hide_the_name_column` (and `flexible` points at Message) |
| 2 | `resource_kind.rs` | `only_events_have_no_labels` |
| 2 | `kind_table.rs` | `cell_index_skips_name_only_when_shown` |
| 2 | `kind_table.rs` | `events_columns_flex_message_with_minimum_width` |
| 2 | `kind_table.rs` | `fit_width_resizes_the_flexible_column` (Events: Message grows; Deployments: Name, as before) |
| 2 | `event_rows.rs` | `event_row_cells_match_column_count` |
| 2 | `event_rows.rs` | `events_sort_newest_first_with_unknown_last` (ties by namespace and name) |
| 2 | `event_rows.rs` | `event_tone_warns_on_warning_and_mutes_normal` |
| 2 | `event_rows.rs` | `object_text_lowercases_kind` (`pod/api`, name alone when kind is empty) |
| 2 | `event_rows.rs` | `object_key_maps_viewable_kinds` (Pod, Node, Deployment, Namespace with no namespace; `HorizontalPodAutoscaler` and `Event` → `None`) |
| 2 | `event_rows.rs` | `message_line_joins_lines` |
| 2 | `event_rows.rs` | `event_title_is_reason_and_object_name` (object name alone without a reason) |
| 2 | `event_rows.rs` | `event_row_has_details_and_message_sections` (titles `Details` and `Message`; full message in `Code`) |
| 2 | `table_selection_tests.rs` | `resource_key_screen_matches_kind` |
| 2 | `table_selection_tests.rs` | `pending_reveal_key_resolves_through_selection_sync`: a key kept while the list loads gives `row_index` → `selection_sync(None, Some(i))` = `Move(i)` once rows arrive, and `Clear` when its row is absent |
| 2 | `workspace.rs` | `group_digits_inserts_thousands_separators` (`999`, `2,000`, `12,345`) |
| 2 | `navigation.rs` | `enabled_items_are_pods_nodes_and_explorer_kinds` (adds Events) |
| 2 | `navigation.rs` | `denied_kind_reason_names_all_namespaces_scope` holds for Events ("list events in all namespaces") |
| 2 | `launch_options_tests.rs` | `kind_screens_parse_from_plural_slugs` covers Events through `ALL` (no edit) |
| 3 | `object_events.rs` | `event_subject_maps_pods_nodes_and_kinds` (Pod, Node with no namespace, Deployment, Namespace; Events → `None`) |
| 3 | `object_events.rs` | `subject_change_keeps_stops_and_starts` (equal → `Keep`; `None` → `Stop`; different or none running → `Start`) |
| 3 | `object_events.rs` | `event_note_by_list_state` (all four rows of the table) |
| 3 | `object_events.rs` | `events_title_counts_only_ready_lists` |
| 3 | `cluster_session_tests.rs` | `open_watch_count_includes_object_events` (3, 4, 5) |
| 3 | `launch_options_tests.rs` | `pod_events_screen_opens_pods_with_drawer` |
| 3 | `screenshot.rs` | `drawer_waits_for_object_events` (`is_drawer_ready` false while pending) |

Step 4 adds no pure logic beyond `warnings_only_selects_warning_type`; it is covered by AC 11.

## Live checks (coder-lite)

| S | Check |
|---|---|
| 1 | `cargo run -p k8sboard-cluster --example probe -- --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --watch-seconds 5`: 14 watch lines (events ≤ 2,000, warning events ≤ events) and 19 access lines; record whether `list events` is allowed. Then the 0001 AC7 credential script: every count is 0 |
| 2 | App `--screen events` and `events-drawer` into `.tmp/ui-shots/`, with the 0003 PNG checks |
| 3 | `pod-events`, `node-drawer` (events from `default`), `deployments-drawer` |
| 4 | Toggle Warnings only on UAT (manual run, or a coder-lite screenshot after a temporary local toggle that is not committed); record row counts with and without it |

## ui-verifier checklist (step 2 for the screen, step 3 for all, step 4 for the button)

- Events table: no Name column; Type, Reason, Object, Message, Count, Last seen; Message flexible; Count and Last seen right-aligned; Warning toned, Normal muted; Object namespace prefix muted; newest first.
- Header: count, scope, ` · newest 2,000` only at the cap; step 4: the Warnings only button (outline; selected style when on).
- Event drawer: badge `Ev`, title `{reason} · {object}`, subtitle `{type} · {namespace} · {source}`; Details section with a link-colored Object; Message in a muted code block that wraps; no Labels section.
- Menus: Go to object, Copy message, View YAML (disabled), Copy name, Delete event… (disabled), separated in that order.
- Pod Events tab, Node and Deployment Events sections: rows toned, `×n`, ages, clamped messages, no hover or pointer, tooltip with the full message, "No recent events" when empty.
- Light and dark: no contrast defects. Files: `.tmp/ui-shots/0006-<screen>-<theme>.png`; dark for `events` and `events-drawer`.
