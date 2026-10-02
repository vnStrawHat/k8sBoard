# Inventory — app frame, dock, palette, Settings, guardrails

[Back to index](README.md). Refs: anatomy section, W1, W2, W8, W8b, W9, keyboard map, tokens. "Code" names `crates/app/src` modules.

## Title bar

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| T1 | Custom GPUI title bar, logo, window controls | Done | 0003 `title_bar.rs` | — |
| T2 | Cluster switcher (single cluster, contexts of one kubeconfig) | Partial | 0003 | env groups, health, Ctrl 1–9, filter, Retry → 0026 |
| T3 | Multi-select clusters, "View N clusters", `prod-eu-1 +1` label | Missing | — | 0027 |
| T4 | Environment badge and env-colored top border (riskiest env) | Missing | — | 0024 (single), 0027 (multi) |
| T5 | Namespace picker (wireframe shows several namespaces: `ns: payments, web`) | Partial | 0003 (one or all) | multi-namespace → 0009 |
| T6 | Search box "Search resources or run a command… Ctrl K" | Missing | — | 0029 |
| T7 | Read-only lock badge | Partial | 0003 (static) | per-cluster toggle, Ctrl Shift R → 0030 |
| T8 | Issues button `⚑ 4` | Missing | — | 0020 |
| T9 | Settings button ⚙ | Partial | 0003 (no action) | 0025 |
| T10 | "Manage clusters…" item | Partial | 0003 (disabled) | 0025 |

## Navigation sidebar

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| N1 | Groups (Cluster … Custom Resources), collapse, active group open | Done | 0003, 0005 `navigation.rs` | — |
| N2 | Items disabled with "Not permitted" from SSAR | Done | 0005 | extend per new kind |
| N3 | Counts per item | Done (0012) | live counts for Pods, Nodes, the visible kind; one-shot counts for the other kinds | — |
| N4 | Error counts (red) | Missing | — | 0020 |
| N5 | Top items Overview, Issues, Topology | Missing (disabled) | — | 0021, 0020, 0022 |
| N6 | Custom Resources group auto-filled from CRDs (e.g. Certificates) | Missing | — | 0018 |
| N7 | Clicking a nav item un-zooms the dock | Done | 0004 | — |

## Workspace header and tables

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| H1 | Title, kind icon, count | Done | 0003, 0005 | "38 of 1,284 match" → 0009 |
| H2 | Filter chips (Namespace, Status, label query, "+ Filter") and `/` filter | Missing | — | 0009 |
| H3 | Columns ▾ (toggle columns), sort | Missing | — | 0009 |
| H4 | Summary chips as filters (Nodes: Ready, NotReady, Cordoned, version skew) | Missing | — | 0009 |
| H5 | List-level buttons (Scale, Trigger now, Reveal all, Hide inactive, Hide system, …) | Missing | — | read-only ones 0009/0015/0016; mutating 0032 |
| H6 | Row checkboxes, multi-select, floating selection bar | Missing | — | selection 0009; bulk actions 0032–0034 |
| H7 | Virtualized table, themed status tones, muted namespace prefix | Done | 0003, 0005 | — |
| H8 | Cluster column in multi-cluster mode | Missing | — | 0027 |
| H9 | Row density 28 / 36 px | Missing | — | 0025 (Appearance) |

## Drawer frame

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| D1 | Overlay drawer, table keeps width, shadow | Done | 0003 | — |
| D2 | Header ⋯ ⤢ ✕; ⋯ equals the row context menu | Done | 0003, 0005, 0010 | — |
| D3 | Tab bar Overview / Monitor / YAML / Events | Done | Overview, Containers (pods), Monitor (0010), YAML, Events | — |
| D4 | WHY / alert box per kind | Missing | — | pods 0008; kinds 0012–0018 |
| D5 | `→` links to related objects (node, owner, target) | Partial | related pods 0005 | 0008, 0012 |
| D6 | ↑↓ moves rows while open, Esc or table click closes | Partial | mouse only | 0028 |

## Dock (W8, W8b)

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| K1 | Log tabs, resize ≤ 60 %, zoom, minimize, close | Done | 0004 | — |
| K2 | Toolbar: filter, Follow, Previous, Wrap, Timestamps, Reconnect | Done | 0004 | — |
| K3 | Regex, level toggles, JSON, density histogram, Export, Pop out | Missing | — | 0019 |
| K4 | Workload log tabs (`deploy/…`), pod colors, container picker chips | Missing | — | 0019 |
| K5 | "+ ▾" new tab, drag to reorder, dashed max line, double-click reset, remembered height | Missing | — | 0019; height persistence 0024 |
| K6 | Shell tabs (W8b shell pane) | Missing | — | 0036 |
| K7 | Drain progress tab (W6 note 5) | Missing | — | 0034 |

## Status bar

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| B1 | Watch state, identity, app version | Done | 0003 `status_bar.rs` | — |
| B2 | API latency ("API 38 ms"; today shows the version) | Partial | 0003 | 0026 |
| B3 | "⇄ N port-forwards", click opens the page | Missing | — | 0035 |

## Palette, keyboard, Settings, multi-cluster, guardrails, tokens

| ID | Item | Status | Gap → spec |
|---|---|---|---|
| P1 | Command palette (W9): prefixes `: @ # >`, fuzzy, live status, scope chips, footer | Missing | 0029 |
| P2 | Keyboard map (22 bindings) and `?` cheat sheet | Missing | 0028 |
| S1 | Settings window (W2) as a separate OS window, single instance | Missing | 0025 |
| S2 | Clusters page: env groups, drag order, form, Test connection, Remove | Missing | 0024, 0025 |
| S3 | Add cluster: import file, watch folder, paste YAML | Missing | 0024, 0025 |
| S4 | Add cluster: scan AWS EKS, GKE, AKS | Missing | backlog |
| S5 | Pages General, Appearance, Keyboard Shortcuts, Safety, Terminal & Shell, Logs, Metrics, Extensions, About | Missing | 0025 (frame), owners: 0028, 0030, 0036, 0019, backlog |
| M1 | Multi-cluster aggregated tables | Missing | 0027 |
| G1 | Env tiers: prod typed name; staging, dev, local a confirm dialog with a click (user 2026-10-02; W10 text superseded) | Missing | 0030 |
| G2 | Prod opens read-only; lock toggle; diff + dry-run before writes; audit log | Missing | 0030, 0031 |
| G3 | Tokens: status tones OK/WARN/BAD/INFO/DONE | Done | 0003 `status_tone.rs` |
| G4 | Env tokens PROD/STG/DEV/LOCAL as theme colors | Missing | 0024 |
