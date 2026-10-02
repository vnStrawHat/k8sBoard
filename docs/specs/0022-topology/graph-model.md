# 0022 · Graph model

[Back to index](README.md) · Step 1 (expand, filters, Group by: step 2) · Module: `topology_graph.rs` (new) + `topology_graph_tests.rs`. Pure: no GPUI context; `SharedString` and `StatusTone` only. No `tracing::`.

## Types

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TopologyKind { Ingress, HorizontalPodAutoscaler, Service, Deployment, StatefulSet, DaemonSet,
    ReplicaSet, Pod, ConfigMap, Secret, PersistentVolumeClaim }        // Ord = in-column sort order
impl TopologyKind {
    pub(crate) fn placement(self) -> Placement;        // layout.md
    pub(crate) fn badge(self) -> &'static str;          // ResourceKind::badge(); Pod "Po"
    pub(crate) fn object_kind(self) -> &'static str;    // "Deployment", for ResourceKey::of_object
    pub(crate) fn filter(self) -> KindFilter;           // chip group
    pub(crate) fn resource_kind(self) -> Option<ResourceKind>; // None for Pod
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum KindFilter { Ingress, Service, Workload, Config }

/// Stable identity across rebuilds: pins, expansion, layout seeding, and selection use it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum NodeId {
    Object { kind: TopologyKind, name: String },
    PodGroup { owner_kind: TopologyKind, owner: String },
    Missing { kind: TopologyKind, name: String },     // ghost: referenced, not listed
    NoPods { service: String },                        // ghost: W11 "0 pods match"
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeLook { Plain, Ghost, Unchecked }   // Unchecked: feed Loading/Off (decision 4)
pub(crate) struct TopologyNode {
    pub(crate) id: NodeId, pub(crate) kind: TopologyKind, pub(crate) look: NodeLook,
    pub(crate) name: SharedString,       // object name; group "48 pods"; NoPods "selector app=x"
    pub(crate) caption: SharedString,    // "Deployment · 2/3", "CrashLoopBackOff · api", "Secret · not checked"
    pub(crate) tone: Option<StatusTone>, // None = neutral
    pub(crate) group: Option<String>,    // app label
    pub(crate) objects: usize,           // 1; a group: its pods; ghosts and unchecked: 0
    pub(crate) key: Option<ResourceKey>, // drawer/reveal target; None for ghosts, groups, unchecked
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Relation { Owns, RoutesTo, Mounts }
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TopologyEdge { pub(crate) from: usize, pub(crate) to: usize, pub(crate) relation: Relation }
pub(crate) struct TopologyGraph { pub(crate) nodes: Vec<TopologyNode>, pub(crate) edges: Vec<TopologyEdge>,
    pub(crate) checks: Vec<ConfigCheck>, pub(crate) resources: usize /* Σ objects */ }
pub(crate) enum TopologyBuild { Graph(TopologyGraph), TooLarge(TooLarge) }
pub(crate) enum TooLarge { Objects(usize) /* > RAW_LIMIT */, Nodes(usize) /* > NODE_LIMIT */ }

pub(crate) struct TopologyInputs<'a> {
    pub(crate) namespace: &'a str,
    pub(crate) pods: Option<&'a [PodSummary]>,          // None until Ready; other namespaces are skipped
    pub(crate) rows: &'a [(TopologyKind, FeedRows<'a>)],
    pub(crate) nodes: &'a [NodeSummary],                 // kind_diagnosis input
    pub(crate) filter: &'a TopologyFilter, pub(crate) expanded: &'a BTreeSet<NodeId>, pub(crate) now: Timestamp,
}
pub(crate) enum FeedRows<'a> { Ready(&'a [KindRow]), Loading, Off }   // a chip-disabled kind is Off
pub(crate) struct TopologyFilter { pub(crate) kinds: BTreeSet<KindFilter>, pub(crate) problems_only: bool,
    pub(crate) group_by: Option<GroupBy> /* None = automatic, decision 27 */ }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GroupBy { App, Components }
pub(crate) fn build_topology(inputs: &TopologyInputs) -> TopologyBuild;
/// The filter choice, else App when a namespace pod has an app label, else Components (decision 27).
pub(crate) fn resolve_group_by(choice: Option<GroupBy>, namespace: &str, pods: &[PodSummary]) -> GroupBy;
```

Nodes are sorted by `NodeId`, and edges by `(from, to, relation)` with duplicates removed.

## Input cap and indexes (decision 14)

1. `raw = Σ Ready rows + pods in the namespace`. If `raw > RAW_LIMIT` (5,000), return `TooLarge::Objects(raw)` before anything else is built.
2. Build the indexes once: pods by controller `(kind, name)`, ReplicaSets by owner Deployment, and listed names per kind (`HashSet`). Every rule below is a lookup into them. Service matching is the only S × P scan (`Selector::matches`).

## Nodes

| Source | Node | Caption | Tone |
|---|---|---|---|
| feed row | `Object` | `{Kind} · {extra}`: Deployment/STS/DS `ready/desired`; RS `rev {n}`; HPA `{min}–{max}`; Service: the first routing ingress path (decision 19), else type; PVC: phase; Ingress: its first host | `row.status.tone` when Bad/Warn; raised by a WHY box |
| pod | `Object { Pod }` | not Ok: `{status} · {first not-ready container}`; Ok: `{status} · {ready}/{total}` | `pod_status_label(pod).tone` when Bad/Warn |
| pod group | `PodGroup` | `{running} running · {bad} failing` (the failing part is dropped at 0) | Warn if any member is Warn, else `None` |
| ghost | `Missing`, `NoPods` | `Missing {Kind}` / `0 pods match` | the check's tone |
| unverified target | `Object`, `look: Unchecked` | `{Kind} · not checked` | `None` |

`key` = `ResourceKey::of_pod` for pods, else `ResourceKey::of_object(kind.object_kind(), Some(ns), name)`.

## Edges

| From | To | Relation | Source field |
|---|---|---|---|
| Ingress | Service | RoutesTo | `rules[].service`, `default_service` |
| Ingress | Secret | Mounts | `tls[].secret_name` |
| Service | Pod / group | RoutesTo | `Selector::of_labels(&service.selector)` then `matches(&pod.labels)` (0012). Selector-less and `ExternalName` services have none |
| Deployment | ReplicaSet | Owns | `rs.owner` |
| RS / STS / DS | Pod / group | Owns | `pod.controller` (via the index) |
| HPA | Deployment / STS / RS | Owns | `hpa.target` |
| workload or pod | ConfigMap / Secret / PVC | Mounts | pod refs, aggregated |

**Pod refs** (every container kind): env `ConfigMapKey`/`SecretKey`; `env_from` `ConfigMap`/`Secret`; mounts `ConfigMap`, `Secret`, `PersistentVolumeClaim`, and `Projected { config_maps, secrets }`; plus `pod.image_pull_secrets` (0016). `kube-root-ca.crt` is skipped.

**Aggregation**: `top_owner(pod)` is the pod's controller, or its ReplicaSet's Deployment when that is listed. If the top owner is a visible node, the ref edge leaves it; otherwise it leaves the pod. Edges are deduped. When a target is not listed, a Ready feed makes it `Missing` (checks); a Loading or Off feed makes it `Unchecked`.

## Pod groups (decision 13)

The key is the pod's controller (the ReplicaSet, so a rollout shows two sets). An owner with `> POD_GROUP_LIMIT` (6) non-Bad pods, whose `PodGroup` id is not in `expanded`, gets one group node; edges to its pods merge into it. A click inserts the id into `expanded`, and a namespace change clears the set.

## Filters (step 2) and node cap (step 1)

1. Build. 2. Drop the nodes of hidden `KindFilter`s and their edges. 3. Problems only (config-checks.md). 4. Drop config nodes left without edges unless they have a check (decision 8). 5. More than `NODE_LIMIT` (500) nodes → `TooLarge::Nodes`. 6. Keep only the checks whose node is visible, and remap the edge indices.

`resources` = Σ `objects` of the visible nodes (the header's `{n} resources`).

## Group key

The node's own label: `app.kubernetes.io/name`, then `app`, then `k8s-app` (pods use `pod.labels`, others `row.labels`). A node without one takes the group of its first neighbour that has one, in one pass over the edges sorted by `(from, to)`; otherwise it is `None` ("Ungrouped"). The view passes `resolve_group_by(..)` to `layout` and shows it in the dropdown label.
