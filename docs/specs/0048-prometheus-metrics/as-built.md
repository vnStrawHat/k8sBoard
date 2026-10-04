# 0048 · As built

[Back to index](README.md). Records what differs from the spec and the facts the live checks settled. Steps 1 to 4 and the review fixes.

## Steps 1 and 2: merged commit

Clippy flags the step 1 items as dead code until `metrics_query.rs` reads them (`usage_query`, `proxy_path`, `InvalidName`), so steps 1 and 2 ship as one commit, as [files-to-touch.md](files-to-touch.md) allows. Gate: fmt, clippy (workspace and `--features screenshot`), and tests pass; `Cargo.lock` gains only the two dependency lines `form_urlencoded` and `http-body-util` under `k8sboard-cluster`, no new `[[package]]`.

## Deviations

| # | Spec | Built | Why |
|---|---|---|---|
| 1 | Node selector `id="/",node="{node}"` | `sum(...{id=~"/|",pod="",node="{n}"}) or sum(...{id=~"/|",pod="",kubernetes_io_hostname="{n}"})` (`id=~"/|"` also matches an absent label; decision 16) | UAT facts below: the scrape drops the `id` label, and `node` is wrong on workers |
| 2 | `slow_body_times_out`: "body never ends" | The fake never answers the head (`Failure::Hang`) | kube's `Body` has no public constructor for a stalled stream. The test covers the same single 20 s deadline around `send` and the body read |
| 3 | `endpoints_are_fixed`: "the three endpoint strings" | Two (`query`, `query_range`) | 0049 adds the third |
| 4 | Probe line: "series, points, bytes, ms" | points with data, total points, `was_cut`, ms | The typed public API returns `UsageSeries` only; series and byte counts appear in the crate's `read metrics` debug trace |
| 5 | `regex_literal` (`pub(crate)`) | Not built; `regex_escape` (private) plus `string_literal` | Only the workload pattern needs it and that pattern mixes escaped names with unescaped regex pieces; test `regex_literal_escapes_metacharacters` covers the pair |
| 6 | Step table location unspecified | `pub const RANGE_STEPS` and `RangeSpec::ending_at(now, span)` in `promql.rs` | Step 4 needs one place for the span-to-step map and the "end rounded down to the step" rule |
| 7 | `MetricsError::Unexpected` for a transport error: `{message}` | A fixed text per `kube::Error` family | `kube::Error::Auth` and TLS texts can carry secrets (the 0002 rule) |
| 8 | Node Disk I/O from the source | `usage_range` for `Node` with `DiskRead` or `DiskWrite` builds no query and returns `MetricsError::Unsupported` (decision 17) | Device-mapper aliases double count; Node disk stays on the kubelet feed. Pod and workload disk use the source |

## Live facts (UAT `readonly@Monitor`, VictoriaMetrics cluster, via `probe --metrics-source`)

| Fact | Result |
|---|---|
| Check query | reachable, 32 to 165 ms, 318 `container_cpu_usage_seconds_total` series |
| 30d `query_range`, node CPU, step 2 h | 361 of 361 points with data, 71 to 90 ms |
| Root-cgroup series carry `id="/"` | **No `id` label at all.** The root cgroup is the series with `pod=""` (4 CPU series, one per node), with a `node` label. Its CPU rate matched node-exporter within 3 % on all four nodes |
| `node` label value | **Wrong for workers**: `master-01` is right, the three workers all carry `node="mon"`. `kubernetes_io_hostname` and `instance` on the same series hold the real node names (`worker-01` to `-03`). `kube_node_info` has the correct `node` |
| Node queries per node, first selector only (1h, has data) | node #0 (`master-01`): CPU, memory, network true. Nodes #1 to #3: all false |
| Node queries per node, with the `or` on `kubernetes_io_hostname` (decision 16) | All four nodes: CPU, memory, network-rx, network-tx true; 30d node CPU 361 of 361 points, 73 ms |
| node-exporter alternative | Series carry `instance` (`IP:9101`) and `pod`, no node-name label; `node_uname_info{nodename}` joins by `instance` |
| `container_fs_{reads,writes}_bytes_total` | Exist (659 series, 612 with a pod). The node-level (`pod=""`) series list each device and its device-mapper alias (`/dev/dm-1` and `/dev/mapper/...`), so a node sum **double counts** and is far above node-exporter's `node_disk_*`; pod-level disk is not affected |
| Pod network counted once | Yes: 5 busiest pods on the first node, source rate over kubelet summary rate 0.87 to 1.05 and 0.81 to 0.91 on two runs; 5 of 5 within 20 % |
| Node memory | cAdvisor root working set is about 30 % above node-exporter used memory (different definitions) |

## Decided after the first review

1. Node subjects: option A, `node` first and `kubernetes_io_hostname` second joined with `or` (decision 16; deviation 1).
2. Node Disk I/O stays on the kubelet feed; pod and workload disk may use the source (decision 17; deviation 8).

## Security review fixes (07726bb)

Error texts: a 401 `Status` maps to `Denied("credentials were rejected")` without the backend message; an empty `Status` or backend message gets a fixed text; `one_line` also drops bidi and format characters; a Job pod pattern accepts an Indexed Job's index.

## Steps 3a, 3b, and 4 (app)

| # | Spec | Built | Why |
|---|---|---|---|
| 9 | 3a and 3b as two commits | One commit | `SourceState` fields and `ActiveConnection` are read only by the Metrics page; clippy flags them as dead code alone |
| 10 | `source_note_texts` and `SourceState::ready` in 3a | In step 4 | Their reader is the Monitor tab |
| 11 | `ActiveConnection { cluster, label, connection }` | Also `session` (weak) and `generation` | The page shows the session's `SourceState` and tells a reconnect from the same connection |
| 12 | `AppShell.source_fetch` | `MonitorState.source: Option<SourceFetch>` | The tab reads it through `MonitorView`; another drawer subject drops it with its state |
| 13 | `SourceFetch.result` | `SourceFetch.view` (the built `SourceView`) and `last_failure` | The charts are built once when an answer lands, so a repaint copies nothing |
| 14 | `sync_monitor_source` next to `sync_kubelet_demand` | Called before `refresh_monitor_cache` | A long range reset to 24h must be seen by the cache of the same frame |
| 15 | `pod-monitor-source-fixture` has no cluster | The first pod of the connected cluster, 30d, synthetic source data, no request to a source | The drawer needs a live session, which offline fixtures of the app do not have |
| 16 | `source_monitor_data(input, result)` reads OOM from the sampler history | `SourceResult.oom` (read from the history when the answer lands; the fixture sets its own) | One place decides the markers |
| 17 | x axis label of a long range | `-7d`, `-30d` (whole days from 7) | `-720h` is unreadable |
| 18 | Test `ranges_follow_the_source_state`, `range_click_indexes_the_shown_set` | `ranges_follow_the_source` (the sets), the click indexes `shown` in the same function | The toolbar is drawn by GPUI code; the set is the testable part |

Live checks (UAT, light theme, `.tmp/shots-0048/`): `settings-metrics` lists the two VictoriaMetrics rows with the saved vmselect row selected and `Saved: … · Ready`; `pod-monitor` with the saved entry shows six ranges, `step 15s · metrics source`, and the source footer; `pod-monitor-source-fixture` shows 30d with `step 2h · metrics source`, the OOM marker, and the footer.

## Final review fixes

| # | Change |
|---|---|
| 19 | `Choice::Candidate` holds the candidate's fields, not its index; Save and Test are off while the list is being read (a Detect again could reorder the rows and wipe the saved source). |
| 20 | Detection starts at the first draw of the Metrics page, not when the Settings window opens on another page. |
| 21 | The Other-service inputs are cleared on a switch to a cluster with no saved entry. |
| 22 | The node Disk I/O card follows `input.range.resolution()`. |
| 23 | A new live connection keeps a `Ready` source state when the stored entry is unchanged. In production a session goes Live once (Retry runs only after a failure), so the seam `go_live_for_test` tests it. |
| 24 | The `ActiveConnection` global is removed when the shell is released. |
| 25 | `pod-monitor-source-fixture` forgets the stored entry in its session, so it sends no check query (deviation 15 holds). |
| 26 | The fallback line reads `Metrics source: {reason}. Showing k8sBoard samples.` (the spec text said `Prometheus:`). |
| 27 | `registry.clusters[].metrics` is read leniently (`StoredMetrics`): a bad entry (`scheme: "ftp"`, a missing port) becomes `MetricsSourceError::Unreadable`, state `Invalid`, no request, and is written back as it was; the rest of the file loads. |
| 28 | `one_line` also drops U+061C. |
