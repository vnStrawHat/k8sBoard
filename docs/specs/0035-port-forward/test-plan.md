# 0035 · Test plan

[Back to index](README.md). Offline and deterministic. **No test, probe, or agent run opens a port-forward to any cluster.** Transport tests use the 0030 `FakeApi` (tower `service_fn`) for GETs and upgrade refusals, and `tokio::io::duplex` behind a private seam for the socket body; listeners bind `127.0.0.1:0` in tests. Paused tokio time for delays.

## Step 1 — cluster crate (`port_forward_tests.rs`, `access_review` tests)

| Test | Checks |
|---|---|
| `permit_needs_get_and_create` | both allowed → `Some`; either denied or missing → `None` |
| `get_port_forward_check_targets_the_subresource` | SSAR `get`, `""`, `pods`, `portforward`, namespaced |
| `blocked_policy_sends_nothing` | `WritePolicy::Blocked` → `Ended(WritesBlocked)`, zero recorded requests |
| `listener_binds_loopback_only` | `Listening.local.ip() == 127.0.0.1`; the second listener (when up) is `[::1]` on the same port; nothing else is bound |
| `ipv6_listener_failure_is_ignored` | `[::1]` bind error (seam) → still `Listening { has_ipv6: false }`, forward runs |
| `connections_from_both_listeners_share_the_set` | one accept on each → `open_connections == 2` |
| `exact_port_in_use_ends_with_port_in_use` | pre-bound port + `Exact` → `Ended(PortInUse(p))` |
| `bind_permission_denied_is_treated_as_in_use` | `Auto(p)` + `PermissionDenied` on `p` (bind seam) → `p+1` |
| `exact_reserved_port_ends_port_reserved` | `Exact(p)` + `PermissionDenied` → `Ended(PortReserved(p))`, text `Port p is reserved by the system` |
| `auto_port_moves_to_the_next_free_port` | pre-bound `p` + `Auto(p)` → `p+1` |
| `candidate_ports_then_os_assigned` | `p..=p+20`, then `0` |
| `default_local_port_table` | 5432 → 15432; 8080 → 18080; 60000 → 60000 |
| `ready_pod_skips_unready_and_deleting` | Pending, Ready=false, deleting skipped; first by name |
| `service_target_port_number_name_and_default` | int → int; name → container port; absent → service port; unknown name → `PortNotDeclared` |
| `service_without_selector_is_unsupported` | → `UnsupportedService` |
| `deployment_resolves_through_its_selector` | matchLabels + matchExpressions → matching ready pod (FakeApi list) |
| `upgrade_403_ends_not_permitted` | `ProtocolSwitch(403)` → `Ended(NotPermitted)`, fixed text |
| `upgrade_401_ends_unauthorized` | → `Ended(Unauthorized)` |
| `upgrade_404_checks_the_pod_then_reconnects` | 404 + GET 404 → `ConnectionLost`, `Reconnecting{1}` |
| `live_pod_does_not_trigger_reconnect` | socket error + GET Running → `ConnectionLost` only |
| `reconnect_backoff_is_1_5_15_30_60` | delays between attempts with `tokio::time::advance` |
| `reconnect_success_emits_reconnected_and_resolved` | attempt 2 finds a pod → `Reconnected`, `Resolved` |
| `five_failures_end_with_target_lost` | → `Ended(TargetLost)`; listener closed after |
| `connections_during_reconnect_are_closed` | accept while reconnecting → peer sees EOF |
| `connection_limit_closes_the_65th` | 64 open + 1 → closed, one `ConnectionLimit` |
| `traffic_counts_both_directions_once_per_second` | duplex fixture 10 B up, 20 B down → `sent 10, received 20`, one sample per tick |
| `error_channel_ends_the_socket_during_the_copy` | error-channel message while the copy is idle → socket ends at once, `ConnectionRefused` |
| `reason_text_strips_and_caps_every_reason` | error channel, WebSocket close, and resolve error texts: control characters stripped, 200 chars |
| `refuse_control_closes_new_connections_only` | `Refuse` → `Paused`, new accept closed, an open socket keeps copying; `Accept` → `Resumed` |
| `dropping_the_stream_aborts_sockets` | abort guard observed via a drop flag in the seam |
| `debug_shows_no_payload` | `format!("{:?}")` of updates and errors holds no byte content |

## Step 2 — app model and page

`port_forwards_tests.rs`: `apply_transition_table` (every update → state, local, pod, counters), `status_text_matches_the_wireframe` (`Reconnecting 2/5`, `Port 3000 in use`, `Stopped · preset`), `events_keep_the_last_20`, `stop_removes_non_preset_rows`, `stop_keeps_preset_rows_stopped`, `presets_load_as_stopped_rows`, `preset_round_trips_and_has_no_secret_keys`, `save_as_preset_dedups`, `running_for_matches_cluster_namespace_target_port`, `own_port_conflict_is_refused`, `parse_target_accepts_pod_svc_deploy_sts`, `local_port_for_typed_is_exact`.

Window tests (`app_shell_tests.rs`): `status_bar_shows_running_count_and_opens_the_page`, `page_lists_forwards_of_every_cluster`, `switch_cluster_keeps_forwards` (0026 switch with a fixture forward row holding a dummy subscription), `release_slot_keeps_forwards` (0027).

## Steps 3a and 3b — gate, start, menus, dialogs

| Test | Checks |
|---|---|
| `port_forward_needs_get_and_create` | both → Enabled; one denied → `Not permitted: get and create pods/portforward`; checking/unknown → disabled |
| `locked_cluster_pauses_running_forwards_and_blocks_start` | lock → `Refuse` sent, row `Paused · read-only`; Start shows `{cluster} is read-only`; unlock → `Accept`, row Active |
| `start_needs_a_viewed_cluster` | preset of a non-viewed cluster → `Open {cluster} to start this forward` |
| `run_guarded_picks_the_port_forward_permit` | `ConnectOpen::PortForward` gets a permit; no permit → `open` not called, no audit |
| `start_appends_one_audit_line` | action `Port-forward`, object, fields `remote_port`, `local_port`; no traffic keys |
| `forward_button_states` | Offer, Live (`● localhost:19090 · Stop`), Disabled (UDP, no selector, gate reason) |
| `pod_menu_lists_tcp_ports_with_tags` | MAIN/SIDECAR tags; one port → direct item |
| `f_key_with_several_ports_opens_new_forward` | dialog prefilled with the cursor pod |
| `forward_uses_the_rows_cluster` | 0027 fixture: drawer of cluster B → guard B, never the primary |
| `twenty_first_forward_is_refused` | notice, no start |

## Live checks (coder-lite, UAT, denied path only, debug build)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0035-<case>`; never `K8SBOARD_ALLOW_WRITES`.

1. Record the session SSAR answers for `get pods/portforward` and `create pods/portforward` (trace of the SSAR responses: allowed true/false only). If **both** are allowed, stop and report; the check never presses Forward.
2. A pod drawer Containers tab: Forward disabled with `Not permitted: get and create pods/portforward`; F on a pod row shows the notice; a Service drawer port the same; the page `+ New forward` → Forward is refused by the gate with the same text.
3. `RUST_LOG=kube=trace`: no request path contains `/portforward`; only GETs (lists/watches) and SSAR POSTs. Counts only; never copy headers.
4. Port Forwarding page: empty state; status bar shows no `⇄` item.

## ui-verifier

`--screen port-forwards`, light and dark: W7 page columns, five fixture rows with the right tokens, drawer sections Forward / Traffic / Recent events, ⋯ menu order. Report color literals, clipped text, missing env badges.

## Write-capable cluster (R2, later, user-run)

With `K8SBOARD_ALLOW_WRITES=1` set by the user: forward a pod and a Service, curl through it, delete the pod (StatefulSet) and watch `Reconnecting n/5` → Active, lock the cluster and see new connections refused, occupy a port to see the conflict, switch clusters and confirm the forward survives.
