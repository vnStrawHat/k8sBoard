# 0009 · Cluster: several namespaces, pod labels

[Back to index](README.md) · Step 1 · Modules: `namespace.rs`, `connection.rs`, `resource_watch.rs`, `access_review.rs`, `pod.rs`, `event.rs`, every `watch_*` caller, `examples/probe.rs`

## `NamespaceScope`

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamespaceScope {
    All,
    Named(String),
    /// Two or more namespaces, sorted and unique. Build it with `of_namespaces`.
    Several(Vec<String>),
}
impl NamespaceScope {
    /// Sorts and dedups: none → `All`, one → `Named`, more → `Several`.
    pub fn of_namespaces(names: impl IntoIterator<Item = String>) -> Self;
    /// The picked namespaces in order; empty for `All`.
    pub fn namespaces(&self) -> &[String];   // Named → std::slice::from_ref
}
```

No limit in the crate; the app enforces `MAX_NAMESPACES` (decision 15).

## Scoped APIs, watches, lists

| Today | Step 1 |
|---|---|
| `scoped_api(scope) -> Api<K>` | `scoped_apis(scope) -> Vec<(Option<String>, Api<K>)>`: `All` → `[(None, Api::all)]`; `Named`/`Several` → one `(Some(ns), Api::namespaced)` per namespace, in order |
| `summary_watch(conn, api, ..)` | takes that vec; one entry → today's stream; more → `merge_snapshots` of one watch per entry |
| `limited_summary_watch(.., limit)` | same; the merge trims with the same `StoreLimit` |
| `list_pods(scope)` | lists each api in order and concatenates |
| `watch_namespaces`, `watch_nodes`, `watch_object_events` | pass a one-entry vec |

## `merge_snapshots` (Several only)

```rust
/// One input per namespace, in namespace order. Coalesces like the Batcher: at most one merged
/// snapshot per `BATCH_WINDOW`. A failing namespace never hides the others' data.
pub(crate) fn merge_snapshots<T: Clone + Send + 'static>(
    inputs: Vec<(String, BoxStream<'static, WatchUpdate<T>>)>,
    limit: Option<StoreLimit<T>>,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static;
```

Per input state: `Waiting` (nothing yet), `Items(Vec<T>)` (latest snapshot), `FailedEmpty` (failed before any snapshot). An input is *settled* unless `Waiting`.

| Input event | Effect |
|---|---|
| `Snapshot(items)` | state `Items(items)`; mark dirty; start the window (`deadline = now + BATCH_WINDOW`) if none |
| window ends (`wait_until(deadline)`), all settled, at least one `Items` | emit `Snapshot(concatenation)`, trimmed by `limit` |
| `Failed(error)` | `Waiting` → `FailedEmpty` (an `Items` input keeps its stale items). Flush now: if dirty, all settled, and one `Items` exists, emit the merged snapshot. Then emit `Failed(ClusterError::Namespace { namespace, source })` |
| an input ends | dropped from `select_all`; its last state stays; the merge ends when every input ended |

- Order follows the Batcher: pending snapshot first, then the failure. So a `LiveList` gets Ready data from the healthy namespaces, then an interruption naming the failing one; the existing banner shows it, and that namespace's next snapshot clears it. No failing namespace with data anywhere → only `Failed` reaches the list (its error state).
- Shape: a struct like `Batcher` driven by `stream::unfold`, with `tokio::select! { biased; wait_until(deadline), select_all.next() }`. No task is spawned; dropping it drops every watch.
- Order: inputs are sorted by (namespace, name) and in namespace order, so the concatenation keeps the crate's snapshot order. `limit` keeps the `max_items` most recent, preserving order. `StoreLimit` derives `Clone, Copy`.

```rust
// connection.rs, ClusterError
#[error("namespace '{namespace}'")]
Namespace { namespace: String, #[source] source: Box<ClusterError> },
```

The app's `error_text` then reads `namespace 'web': cannot reach the API server …`.

## Access review

| Scope | `review_access` |
|---|---|
| `All`, `Named` | unchanged (19 concurrent SSARs) |
| `Several(names)` | the 3 cluster-scoped checks once; then the 16 namespaced checks per namespace, **one namespace at a time**, concurrent within it: 16 × N + 3 SSARs |

```rust
impl AccessReport {
    /// One review per check, in `AccessCheck::ALL` order: Allowed only if allowed in every
    /// report that contains it; else the first denial.
    pub(crate) fn all_of(reports: Vec<AccessReport>) -> AccessReport;
}
```

- A private `review_checks(&self, checks, namespace: Option<&str>)` holds today's concurrent body; `resource_attributes(check, namespace: Option<&str>)` replaces its scope parameter. No async recursion.
- The AND is **conservative gating** (menus and sidebar items), not data filtering: lists still show every namespace that answers.
- Any request error still fails the whole call (a report is never partial).

## Pod labels

`PodSummary.labels: Vec<String>`, `key=value` in key order, from `workload::label_terms`. Labels are not secret (kubectl shows them); annotations are never read. This was a 0012 item; 0012 drops it.

## App compile arms (step 1 only)

| Place | `Several(names)` arm |
|---|---|
| `LiveCluster::scope_label` | `namespaces_label(names)`: `a, b`; three or more → `a, b +N` (pure, `cluster_session.rs`) |
| `title_bar.rs` picker label | `ns: ` + the same text |
| `navigation.rs::kind_availability` | `Not permitted: {check} in {namespaces_label}` |
| test fixtures building `PodSummary` | `labels: Vec::new()` |

## Probe

`--namespace a,b` splits on commas into `of_namespaces`. The pods line prints a count; coder-lite compares it with single-namespace runs (AC5).
