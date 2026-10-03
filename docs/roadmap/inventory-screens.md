# Inventory — screens W3, W4–W4c, W5–W6, W10, W11, Issues, Port Forwarding

[Back to index](README.md). Kind tables and drawers for the 29 kinds are in [inventory-kinds.md](inventory-kinds.md).

## Overview (W3) and Issues

| ID | Item | Status | Gap → spec |
|---|---|---|---|
| O1 | Header: cluster, version, region; "Last 15 min ▾"; Export report | Done (0021 steps 2 and 4) | 0021 |
| O2 | Needs attention: rule engine (pod and container states, Warning events, node conditions, cert expiry (data source: 0016 `watch_tls_secrets`), PDB blocks, stuck namespace), plain-language cause, one primary action per row | Done (engine in 0020, panel in 0021 step 3) | 0021 |
| O3 | Capacity: three-layer bars (used, requested, allocatable) for CPU, Memory, Pods, Volumes | Done (0021 step 1) | 0021 |
| O4 | Node heatmap, NotReady outlined, click opens Nodes with the node selected | Done (0021 step 1) | 0021 |
| O5 | Recent changes timeline (revisions, managedFields, events), click opens a diff | Partial (events and state rows, window 15 min or 1 h; managedFields and diff view → 0031) | 0021 |
| I1 | Issues screen (sidebar top item with a red count) | Done (0020) | — |

## Pods (W4, W4b, W4c)

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| W4-1 | Live table: Name, Status, Ready, Restarts, Node, Age | Done | 0003 | — |
| W4-2 | Memory column (W4) | Done | 0010 | — |
| W4-3 | Context menu = ⋯ menu, grouped, Delete last in red | Partial | View logs, Open shell (0036: gated on `get` and `create` on `pods/exec`, confirm dialog), Port-forward (0035: gated on `get` and `create` on `pods/portforward`, a submenu of the TCP ports), Copy name, Copy kubectl command (0008) | Edit YAML, Restart, Evict, Delete → 0031, 0033; Attach: a later item |
| W4-4 | Container submenu (MAIN/SIDECAR) for Logs, Shell, Port-forward; "Debug container…" | Partial | 0004 picks the container inside the log tab; 0019 adds the Logs sub-tab and the workload container picker; 0036 adds the Open shell submenu (running containers, init left out) | Logs and Port-forward submenus (0019 decision 30 skipped them); Debug container → 0037 |
| W4-5 | Overview tab: Node, Pod IP, QoS, Controlled by, Conditions, container summary | Done | 0003, 0008 | — |
| W4-6 | WHY box tied to a container with "Open container …" link | Done | 0008 | — |
| W4-7 | Containers tab master-detail, lifecycle groups, expand ⤢ | Done (`[` `]` switching in 0028) | 0003, 0028 | — |
| W4-8 | Container detail: State, Last state, Restarts, Image | Done | 0003, 0008 | — |
| W4-9 | Container sub-tabs Info / Env / Mounts / Logs / Monitor | Done | 0008 (Info, Env, Mounts), 0010 (Monitor), 0019 (Logs) | — |
| W4-10 | Ports with Forward button / live "● localhost:19090 · Stop" | Done | 0035 `drawer::port_row`, `port_forward_menu.rs` (Offer, Live, Disabled: UDP, no selector, the gate of the drawer subject's cluster); the allowed path awaits a write-capable cluster (risks R2) | — |
| W4-11 | Resource bars (usage vs limit, request tick, red near limit) | Done | 0008, 0010 | — |
| W4-12 | Probes with current result | Done | 0008 | — |
| W4-13 | Env and mounts summary (sources: ConfigMap, Secret) | Done | 0008 (names and sources only, decision C1) | — |
| W4c-1 | Monitor tab: CPU, Memory with request/limit lines, OOM markers | Done | 0010 | OOM marks from container status |
| W4c-2 | Monitor: Network, Disk I/O (kubelet cAdvisor via API proxy) | Done | 0011 | Network (receive/transmit) and Disk I/O (read/write) from the kubelet through the node proxy |
| W4c-3 | Range 15m/1h/6h/24h, scope pod/container, Table view, source note | Partial | 0010 (15m to 24h, scope, Table view, source note) | Prometheus ranges: backlog |

## Nodes (W5) and Drain (W6)

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| W5-1 | Table: Name, Status (Ready · SchedulingDisabled), Roles, Taints (+N, tooltip), Version, Internal IP, Age | Done | 0003 | — |
| W5-2 | CPU and Memory bar columns | Done | 0010 | — |
| W5-3 | Summary chips as filters, version-skew highlight, Columns ▾ | Missing | — | 0009 |
| W5-4 | Node drawer: conditions, allocatable used, system info | Done | 0003, 0008, 0010 (allocatable used) | — |
| W5-5 | Menu: node shell, Cordon, Drain…, Edit taints/labels, View pods on node, View YAML, Copy name | Partial | shell/cordon/drain disabled; Copy name | View pods on node → 0009; View YAML done (0007); rest → 0034, 0037 |
| W5-6 | Multi-select + selection bar (Cordon, Uncordon, Drain…) | Missing | — | 0009 (select), 0034 (actions) |
| W5-7 | Dock tabs "node shell (debug pod)" and "logs · kubelet" | Missing | — | 0037; kubelet logs deferred (0019 decision 24) |
| W6-1 | Drain dialog: 3 steps, kubectl-flag options with consequences, grace, timeout | Missing | — | 0034 |
| W6-2 | Per-pod eviction preview from PDBs (blocked first) | Missing | — | 0034 (preview logic is read-only, built on 0013) |
| W6-3 | Typed node-name confirm, "Cordon only", progress in dock, cancel | Missing | — | 0030, 0034 |

## Edit YAML (W10)

| ID | Item | Status | Gap → spec |
|---|---|---|---|
| W10-1 | Read-only YAML view (managedFields hidden, status kept) | Done (0007) | — |
| W10-2 | Editor (GPUI Kit Code Editor, Tree-sitter, LSP with cluster schema) | Done without LSP (0031; the server dry-run validates, decision C6) | — |
| W10-3 | Diff vs cluster (default), semantic change list, checks (dry-run, quota, rollout impact) | Partial (0031: dry-run, rollout, stale last-applied, moved placeholders, leading zeros; no quota check) | quota check: a later spec |
| W10-4 | Revision history tab | Missing | read-only list 0012; diff/restore: a later spec (0031 non-goal) |
| W10-5 | Env-tier confirm dialog, audit-log note, snapshot for one-step rollback | Partial (confirm dialog and audit note: 0030, used by Apply in 0031; snapshot: a later spec, C1 first) | 0030, 0031 |

## Topology (W11)

| ID | Item | Status | Gap → spec |
|---|---|---|---|
| W11-1 | Resources mode graph: ownerRef, selector, mounts, ingress → service | Done (0022; optional refs are flagged as missing) | 0022 |
| W11-2 | Config checks: Service with no pods, Ingress to a missing Service, unbound PVC, missing Secret | Done (0022; optional refs are flagged as missing) | 0022 (shares rules with 0020) |
| W11-3 | Layered layout, canvas, zoom/pan, remembered positions, minimap, Fit | Done (0022; positions are kept in memory only; visuals 0022b) | 0022, 0022b |
| W11-4 | Group by, kind toggles, live node status, select opens the drawer | Done (0022) | 0022 |
| W11-5 | Export PNG | Done (0022; a .svg path gets the SVG; visuals 0022b) | 0022 (decision C9), 0022b |
| W11-6 | Traffic mode (service mesh or eBPF) | Missing | backlog |

## Port Forwarding page (W7 "Port Forwarding")

| ID | Item | Status | Gap → spec |
|---|---|---|---|
| PF-1 | Local list across clusters: Target, Ports, Status, Cluster, Uptime, Stop/Retry/Start | Done | 0035 `port_forward_page.rs`, `port_forwards.rs` |
| PF-2 | Drawer: forward details, traffic counters, recent events; presets; auto-reconnect | Done | 0035 (reconnect in `port_forward.rs`; the allowed path awaits a write-capable cluster, risks R2) |
