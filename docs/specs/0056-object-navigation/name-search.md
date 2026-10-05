# 0056 · Palette name search beyond loaded kinds (M28)

[Back to index](README.md). Supersedes 0029 decision 9 ("no new list call") for the Resources group only.

## Today

Resources searches the visible explorer kind, Pods, Nodes. A Service or Deployment name finds nothing: `No matches. Searched: Pods, Nodes.`

## Two layers

| Layer | Kinds | Source | API load |
|---|---|---|---|
| 1 · Already loaded | Deployments, DaemonSets, Jobs, HPAs, PDBs, ResourceQuotas, PVCs, TLS Secrets | the Issues condition feeds (`issue_feeds`) | none |
| 2 · Name index | Services, Ingresses, StatefulSets, CronJobs, NetworkPolicies (`NAME_INDEX_KINDS`) | one-shot **metadata-only LIST** per kind | see cost |

Everything else (ConfigMaps, all Secrets, ServiceAccounts, RBAC, storage, CRDs, custom kinds, ReplicaSets, Events) stays reachable with `:kind`, as today. Adding ConfigMaps, Secrets, ServiceAccounts (the bulky lists) is a possible follow-up once the cost below is measured on UAT.

### Layer 1 rules

- `condition_plan` watches `NamespaceScope::All` when the scope has more than two namespaces, so a feed can hold objects outside the scope: filter feed subjects with the existing `cluster_session::scope_includes`.
- A feed can be `Off` (denied, or TLS Secrets off in Settings) or `Wait` (review running). Only feeds whose list is live and loaded are searched, and only those are named in the `Searched:` text.

## Name index lifecycle

- Owner: `LiveCluster.name_index: NameIndex` (new `crates/app/src/name_index.rs`), dropped with the session.
- Start: `ClusterSession::request_name_index(cx)`, called by `command_palette.rs` when the query is `All` mode with **≥ 2 characters**. Pure `wants_run(fetched_at, in_flight, scope, now)`: runs when never fetched, the scope differs, or `fetched_at` is older than **120 s** (`NAME_INDEX_MAX_AGE`); never while a run is in flight. Opening the palette alone, or `:`/`#`/`>`/`@` modes, start nothing.
- **Every finished run stamps `fetched_at`, failed or not**, so a failure is retried after 120 s, not on the next keystroke.
- Run: mirrors `refresh_kind_counts`: one `runtime.spawn`, kinds in `buffer_unordered(KIND_COUNT_CONCURRENCY)`, one result applied with one `cx.notify()`. The palette re-ranks on that notify (the existing throttled shell refresh).
- Cancellation: `NameIndex` holds the run's task; a scope change or the session drop aborts it, and a result whose scope differs from the current one is discarded.
- Kinds the known access report denies are not listed: state `Denied(check)`. A 403 from the server (report unknown) is `Failed`.
- Privacy: only namespace and name are kept. Object bodies, labels, and annotations are never stored, logged, or traced; queries are not traced (0029 rule).

```rust
// crates/cluster/src/object_names.rs (new)
pub struct ObjectName { pub namespace: Option<String>, pub name: String }
pub struct NameList { pub names: Vec<ObjectName>, pub is_truncated: bool }
impl ClusterConnection {
    /// Metadata-only LIST in pages of 500, stopped at 5,000 names per kind.
    pub async fn list_object_names(&self, kind: ObjectKind, scope: &NamespaceScope) -> Result<NameList, ClusterError>;
}
// crates/app/src/name_index.rs (new)
pub(crate) enum NameListState { Loading, Ready(NameList), Denied(AccessCheck), Failed(String) }
pub(crate) struct NameIndex { scope: NamespaceScope, fetched_at: Option<Instant>, lists: Vec<(ResourceKind, NameListState)>, run: Option<Task<()>> }
```

Uses `scoped_apis` (one list per namespace of a `Several` scope, one cluster-wide list for `All`), like `count_objects`. GET only: allowed by the read-only rule.

## Cost (3,000-pod cluster, scope All)

| Item | Estimate |
|---|---|
| Requests per run | 5 LISTs (`Several(n)`: 5 × n, n ≤ 5), at most one run per 120 s, only while the user searches |
| Server work | `limit=500` without `resourceVersion` is a quorum read from etcd per page on v1.29 (not the watch cache; same reason as `count_params` in `object_count.rs`), and the server still decodes whole objects before trimming to metadata |
| Transfer | metadata items carry `managedFields`, ~1-3 KB each; typical counts (500 svc, 200 ing, 300 sts, 100 cj, 100 netpol) ≈ 1,200 items ≈ 1-4 MB per run |
| Memory kept | names only, ~100 B each ≈ 0.1 MB |
| Scan | the 0029 scan grows from ~3,000 to ~5,000 subjects (plus layer 1); the `palette ranked` trace must stay under 4 ms |

## Results

- New `Subject::Named { kind, namespace, name }` for layers 1 and 2: label = name, detail = `namespace/name`, keywords = kind words (so `svc api` narrows to Services), **no status**, no action pairs (pairs stay on loaded rows, pods, nodes).
- Dedup by key, first source wins: explorer rows, pods, nodes, condition feeds, name index (so a loaded row keeps its status).
- No sub-headings per kind (the kit `Command` groups are fixed); entries rank by score. Cap stays `RESOURCES_CAP` = 50.
- Confirm = `PaletteTarget::Resource` -> `reveal_object`: the kind screen opens, its explorer loads, `pending_reveal` selects the row.

## What the user sees

| State | Footer / empty text |
|---|---|
| Index loading, some hits | hits shown; the palette footer (0029 palette-ui) reads `Searching Services, Ingresses, StatefulSets…` |
| Index loading, no hits | `No matches yet. Searching Services, Ingresses, StatefulSets…` |
| Done, no hits | `No matches. Searched: Pods, Nodes, Deployments, … NetworkPolicies.` then `Not permitted: Ingresses.` when a kind is denied, then `Type :kind for other kinds.` |
| A kind truncated at 5,000 | `Services: first 5,000 names` appended to the searched line |

`empty_text` / `resources_hint` take the feed and index states instead of only `screen`.
