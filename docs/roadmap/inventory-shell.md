# Inventory — app frame, dock, palette, Settings, guardrails

[Back to index](README.md). Refs: anatomy section, W1, W2, W8, W8b, W9, keyboard map, tokens. "Code" names `crates/app/src` modules.

## Title bar

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| T1 | Custom GPUI title bar, logo, window controls | Done | 0003 `title_bar.rs` | — |
| T2 | Cluster switcher (every context of every loaded kubeconfig, env badge) | Done | 0003, 0024, 0026 (popover with filter, env groups, health, Retry, Ctrl Shift C, Ctrl 1–9); one cluster at a time (0046) | — |
| T3 | Multi-select clusters, "View N clusters", `prod-eu-1 +1` label | Removed by user decision 2026-10-03 (single cluster only) | built in 0027; removal 0046 | — |
| T4 | Environment badge and env-colored top border | Done | 0024 `environment.rs`, `title_bar.rs`; the riskiest-of-several rule (0027) goes with 0046 | — |
| T5 | Namespace picker (wireframe shows several namespaces: `ns: payments, web`) | Done | 0003, 0009 (`NamespaceScope::Several`, `title_bar.rs` `ns: a, b`) | — |
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
| H1 | Title, kind icon, count | Done | 0003, 0005, 0009 ("N of M match", `workspace.rs`) | — |
| H2 | Filter chips (Namespace, Status, label query, "+ Filter") and `/` filter | Done | 0009 `filter_bar.rs`, `table_filter.rs` | — |
| H3 | Columns ▾ (toggle columns), sort | Done | 0009 `table_view.rs`; saved per screen by 0024 | — |
| H4 | Summary chips as filters (Nodes: Ready, NotReady, Cordoned, version skew) | Done | 0009 `filter_bar.rs`, `node_summary.rs` | — |
| H5 | List-level buttons (Scale, Trigger now, Reveal all, Hide inactive, Hide system, …) | Done (bulk ones in the selection bar; New missing, see the audit) | 0009, 0015, 0016, 0032, 0032b | read-only ones 0009/0015/0016; mutating 0032 (workload buttons Done) and 0032b (HPA Edit limits, PVC Expand, StorageClass Set default: Done) |
| H6 | Row checkboxes, multi-select, floating selection bar | Done | 0009 `row_selection.rs`, `table_selection.rs` | selection 0009; workload bulk actions Done (0032); HPA, PVC, and StorageClass bulk actions Done (0032b); bulk `Delete…` Done (0033); nodes Done (0034) |
| H7 | Virtualized table, themed status tones, muted namespace prefix | Done | 0003, 0005 | — |
| H8 | Cluster column in multi-cluster mode | Removed by user decision 2026-10-03 (single cluster only) | built in 0027; removal 0046 | the Port Forwarding page keeps its Cluster column (forwards survive a switch, 0035) |
| H9 | Row density 28 / 36 px | Missing | — | 0025 did not ship it; audit 0043 (Appearance) |

## Drawer frame

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| D1 | Overlay drawer, table keeps width, shadow | Done | 0003 | — |
| D2 | Header ⋯ ⤢ ✕; ⋯ equals the row context menu | Done | 0003, 0005, 0010 | — |
| D3 | Tab bar Overview / Monitor / YAML / Events | Done | Overview, Containers (pods), Monitor (0010), YAML, Events | — |
| D4 | WHY / alert box per kind | Done | pods 0008 `pod_diagnosis.rs`; kinds 0012–0018 `kind_diagnosis.rs` | — |
| D5 | `→` links to related objects (node, owner, target) | Done | related pods 0005; node and owner links 0008; Go to owner, target, claim, role 0012–0015 | — |
| D6 | ↑↓ moves rows while open, Esc or table click closes | Done (0028): Esc closes; a row click switches the subject (decision 14) | `keyboard_navigation.rs` | — |

## Dock (W8, W8b)

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| K1 | Log tabs, resize ≤ 60 %, zoom, minimize, close | Done | 0004 | — |
| K2 | Toolbar: filter, Follow, Previous, Wrap, Timestamps, Reconnect | Done | 0004 | — |
| K3 | Regex, level toggles, JSON, density histogram, Export, Pop out | Partial | 0019 (regex, level chips, JSON, histogram, Export via the save dialog) | Pop out (decision 25) |
| K4 | Workload log tabs (`deploy/…`), pod colors, container picker chips | Done | 0019 | — |
| K5 | "+ ▾" new tab, drag to reorder, dashed max line, double-click reset, remembered height | Partial | 0019 ("+ ▾", drag to reorder) | dashed max line and double-click reset (0004 non-goals); height persistence: reserved key `dock.height` (0019 or 0025) |
| K6 | Shell tabs (W8b shell pane) | Done | 0036 `shell_tab.rs`, `terminal_*.rs`, `shell_open.rs`; the allowed path awaits a write-capable cluster (risks R2) | — |
| K7 | Drain progress tab (W6 note 5) | Done | 0034 (`DockTab::Drain`, Cancel, Uncordon, Close) | — |

## Status bar

| ID | Item | Status | Covered by | Gap → spec |
|---|---|---|---|---|
| B1 | Watch state, identity, app version | Done | 0003 `status_bar.rs` | — |
| B2 | API latency ("API 38 ms") | Done | 0026 | — |
| B3 | "⇄ N port-forwards", click opens the page | Done | 0035 `status_bar.rs` | — |

## Palette, keyboard, Settings, multi-cluster, guardrails, tokens

| ID | Item | Status | Gap → spec |
|---|---|---|---|
| P1 | Command palette (W9): prefixes `: @ # >`, fuzzy, live status, scope chips, footer | Done (0029: own scorer, loaded lists only, row actions on the cursor row, `@` single switch, Tab moves the cursor only; Ctrl ⏎ Scale argument added by 0032; no action × search-hit pairs, no carried namespace, no match highlighting: audit, 0029 step 2) | `command_palette.rs`, `palette_search.rs`, `fuzzy_score.rs` |
| P2 | Keyboard map (22 bindings) and `?` cheat sheet | Done (0028; `:` and Ctrl K bound by 0029), mutating letters run since 0031–0037; the menu's A (Attach) is unbound (audit 0040) | `keymap.rs`, `shortcut_sheet.rs` |
| S1 | Settings window (W2) as a separate OS window, single instance | Done | 0025 |
| S2 | Clusters page: env groups, drag order, form, Test connection, Remove | Partial (no drag order) | 0025 |
| S3 | Add cluster: import file, watch folder, paste YAML | Partial (no watch folder) | 0025 |
| S4 | Add cluster: scan AWS EKS, GKE, AKS | Missing | backlog |
| S5 | Pages General, Appearance, Keyboard Shortcuts, Safety, Terminal & Shell, Logs, Metrics, Extensions, About | Partial (Clusters, Appearance, About; Keyboard Shortcuts done in 0028; Safety tier table and audit path in 0030 step 2a/3, the per-cluster Allow node shell toggle in 0037; About lists `oneterm-vt` in 0036) | 0025, 0028, 0030; the rest: Terminal & Shell (moved out of 0036), 0019, backlog |
| M1 | Multi-cluster aggregated tables | Removed by user decision 2026-10-03 (single cluster only) | 0027 built, 0046 removes; 0045 dropped |
| G1 | Env tiers: prod typed name; staging, dev, local a confirm dialog with a click (user 2026-10-02; W10 text superseded) | Done | 0030 `write_guard.rs`, `confirm_dialog.rs` |
| G2 | Prod opens read-only; lock toggle; diff + dry-run before writes; audit log | Done (0030: PROD opens read-only, lock toggle, server dry-run before every write, audit log; 0031: the diff of Edit YAML) | 0030, 0031 |
| G3 | Tokens: status tones OK/WARN/BAD/INFO/DONE | Done | 0003 `status_tone.rs` |
| G4 | Env tokens PROD/STG/DEV/LOCAL as theme colors | Done | 0024 `environment_color` |
