# 0003 · Shell layout: title bar, navigation, status bar, states

[Back to index](README.md) · Modules: `app_shell.rs`, `title_bar.rs`, `navigation.rs`, `status_bar.rs`

## Regions (anatomy)

```text
┌ TitleBar ───────────────────────────────────────────────────────────────┐
│ k8sBoard  [● readonly@Monitor ▾]  [ns: all ▾]          [🔒 Read-only] [⚙] │
├ Sidebar ─┬ Workspace (relative) ─────────────────────────────────────────┤
│ groups   │ header: "Pods"  "104 pods · all namespaces"                    │
│          │ interruption banner (optional)                                 │
│          │ DataTable (fills)                    ┌ Drawer (absolute, right)┐│
│          │                                      └─────────────────────────┘│
├──────────┴──────────────────────────────────────────────────────────────── ┤
│ StatusBar: ● Watching 3 resource types · API v1.29.5 · user: readonly · k8sBoard 0.1.0 │
└──────────────────────────────────────────────────────────────────────────┘
```

- Root: `v_flex().size_full()` holding `TitleBar`, an `h_flex` (sidebar plus workspace), and `StatusBar`. There is no dock region (non-goal). Keep the workspace a single flex child so a dock can be added below it later.
- The drawer is a child of the workspace, so it never covers the title bar, the sidebar, or the status bar ([drawer.md](drawer.md)).

## Title bar (`gpui_kit::component::TitleBar`)

| Item | Component | Behavior |
|---|---|---|
| Logo text "k8sBoard" | `div` | |
| Cluster switcher | `Button` (ghost) + `DropdownMenu` | label = active context, or "No cluster". One `menu_with_check` item per kubeconfig context (checked = active), then a separator, then "Manage clusters…" **disabled** (reason: "Settings window comes later") |
| Namespace picker | `Button` + `DropdownMenu`, `.scrollable(true).max_h(px(360.))` | label `ns: all` or `ns: <name>`. Items: "All namespaces", then a separator, then the namespaces from `LiveList`. While loading, one disabled "Loading namespaces…". On failure: "All namespaces", a disabled error line, and the context default namespace. Selecting calls `session.set_scope` |
| Read-only lock | `Tag`/badge with a lock icon if `IconName` has one, else text | always shown: the app is read-only in this phase. Not clickable |
| Settings | icon `Button`, disabled, tooltip "Settings window comes later" | |

- Clicking a context switches immediately (single cluster).
- W1 multi-select is designed, not built. A future `ClusterSelection { primary, extra: Vec<String> }` replaces the single `session`; rows become `(context, summary)` and the Cluster column appears at 2+ clusters. Nothing in this spec blocks that change.
- There are no env colors and no title bar border color (the title bar border was removed on user request on 2026-10-04). They need Settings (W2) metadata.

## Navigation (`Sidebar` + `SidebarGroup` + `SidebarMenu`)

Taken verbatim from the wireframe navigation model:

| Section | Items (enabled in **bold**) | Open by default |
|---|---|---|
| top | Overview, Issues, Topology | n/a |
| Cluster | **Nodes**, Namespaces, Events | yes |
| Workloads | **Pods**, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs | yes |
| Network | Services, Ingresses, NetworkPolicies, Port Forwarding | no |
| Config | ConfigMaps, Secrets, HPAs, ResourceQuotas, PDBs | no |
| Storage | PVCs, PVs, StorageClasses | no |
| Access Control | ServiceAccounts, Roles, ClusterRoles, RoleBindings, ClusterRoleBindings | no |
| Helm | Releases | no |
| Custom Resources | CRDs | no |

- Disabled items use `.disable(true)`. Only Pods and Nodes show a count suffix (snapshot length, muted text), and none while loading.
- `.active(true)` marks the current screen. Clicking Pods or Nodes sets `screen` and closes the drawer.
- Item names are a `const` table in `navigation.rs` (data, not code per item).

## Workspace header

- Title (the kind name), then a muted count: `104 pods · all namespaces` / `12 pods · payments` / `4 nodes`.
- Below it, the interruption banner. When the list is `Ready` with `interruption: Some(msg)`, show one line in `theme.warning` text: "Live updates interrupted: {msg}. Retrying…".

## Status bar (`StatusBar::new().left(..).right(..)`)

| Slot | Content |
|---|---|
| left 1 | watch state: "Connecting to {context}…" (muted) / "Watching 3 resource types" (dot `success`) / "Live updates interrupted" (dot `warning`, if any list has an interruption or is `Failed`) / "Disconnected" (dot `danger`, when the session failed) |
| left 2 | `API {git_version}` when live |
| left 3 | `user: {kubeconfig user entry}` when known |
| right | `k8sBoard {CARGO_PKG_VERSION}` |

## Workspace states (priority order)

| Condition | Workspace shows |
|---|---|
| kubeconfig `Loading` / session `Connecting` | centered spinner + "Loading kubeconfig…" / "Connecting to {context}…" |
| kubeconfig `Failed`, no context, or session `Failed` | `Alert` (error variant) with the message. Session failure adds a "Retry" button; context errors add "Pick a context from the cluster menu." |
| list `Loading` | the table's built-in loading skeleton (`TableDelegate::loading` returns true) |
| list `Failed` | `Alert` with the message + "Retrying automatically." |
| list `Ready`, empty | `render_empty`: "No pods in {scope}" / "No nodes" |
| list `Ready` | the table (with the banner if interrupted) |
