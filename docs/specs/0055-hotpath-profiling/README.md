# 0055 — hotpath profiling (developer feature)

Status: **implemented 2026-10-05**, requested by the user ("follow OneTerm"). Read-only: no Kubernetes call, no write path. Crates: `crates/app`, `crates/cluster`. Model: OneTerm `IN-0046` (hotpath-rs 0.26).

## Goal

Function-level time and allocation profiling of k8sBoard's hot paths with the `hotpath` crate. A developer builds with one cargo feature, runs a screen, and reads a table: calls, average, percentiles, share of the run (timing), or bytes allocated per function (allocation). It is **off by default and costs nothing when off**: every site is `#[cfg_attr(feature = "hotpath-profiling", hotpath::measure)]`, so without the feature the attribute does not exist and `hotpath` is not in the build graph.

## Features

| Feature (on `k8sboard`) | Effect |
|---|---|
| `hotpath-profiling` | `dep:hotpath` + `hotpath/hotpath` (timing) + `k8sboard-cluster/hotpath-profiling`. `run()` carries `#[hotpath::main]`; the guard lives for `run()`. |
| `hotpath-profiling-alloc` | The above + `hotpath/hotpath-alloc`: hotpath's counting allocator becomes the global allocator, wrapping `live_heap::LiveHeapAllocator`. Adds bytes-allocated tables and the live-heap printer. |

`k8sboard-cluster/hotpath-profiling` only adds the optional `hotpath` dependency; a crate built alone with it compiles hotpath's no-op half. Only `k8sboard` turns measuring on.

## How to run

```bash
# timing
cargo build -p k8sboard --features hotpath-profiling
# allocations (bytes per function) and the live Rust heap on stderr
cargo build -p k8sboard --features hotpath-profiling-alloc

HOTPATH_METRICS_SERVER_OFF=1 HOTPATH_SHUTDOWN_MS=45000 \
HOTPATH_OUTPUT_PATH=report.txt HOTPATH_ALLOC_METRIC=bytes \
  k8sboard --kubeconfig <path> --context <name> --screen pods
```

- The report is written when `run()` returns (window closed), or after `HOTPATH_SHUTDOWN_MS`, which then exits the process. A killed process writes nothing.
- Controls: `HOTPATH_OUTPUT_PATH`, `HOTPATH_OUTPUT_FORMAT` (`table` / `json` / `json-pretty`), `HOTPATH_ALLOC_METRIC` (`bytes` / `count`), `HOTPATH_ALLOC_CUMULATIVE=1` (include nested calls), `HOTPATH_LIMIT`, `HOTPATH_FOCUS`.
- hotpath serves live metrics on port 6770 unless `HOTPATH_METRICS_SERVER_OFF=1`; its thread sampler costs about 10 % of a core. Subtract both from CPU readings.
- The report holds function names, counts, byte sizes, thread names, rustc and OS versions. It never holds kubeconfig data, object contents or environment variables.
- The alloc build prints `[heap] live=… peak=…` to stderr every 5 s: the live Rust heap, which hotpath's per-call bytes cannot tell you. Its own histograms add about 5 MiB to that number.

## What is instrumented

Sites sit at batch granularity (a frame, a snapshot, a layout), never per cell or per byte.

| Area | Sites |
|---|---|
| Startup | `k8sboard::run` (scope), `AppShell::new` |
| Watch to rows | `Batcher::handle_event`, `Batcher::flush_snapshot`, `Merger::flush_if_ready`, `pod::pod_summary` (cluster); `LiveList::apply`, `resource_kind::rows` |
| Tables | `PodTableDelegate::rebuild_view`, `KindTableDelegate::rebuild_view` |
| Render | `AppShell::render`, `render_body`, `render_drawer` (the drawer), `Dock::render`, `LogTab::render` |
| Topology | `build_topology`, `layout`, `route_edges`, `TopologyView::render`, `render_canvas` |
| Logs | `LogBuffer::push` |
| Palette | `palette_entries` |
| Metrics and issues | `PodUsageHistory::record`, `KubeletHistory::record`, `IssueBoard::refresh` |

Adding a site: put the `cfg_attr` line on a function (`hotpath::measure(impl_type = "Type")` inside an `impl`).

## How to read a report

- **timing**: `Total` and `% Total` find where wall time goes; `P95`/`P99` find spikes.
- **alloc-bytes**: bytes allocated by the function itself, not what stays alive. A large `Avg` with a small live heap is churn (frames, snapshots), not retention.
- **threads**: per-thread allocate / free; a growing `Diff` on one thread points at retained data.
- Retained memory: compare `[heap] live` between screens and against a launch that never connects (`--screen overview` with no kubeconfig).

## Tests

`live_heap` has a unit test for the live and peak counters (`cargo test -p k8sboard --features hotpath-profiling-alloc live_heap`). The default gate never compiles `hotpath`.
