# 0054 — Status bar: separators, watched-kinds tooltip, app CPU / memory / network

Status: **implemented 2026-10-05**, requested by the user. Read-only: no new Kubernetes call. Crates: `crates/cluster` (traffic counter), `crates/app` (status bar, sampler).

## Goal

- Status bar items are separated by a vertical rule.
- Hovering `Watching N resource types` lists the watched kinds.
- The right corner shows `↓ in  ↑ out | CPU x%  MEM y` in place of the k8sBoard version (the version stays in Settings › About). Hovering CPU/MEM shows a detailed table, as OneTerm does.

## What OneTerm does (reference `crates/workspace/src/widgets/{resource,net_speed,status_text}.rs`)

- **Resource** item: `CPU 12.3%  MEM 45.2 MB` of its **own process**, `sysinfo`, every 2 s. The `sysinfo` refresh runs on the background executor; the UI tick only reads a mutex slot. CPU is divided by the core count (Task Manager style). The tooltip is a two-column table (Memory, CPU sections) in a nested entity that re-renders on every sample, because the kit builds a tooltip closure once.
- **Network** item: `↓ rx  ↑ tx` in bits/s, delta of the byte counters of its **own SSH session**, every 1 s; first sample 0, a counter that drops reads 0.

k8sBoard mirrors the layout, the pure delta helpers, the background sampling and the live tooltip entity. It differs in: 1 s interval for both items, bytes/s (`KB/s`) like the Topology Traffic mode, and its own traffic being the Kubernetes API client.

## What is measured

- **CPU**: share of the whole machine used by this process (`sysinfo` per-core value / logical cores, clamped 0-100).
- **MEM**: working set on Windows, resident set elsewhere (`sysinfo` `memory()`). OneTerm prefers the Windows private working set, which `sysinfo` does not give; reading it needs FFI (`unsafe_code` is denied), so the working set stands in.
- **Network In/Out**: bytes/s the client received from and sent to the Kubernetes API server. No portable per-process network counter exists, so `crates/cluster/src/traffic.rs` counts at the one place the client is built (`ClusterConnection::open`): a tower layer around the whole HTTP stack, after decompression. It counts request/response heads (method, URI and header lengths), a request body of known size, and every response body data frame, so long watches count as they stream. Not counted: websocket payloads (exec, port-forward) and TLS/TCP framing. `ClusterConnection::traffic()` returns the shared `TrafficCounter`. No header values or bodies are read, logged or kept, only lengths.

## Sampling

`ProcessUsage` (`process_usage.rs`) is an entity owned by `AppShell` and rendered by `StatusBar::right`. One `cx.spawn` loop: sample on `background_executor()`, then update the entity (read the open cluster's counters, record, one `cx.notify()`), then wait 1 s. The first sample runs at once. The loop is held as a `Task` in the entity, so closing the window cancels it. State transitions are pure: `UsageState::record` keeps the previous totals and instant; no cluster, or a switch to a connection with lower totals, reads 0.

## Tooltips

- **Watching**: `Watched resources`, one row per kind sorted by name, `×N` when a kind has several watches (one per namespace). Source: `LiveCluster::watched_kinds()`, built from the same fields as `watch_count()`; `IssueFeeds::watch_count` is now the sum of `IssueFeeds::watched`. The rows sum to the count in `Watching N`.
- **Network**: `Kubernetes API traffic` (Received/Sent rate) and `Since the cluster connected` (totals, `—` without a cluster).
- **CPU / MEM**: `Memory` (working set or RSS, commit or virtual size), `CPU` (usage of N logical cores, threads (`n/a` off Linux), uptime). Names follow the OS as OneTerm's do.

## Dependencies

- `sysinfo` 0.31.4 (`default-features = false`, `system`): already in `Cargo.lock` through gpui.
- `crates/cluster`: `tower` (`util`) is no longer optional, and `http-body` 1 is added; both were already in `Cargo.lock`. No new package.

## Tests

- `traffic.rs`: shared counters, head and request byte math, a client over a fake service counts what it sends and reads.
- `process_usage_tests.rs`: rate math (delta, counter drop, zero elapsed), CPU normalisation, state transitions (first sample, second sample, cluster closed), `format_memory`, `format_uptime`, item texts, table rows, the sampler reads this process.
- `watched_kinds.rs`: merge, sort, `×N`. `status_bar.rs`: tooltip sections.

## Screenshots

`--screenshot` builds accept the dev-only environment variable `K8SBOARD_SCREENSHOT_HOVER=x,y` (window pixels): the pointer rests there before the capture, so a tooltip shows (status bar at y 887: Watching x 90, network x 1100, CPU/MEM x 1250). It does nothing without the variable and is compiled out of normal builds.
