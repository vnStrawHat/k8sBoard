# 0022 · Layout

[Back to index](README.md) · Step 1 (pins and App bands: step 2) · Module: `topology_layout.rs` (new) + `topology_layout_tests.rs`. Pure, in graph units (1 unit = 1 px at zoom 1). No dependency (decision 20).

## API

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GraphPoint { pub(crate) x: f32, pub(crate) y: f32 }
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GraphRect { pub(crate) origin: GraphPoint, pub(crate) width: f32, pub(crate) height: f32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement { Column(u8), ConfigRow }
pub(crate) struct Band { pub(crate) title: Option<SharedString> /* App */, pub(crate) rect: GraphRect }
pub(crate) struct TopologyLayout {
    pub(crate) rects: Vec<GraphRect>,                 // index = node index in the graph
    pub(crate) bands: Vec<Band>, pub(crate) extent: GraphRect,
    order: Vec<(usize /* band */, Slot, Vec<NodeId>)>, // the seed for the next layout
}
pub(crate) fn layout(graph: &TopologyGraph, group_by: GroupBy, pins: &HashMap<NodeId, GraphPoint>,
    previous: Option<&TopologyLayout>) -> TopologyLayout;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GraphStructure { pub(crate) nodes: Vec<NodeId>, pub(crate) edges: Vec<(NodeId, NodeId, Relation)> }
pub(crate) fn structure(graph: &TopologyGraph) -> GraphStructure;
```

## Placement (W11)

| Placement | Kinds | x |
|---|---|---|
| `Column(0)` | Ingress, HPA | `MARGIN` |
| `Column(1)` | Service, Deployment, StatefulSet, DaemonSet | `MARGIN + 1 × COLUMN_PITCH` |
| `Column(2)` | ReplicaSet | `+ 2 × COLUMN_PITCH` |
| `Column(3)` | Pod, pod group, `NoPods` ghost | `+ 3 × COLUMN_PITCH` |
| `ConfigRow` | ConfigMap, Secret, PVC (and their `Missing` ghosts) | under the band, see step 5 |

`Missing { kind }` takes `kind`'s placement. Empty columns keep their x, so the columns line up across bands.

## Algorithm

1. **Bands.** For `Components`, a band is an undirected connected component. Bands with two or more nodes are ordered by node count (descending), then by smallest `NodeId`. All single nodes share one last band. For `App`, a band is a group key ordered by name, with "Ungrouped" last.
2. **Initial order** per (band, slot) (a slot is a column or the config row):
   - `previous` is `None`: sort by `(kind, NodeId)`, then run the sweeps (step 3).
   - `previous` is `Some`: take the previous order of each slot, drop the ids that are gone, and **append new ids at the end** in `(kind, NodeId)` order. Skip the sweeps. Siblings never move when a pod is added (W11 pin 3).
3. **Barycenter sweeps** (`SWEEPS` = 4, alternating, only without `previous`). The down sweep visits columns 1→3. Each node's key is the mean index of its neighbours in columns with a **smaller index** (to its left). The up sweep visits columns 2→0 and uses neighbours in columns with a **larger index** (to its right). Config-row nodes are ordered by the mean x slot of their sources. A node without such neighbours keeps its own index as its key. The sort is stable, and ties fall back to `NodeId`.
4. **Columns.** Inside a band, the nodes of a column stack at `ROW_PITCH`. A shorter column is centered: `offset = (max − count) × ROW_PITCH / 2`.
5. **Config row** at `y = band column bottom + CONFIG_GAP`. Each config node wants the slot `x` of the column of its first source (in source order). If that slot is taken, it takes the next free slot to the right (slots step by `COLUMN_PITCH` and may go past column 3). A node without a source starts at slot 0. This follows W11 (ConfigMap under the Deployment, Secret one column right).
6. **Bands** stack top to bottom with `BAND_GAP`. App bands add `BAND_TITLE` above and a band rect padded by `BAND_PAD`.
7. **Pins**: a pinned id takes its pinned origin. Nothing moves out of its way.
8. **Extent** = the union of rects and bands, plus `MARGIN`.

`ponytail:` edges that span columns (Service → Pod over the ReplicaSet column) are not routed around nodes. Upgrade path: dummy nodes per spanned column (README open item 3).

## When to pass `previous` (decision 22)

| Change since the last layout | Call |
|---|---|
| `structure` and `group_by` equal | keep the layout; only tones and captions change |
| structure differs, same namespace and `group_by` | `layout(graph, group_by, pins, Some(&previous))` |
| first layout, a new namespace, a `group_by` change, or Reset positions | `layout(graph, group_by, pins, None)` + Fit |

The viewport only moves on Fit.

## Sizes (W11 measurements)

| Name | Value | Note |
|---|---|---|
| `NODE_WIDTH` × `NODE_HEIGHT` | 170 × 54 | every node |
| `COLUMN_PITCH` | 210 | W11 column pitch (gap 40) |
| `ROW_PITCH` | 70 | W11 pod pitch |
| `CONFIG_GAP` | 50 | W11 Deployment 283 → ConfigMap 390 |
| `BAND_GAP`, `BAND_PAD`, `BAND_TITLE` | 40, 12, 22 | |
| `MARGIN`, `SWEEPS` | 24, 4 | |

## Budget (AC 6)

`topology_budget` runs in the normal gate. It uses a realistic input: 40 Services, 3,000 pods, 100 ReplicaSets (and their Deployments), with grouping on. It times `build_topology` + `layout(.., None)` and asserts ≤ 80 ms (a debug build). Record the release-build time in [decisions.md](decisions.md) "Measurements" (target ≤ 8 ms). The sweeps are O(SWEEPS × E log V), and Service matching is O(S × P).
