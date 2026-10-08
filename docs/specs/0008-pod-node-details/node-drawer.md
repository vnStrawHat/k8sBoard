# 0008 · App: node drawer details

[Back to index](README.md) · Step 3 · Modules: `node_drawer.rs`, `related_pods.rs` (new, moved), `kind_drawer.rs`, `kind_row.rs`, `status_tone.rs`. Builds on 0007 (Node tabs Overview · Pods · Monitor · YAML · Events, expand toggle). No wireframe draws the node drawer; the layout follows the pod drawer and the W5 table columns.

## Overview tab (sections in this order)

| Section | Rows |
|---|---|
| Node | Status (toned), Scheduling, Roles ("—"), Taints (one per line, mono, as today), Created (timestamp and age) |
| System info | one `wide_detail_row(kind, address)` per address (mono, copyable; none when the node has no addresses), then OS `{operating_system}/{architecture} · {os_image}` (empty parts and their separators left out), Kernel, Container runtime, Kubelet (the existing `kubelet_version`) |
| Allocatable used | CPU, Memory, and Pods rows with usage bars (see Later changes) |
| Labels | `chips(&labels)` |
| Annotations | the annotation terms as chips like Labels (no fold) |
| Conditions | one row per `NodeCondition` in API order: name, then `{status}` in `node_condition_tone`, then muted `{reason} · since {age}`; tooltip on the row = `message` when Some |

The pods are not in the Overview: the Pods tab shows them (below). The raw capacity and allocatable quantities are not shown; the Allocatable used rows carry the human units.

- Labels and values use `wide_detail_row` ("Container runtime" does not fit 104 px).

```rust
/// Ready: True is Ok, False is Bad, Unknown is Warn. Every other type (pressure, NetworkUnavailable,
/// node-problem-detector conditions) reports a problem when True: True is Bad, False is Ok, Unknown is Warn.
pub(crate) fn node_condition_tone(condition: &NodeCondition) -> StatusTone; // status_tone.rs
pub(crate) fn condition_status_text(status: ConditionStatus) -> &'static str; // "True" | "False" | "Unknown"
```

## Pods tab (`related_pods.rs`)

- The node's own tab, titled `Pods {n}` (plain `Pods` without a live cluster); the body is `pods_section(&PodOwner::Node { name }, ..)` in a scrolled `DrawerBody::Scrolling`, empty without a live cluster.

- Move `pods_section`, `related_pod_row`, `PodRowDetail`, and `MAX_RELATED_PODS` from `kind_drawer.rs` to a new `related_pods.rs`; `pods_section` becomes `pub(crate)`. Behavior for kinds is unchanged.
- `kind_row::PodOwner` gains `Node { name: String }`; doc becomes "Which pods a drawer lists". `owns_pod` matches `pod.node_name == Some(name)` in any namespace.
- For `PodOwner::Node` the rows use a new `PodRowDetail::NamespaceAndStatus`: muted `{namespace}/` prefix before the name, then the status.
- When `live.scope` is `NamespaceScope::Named(ns)`, a muted note under the title: `Only pods in {ns} are listed` (the pods watch follows the namespace picker).
- Clicking a row reveals the pod (existing behavior).

"View pods on node" in the Node menu stays in 0009 (it needs the filter model); this section covers the need until then.

## Not in this step

CPU/Memory usage and allocated (requested) totals (0010, 0021), node shell, cordon, drain (0034, 0037), kubelet logs (0019), Edit taints/labels (mutating).

## Later changes

The Pods tab tags each pod `DS`, `emptyDir`, `PDB 0`, or `no controller` (see 0034 `drain-dialog.md`).

The CPU and Memory rows of "Allocatable used" read `3.3 used · 6.9 requested / 26 cores` when the pods of every namespace are known (bar tick = requests; the bar's tooltip names used, requested, and allocatable). The Nodes table has two opt-in columns, `CPU req` and `Mem req` (the pods' requests as a share of allocatable), hidden until ticked in the Columns menu; they read `—` under a namespace scope.

When the node allocates `ephemeral-storage` or `hugepages-*`, a row follows Pods for each (`Ephemeral storage`, then `Hugepages 2Mi` and so on). metrics-server has no usage for them, so the row is requests over allocatable: `1 requested / 50Gi` with a bar (no tick) when the pods are known, `— / 50Gi` without a bar under a namespace scope. A node without the resource gets no row.

The body of the node drawer scrolls with a handle (`DrawerState.scroll`), and every drawer built by `drawer_frame` tracks it. A drawer has the keyboard after Enter opened it (or a left press inside it): PageUp, PageDown, Home, and End then scroll its body (a page is 90 % of the view) instead of moving the table cursor, until the drawer closes or a table row is clicked. The header subtitle has a `Pods (N)` link (tooltip "Show the pods on this node") that switches to the Pods tab.

The Pods tab body starts with the section title and no top gap (`SectionPlace::First`); the Overview keeps its gap between sections.
