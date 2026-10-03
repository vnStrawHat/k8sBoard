# Inventory — app frame, dock, palette, Settings, guardrails

[Back to index](README.md). Refs: anatomy section, W1, W2, W8, W8b, W9, keyboard map, tokens. "Code" names `crates/app/src` modules.

## Title bar

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| T1 | Custom GPUI title bar, logo, window controls | Done | 0003 `title_bar.rs` | — |
| T2 | Cluster switcher (every context of every loaded kubeconfig, env badge) | Done | 0003, 0024, 0026 (popover with filter, env groups, health, Retry, Ctrl Shift C, Ctrl 1–9); several live clusters 0027 | — |
| T3 | Multi-select clusters, "View N clusters", `prod-eu-1 +1` label | Done | 0027 `cluster_switcher.rs` (ticks, Space, footer, Enter), `cluster_view.rs`, `title_bar.rs` | — |
| T4 | Environment badge and env-colored top border (riskiest env) | Done | 0024 `environment.rs`, `title_bar.rs`; riskiest of several clusters 0027 | — |
| T5 | Namespace picker (wireframe shows several namespaces: `ns: payments, web`) | Partial | 0003 (one or all) | multi-namespace → 0009 |
| T6 | Search box "Search resources or run a command… Ctrl K" | Done | 0029 (`title_bar.rs` middle slot; click opens the palette) | — |
| T7 | Read-only lock badge | Done | 0030 `title_bar.rs` (per-session lock, dashed env border, Ctrl Shift R on the cursor cluster) | — |
| T8 | Issues button `⚑ 4` | Done | 0020 | — |
| T9 | Settings button ⚙ | Done | 0025 (opens the Settings window, Ctrl ,) | — |
| T10 | "Manage clusters…" item | Done | 0025 | — |

## Navigation sidebar

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| N1 | Groups (Cluster … Custom Resources), collapse, active group open | Done | 0003, 0005 `navigation.rs` | — |
| N2 | Items disabled with "Not permitted" from SSAR | Done | 0005 | extend per new kind |
| N3 | Counts per item | Done (0012) | live counts for Pods, Nodes, the visible kind; one-shot counts for the other kinds | — |
| N4 | Error counts (red) | Done | 0020 | — |
| N5 | Top items Overview, Issues, Topology | Done (Issues in 0020, Overview in 0021 and the default landing, Topology in 0022) | 0020, 0021, 0022 | — |
| N6 | Custom Resources group auto-filled from CRDs (e.g. Certificates) | Done (grouped by API group) | — | 0018 |
| N7 | Clicking a nav item un-zooms the dock | Done | 0004 | — |

## Workspace header and tables

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| H1 | Title, kind icon, count | Done | 0003, 0005 | "38 of 1,284 match" → 0009 |
| H2 | Filter chips (Namespace, Status, label query, "+ Filter") and `/` filter | Missing | — | 0009 |
| H3 | Columns ▾ (toggle columns), sort | Done | 0009 `table_view.rs`; saved per screen by 0024 | — |
| H4 | Summary chips as filters (Nodes: Ready, NotReady, Cordoned, version skew) | Missing | — | 0009 |
| H5 | List-level buttons (Scale, Trigger now, Reveal all, Hide inactive, Hide system, …) | Missing | — | read-only ones 0009/0015/0016; mutating 0032 |
| H6 | Row checkboxes, multi-select, floating selection bar | Missing | — | selection 0009; bulk actions 0032–0034 |
| H7 | Virtualized table, themed status tones, muted namespace prefix | Done | 0003, 0005 | — |
| H8 | Cluster column in multi-cluster mode | Done (Pods, Nodes, every kind screen) | 0027 `cluster_rows.rs`, `table_layout.rs` | Issues has none: it draws the primary cluster |
| H9 | Row density 28 / 36 px | Missing | — | 0025 (Appearance) |

## Drawer frame

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| D1 | Overlay drawer, table keeps width, shadow | Done | 0003 | — |
| D2 | Header ⋯ ⤢ ✕; ⋯ equals the row context menu | Done | 0003, 0005, 0010 | — |
| D3 | Tab bar Overview / Monitor / YAML / Events | Done | Overview, Containers (pods), Monitor (0010), YAML, Events | — |
| D4 | WHY / alert box per kind | Missing | — | pods 0008; kinds 0012–0018 |
| D5 | `→` links to related objects (node, owner, target) | Partial | related pods 0005 | 0008, 0012 |
| D6 | ↑↓ moves rows while open, Esc or table click closes | Done (0028): Esc closes; a row click switches the subject (decision 14) | `keyboard_navigation.rs` | — |

## Dock (W8, W8b)

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| K1 | Log tabs, resize ≤ 60 %, zoom, minimize, close | Done | 0004 | — |
| K2 | Toolbar: filter, Follow, Previous, Wrap, Timestamps, Reconnect | Done | 0004 | — |
| K3 | Regex, level toggles, JSON, density histogram, Export, Pop out | Partial | 0019 (regex, level chips, JSON, histogram, Export via the save dialog) | Pop out (decision 25) |
| K4 | Workload log tabs (`deploy/…`), pod colors, container picker chips | Done | 0019 | — |
| K5 | "+ ▾" new tab, drag to reorder, dashed max line, double-click reset, remembered height | Partial | 0019 ("+ ▾", drag to reorder) | dashed max line and double-click reset (0004 non-goals); height persistence: reserved key `dock.height` (0019 or 0025) |
| K6 | Shell tabs (W8b shell pane) | Missing | — | 0036 |
| K7 | Drain progress tab (W6 note 5) | Missing | — | 0034 |

## Status bar

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| B1 | Watch state, identity, app version | Done | 0003 `status_bar.rs` | — |
| B2 | API latency ("API 38 ms") | Done | 0026 | — |
| B3 | "⇄ N port-forwards", click opens the page | Missing | — | 0035 |

## Palette, keyboard, Settings, multi-cluster, guardrails, tokens

| ID | Item | Status | Gap → spec |
|---|---|---|---|
| P1 | Command palette (W9): prefixes `: @ # >`, fuzzy, live status, scope chips, footer | Done (0029: own scorer, loaded lists only, row actions on the cursor row, `@` single switch, Tab moves the cursor only; no action × search-hit pairs, no Ctrl ⏎ (0032), no carried namespace, no match highlighting) | `command_palette.rs`, `palette_search.rs`, `fuzzy_score.rs` |
| P2 | Keyboard map (22 bindings) and `?` cheat sheet | Done (0028; `:` and Ctrl K bound by 0029), mutating letters stay gated until 0031–0036 | `keymap.rs`, `shortcut_sheet.rs` |
| S1 | Settings window (W2) as a separate OS window, single instance | Done | 0025 |
| S2 | Clusters page: env groups, drag order, form, Test connection, Remove | Partial (no drag order) | 0025 |
| S3 | Add cluster: import file, watch folder, paste YAML | Partial (no watch folder) | 0025 |
| S4 | Add cluster: scan AWS EKS, GKE, AKS | Missing | backlog |
| S5 | Pages General, Appearance, Keyboard Shortcuts, Safety, Terminal & Shell, Logs, Metrics, Extensions, About | Partial (Clusters, Appearance, About; Keyboard Shortcuts done in 0028; Safety tier table and audit path in 0030 step 2a/3) | 0025, 0028, 0030; the rest: 0036, 0019, backlog |
| M1 | Multi-cluster aggregated tables | Partial (Pods, Nodes, every kind screen) | 0027; Overview, Issues, Topology draw the primary cluster only until 0020–0022 follow "Screens that land later" |
| G1 | Env tiers: prod typed name; staging, dev, local a confirm dialog with a click (user 2026-10-02; W10 text superseded) | Done | 0030 `write_guard.rs`, `confirm_dialog.rs` |
| G2 | Prod opens read-only; lock toggle; diff + dry-run before writes; audit log | Partial (done in 0030: PROD opens read-only, lock toggle, server dry-run before every write, audit log; the diff comes in 0031) | 0030, 0031 |
| G3 | Tokens: status tones OK/WARN/BAD/INFO/DONE | Done | 0003 `status_tone.rs` |
| G4 | Env tokens PROD/STG/DEV/LOCAL as theme colors | Done | 0024 `environment_color` |
