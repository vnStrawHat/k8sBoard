# k8sBoard roadmap — everything in the wireframes

Baseline: HEAD `e80ed80` (specs 0001–0005 implemented; 0006 Events in design). Source of truth: `docs/k8sboard-wireframes.html` v0.6 (W1–W11, keyboard map, tokens, stack table).

## How to use

- The **inventory** files list every wireframe item with a status and the spec that covers it. When a spec merges, flip its rows to Done or Partial and update the table below.
- The **gap plan** files define the future specs 0007–0038 in delivery order. The architect turns each one into a `docs/specs/NNNN-title/` folder.
- Settle the [cross-cutting decisions](cross-cutting.md) before the spec that needs them (each row names its first consumer).

Status words: **Done** (matches the wireframe), **Partial** (built, but parts are missing), **Planned(0006)**, **Missing**.

## Files

| File | Contents |
|---|---|
| [inventory-shell.md](inventory-shell.md) | title bar, sidebar, workspace header, tables, drawer frame, dock, status bar, palette, keyboard, Settings, multi-cluster, guardrails, tokens |
| [inventory-screens.md](inventory-screens.md) | Overview (W3), Issues, Topology (W11), Pods (W4–W4c), Nodes and Drain (W5–W6), Edit YAML (W10), Port Forwarding page |
| [inventory-kinds.md](inventory-kinds.md) | the 29 W7 kinds: table, drawer, tabs, and actions per kind |
| [gap-plan-read-only.md](gap-plan-read-only.md) | specs 0007–0023: read-only features (UAT-verifiable) |
| [gap-plan-local-and-mutating.md](gap-plan-local-and-mutating.md) | specs 0024–0029 (local only), 0030–0038 (mutating), and the backlog |
| [cross-cutting.md](cross-cutting.md) | decisions to settle first, new dependencies, UAT data availability |
| [risks.md](risks.md) | top risks and mitigations |

## Status by area (at `e80ed80`)

| Area | Status | Covered by | Next spec |
|---|---|---|---|
| Shell frame, tables, overlay drawer, status bar | Partial | 0003 | 0009, 0028 |
| Pods (W4/W4b) | Partial | 0003, 0004, 0007, 0008 | 0010 |
| Nodes (W5) | Partial | 0003, 0008 | 0010, 0034 |
| Logs dock (W8/W8b) | Partial (workload tabs, filters, histogram, Export, "+ ▾", reorder done in 0019) | 0004, 0019 | Pop out, kubelet logs, Follow toggle, SYS marker lines |
| Kind Explorer (W7): 28 of 29 kinds live | Partial (drawer completions of the live kinds done in 0012; policy kinds done in 0013; storage kinds done in 0014; access-control kinds done in 0015; Secrets and Ingress TLS done in 0016; Helm Releases done in 0017; CRDs, custom resources, and stuck namespaces done in 0018) | 0003, 0005, 0012, 0013, 0014, 0015, 0016, 0017, 0018 | — |
| Events screen and drawer events | Planned(0006) | 0006 | — |
| Drawer YAML tab (W4c) | Done | 0007 | — |
| Drawer Monitor tab (W4c) | Done | 0010, 0011 | |
| RBAC and policy analysis: Who can…, Check permissions, Can do, Test traffic | Done | 0023 | — |
| Overview (W3), Issues, Topology (W11) | Partial (Issues engine and screen done in 0020) | 0020 | 0021, 0022 |
| Settings (W2), multi-cluster (W1), env colors | Missing | — | 0024–0027 |
| Keyboard map, command palette (W9) | Missing | — | 0028, 0029 |
| Every mutation: YAML edit (W10), drain (W6), shell, port-forward, delete | Missing | — | 0030–0037 (0038 Helm writes deferred) |

## Spec order (summary)

| Group | Specs | Gate |
|---|---|---|
| Read-only | 0007 YAML view · 0008 Pod/Node details · 0009 Table toolkit · 0010 Metrics I · 0011 Metrics II (kubelet) · 0012 Kind drawer completions · 0013 Policy kinds · 0014 Storage kinds · 0015 Access-control kinds · 0016 Secrets · 0017 Helm (read) · 0018 CRDs and custom resources · 0019 Workload logs · 0020 Issues (done) · 0021 Overview · 0022 Topology · 0023 RBAC and policy analysis | live-verifiable on UAT |
| Local only | 0024 Settings store and environments · 0025 Settings window · 0026 Cluster switcher · 0027 Multi-cluster views · 0028 Keyboard map · 0029 Command palette | no cluster writes; app writes its own config files |
| Mutating | 0030 Guardrails and write path · 0031 Edit YAML · 0032 Workload actions · 0033 Delete and pod lifecycle · 0034 Node maintenance · 0035 Port-forward · 0036 Terminal and pod shell · 0037 Node shell and debug containers · 0038 Helm write actions (deferred by the user, 2026-10-02; not scheduled) | C3 approved once for 0030–0037 (user, 2026-10-02); RBAC-gated (SSAR); disabled on UAT |

## Open items

1. No write-capable test cluster exists. Every mutating spec needs one for live checks ([risks.md](risks.md) R2).
2. The backlog (Prometheus, cloud scans, Traffic topology, plugins, AI, GitOps) needs a user decision on whether "everything" includes the Stack-section phase 3 items, which have no wireframe.
