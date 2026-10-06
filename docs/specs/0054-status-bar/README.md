# 0054 — Status bar: separators, watched-kinds tooltip, app CPU / memory / network

Status: **implemented 2026-10-05**, requested by the user. Read-only: no new Kubernetes call. Crates: `crates/cluster` (traffic counter), `crates/app` (status bar, sampler).

## Goal

- Status bar items are separated by a vertical rule.
- Hovering `Watching N resource types` lists the watched kinds.
- The right corner shows `↓ in  ↑ out | App CPU x% · App MEM y` ("App" says the figures are the k8sBoard process, not the cluster) in place of the k8sBoard version (the version stays in Settings › About). Hovering CPU/MEM shows a detailed table, as OneTerm does.

## What OneTerm does (reference `crates/workspace/src/widgets/{resource,net_speed,status_text}.rs`)

- **Resource** item: `App CPU 12.3% · App MEM 45.2 MB` of its **own process**, `sysinfo`, every 2 s. The `sysinfo` refresh runs on the background executor; the UI tick only reads a mutex slot. CPU is divided by the core count (Task Manager style). The tooltip is a two-column table (Memory, CPU sections) in a nested entity that re-renders on every sample, because the kit builds a tooltip closure once.
- **Network** item: `↓ rx  ↑ tx` in bits/s, delta of the byte counters of its **own SSH session**, every 1 s; first sample 0, a counter that drops reads 0.

k8sBoard mirrors the layout, the pure delta helpers, the background sampling and the live tooltip entity. It differs in: 1 s interval for both items, bytes/s (`KB/s`) like the Topology Traffic mode, and its own traffic being the Kubernetes API client.

## What is measured

- **CPU**: share of the whole machine used by this process (`sysinfo` per-core value / logical cores, clamped 0-100).
- **MEM**: the **private working set** (what Task Manager's "Memory" column shows), as OneTerm does. `sysinfo` 0.31 does not give it, so `crates/app/src/process_memory.rs` reads `PROCESS_MEMORY_COUNTERS_EX2` with one `GetProcessMemoryInfo` call (`windows` crate, features `Win32_System_ProcessStatus` and `Win32_System_Threading`). That file holds the one `#[cfg(windows)]` FFI module with `#[allow(unsafe_code)]` and a `// SAFETY:` comment, beside the clipboard module of spec 0016; the user approved it on 2026-10-05 ("just follow OneTerm"). The choice (`displayed_memory`) is pure and cross-platform: the private working set when given and above 0, else the resident figure (the working set on Windows, RSS elsewhere, `sysinfo` `memory()`). An older Windows (before 10 22H2 / 11 22H2 with the September 2023 update) fills no private working set, so MEM reads the working set there.
- **Commit**: on Windows `PrivateUsage` from the same call, not `sysinfo` 0.31's `virtual_memory()`, which there is the whole address space (about 4 GB of reservation in every process). Elsewhere the virtual size.
- **Network In/Out**: bytes/s the client received from and sent to the Kubernetes API server. No portable per-process network counter exists, so `crates/cluster/src/traffic.rs` counts at the one place the client is built (`ClusterConnection::open`): a tower layer around the whole HTTP stack, after decompression. It counts request/response heads (method, URI and header lengths), a request body of known size, and every response body data frame, so long watches count as they stream. Not counted: websocket payloads (exec, port-forward) and TLS/TCP framing. `ClusterConnection::traffic()` returns the shared `TrafficCounter`. No header values or bodies are read, logged or kept, only lengths.

## Sampling

`ProcessUsage` (`process_usage.rs`) is an entity owned by `AppShell` and rendered by `StatusBar::right`. One `cx.spawn` loop: sample on `background_executor()`, then update the entity (read the open cluster's counters, record, one `cx.notify()`), then wait 1 s. The first sample runs at once. The loop is held as a `Task` in the entity, so closing the window cancels it. State transitions are pure: `UsageState::record` keeps the previous totals and instant; no cluster, or a switch to a connection with lower totals, reads 0.

## Tooltips

- **Watching**: `Watched resources`, one row per kind sorted by name, `×N` when a kind has several watches (one per namespace). Source: `LiveCluster::watched_kinds()`, built from the same fields as `watch_count()`; `IssueFeeds::watch_count` is now the sum of `IssueFeeds::watched`. The rows sum to the count in `Watching N`. UX batch 5c: past 20 kinds the rest fold into one `and more +N` row, and a note says the number follows the screens you have opened. The resource item reads `App CPU x% · App MEM y`.
- **Network**: `Kubernetes API traffic` (Received/Sent rate) and `Since the cluster connected` (totals, `—` without a cluster).
- **CPU / MEM**: `Memory` (Private working set, Working set or RSS, Commit (private bytes) or Virtual size, Peak working set; a figure the OS does not give is left out) and `CPU` (usage of N logical cores, threads (only where the OS lists them; the row is hidden otherwise), uptime). Names and order follow OneTerm. Not mirrored: its `CPU time (user + kernel)` row (`sysinfo` 0.31 has no accumulated CPU time) and its Windows thread count (a whole-system process snapshot every sample).

## Dependencies

- `sysinfo` 0.31.4 (`default-features = false`, `system`): already in `Cargo.lock` through gpui.
- `windows` 0.62 (Windows only, already a dependency): two more features, `Win32_System_ProcessStatus` and `Win32_System_Threading`.
- `crates/cluster`: `tower` (`util`) is no longer optional, and `http-body` 1 is added; both were already in `Cargo.lock`. No new package.

## Tests

- `traffic.rs`: shared counters, head and request byte math, a client over a fake service counts what it sends and reads.
- `process_usage_tests.rs`: rate math (delta, counter drop, zero elapsed), CPU normalisation, state transitions (first sample, second sample, cluster closed), `format_memory`, `format_uptime`, item texts, table rows, missing figures left out, `displayed_memory` and its fallback, the sampler reads this process. `process_memory.rs`: this process has a private working set on Windows.
- `watched_kinds.rs`: merge, sort, `×N`. `status_bar.rs`: tooltip sections.

## Screenshots

`--screenshot` builds accept the dev-only environment variable `K8SBOARD_SCREENSHOT_HOVER=x,y` (window pixels): the pointer rests there before the capture, so a tooltip shows (status bar at y 887: Watching x 90, network x 1100, CPU/MEM x 1250). It does nothing without the variable and is compiled out of normal builds.
