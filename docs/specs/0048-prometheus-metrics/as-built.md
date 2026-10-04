# 0048 · As built

[Back to index](README.md). Records what differs from the spec and the facts the live checks settled. Steps 1 and 2 (cluster crate) so far.

## Steps 1 and 2: merged commit

Clippy flags the step 1 items as dead code until `metrics_query.rs` reads them (`usage_query`, `proxy_path`, `InvalidName`), so steps 1 and 2 ship as one commit, as [files-to-touch.md](files-to-touch.md) allows. Gate: fmt, clippy (workspace and `--features screenshot`), and tests pass; `Cargo.lock` gains only the two dependency lines `form_urlencoded` and `http-body-util` under `k8sboard-cluster`, no new `[[package]]`.

## Deviations

| # | Spec | Built | Why |
|---|---|---|---|
| 1 | Node selector `id="/",node="{node}"` | `id=~"/\|",pod="",node="{node}"` (`id=~"/\|"` also matches an absent label) | UAT facts below: the scrape drops the `id` label, so `id="/"` matches nothing |
| 2 | `slow_body_times_out`: "body never ends" | The fake never answers the head (`Failure::Hang`) | kube's `Body` has no public constructor for a stalled stream. The test covers the same single 20 s deadline around `send` and the body read |
| 3 | `endpoints_are_fixed`: "the three endpoint strings" | Two (`query`, `query_range`) | 0049 adds the third |
| 4 | Probe line: "series, points, bytes, ms" | points with data, total points, `was_cut`, ms | The typed public API returns `UsageSeries` only; series and byte counts appear in the crate's `read metrics` debug trace |
| 5 | `regex_literal` (`pub(crate)`) | Not built; `regex_escape` (private) plus `string_literal` | Only the workload pattern needs it and that pattern mixes escaped names with unescaped regex pieces; test `regex_literal_escapes_metacharacters` covers the pair |
| 6 | Step table location unspecified | `pub const RANGE_STEPS` and `RangeSpec::ending_at(now, span)` in `promql.rs` | Step 4 needs one place for the span-to-step map and the "end rounded down to the step" rule |
| 7 | `MetricsError::Unexpected` for a transport error: `{message}` | A fixed text per `kube::Error` family | `kube::Error::Auth` and TLS texts can carry secrets (the 0002 rule) |

## Live facts (UAT `readonly@Monitor`, VictoriaMetrics cluster, via `probe --metrics-source`)

| Fact | Result |
|---|---|
| Check query | reachable, 32 to 165 ms, 318 `container_cpu_usage_seconds_total` series |
| 30d `query_range`, node CPU, step 2 h | 361 of 361 points with data, 71 to 90 ms |
| Root-cgroup series carry `id="/"` | **No `id` label at all.** The root cgroup is the series with `pod=""` (4 CPU series, one per node), with a `node` label. Its CPU rate matched node-exporter within 3 % on all four nodes |
| `node` label value | **Wrong for workers**: `master-01` is right, the three workers all carry `node="mon"`. `kubernetes_io_hostname` and `instance` on the same series hold the real node names (`worker-01` to `-03`). `kube_node_info` has the correct `node` |
| Node queries per node (1h, has data) | node #0 (`master-01`): CPU, memory, network, disk all true. Nodes #1 to #3: all false |
| node-exporter alternative | Series carry `instance` (`IP:9101`) and `pod`, no node-name label; `node_uname_info{nodename}` joins by `instance` |
| `container_fs_{reads,writes}_bytes_total` | Exist (659 series, 612 with a pod). The node-level (`pod=""`) series list each device and its device-mapper alias (`/dev/dm-1` and `/dev/mapper/...`), so a node sum **double counts** and is far above node-exporter's `node_disk_*`; pod-level disk is not affected |
| Pod network counted once | Yes: 5 busiest pods on the first node, source rate over kubelet summary rate 0.87 to 1.05 and 0.81 to 0.91 on two runs; 5 of 5 within 20 % |
| Node memory | cAdvisor root working set is about 30 % above node-exporter used memory (different definitions) |

## Open for the architect

1. **Node subjects on UAT.** With the built selector a worker node returns no data (wrong `node` label), so step 4 would fall back to the sampler for 3 of 4 UAT nodes. Options: (A) match `node` first and `kubernetes_io_hostname` second with `sum(...) or sum(...)` (works on UAT and on stacks where `node` is right); (B) remove `UsageTarget::Node` as the spec's "none usable" branch says; (C) accept the fail-safe (empty answer, sampler fallback).
2. **Node disk I/O** from the source double counts on LVM hosts; consider keeping Node Disk I/O on the kubelet feed whatever option 1 picks.
