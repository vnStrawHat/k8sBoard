# 0018 · Instances counts and stuck namespaces

[Back to index](README.md) · Steps 4 and 5 · Modules: `cluster_session.rs` (+ tests), `kind_join.rs` (+ tests), `navigation.rs`, `app_shell.rs`; step 5: `crates/cluster/src/namespace.rs` (+ tests), `namespace_rows.rs`, `kind_row.rs`, `kind_diagnosis.rs` (+ tests)

## Instances counts (step 4, decision 31)

```rust
// cluster_session.rs
pub(crate) struct CustomCounts { counts: HashMap<CustomKind, u64>,
    refreshed_at: Option<Instant>, _task: Option<Task<()>> }
impl ClusterSession { fn refresh_custom_counts(&mut self, cx: &mut Context<Self>); }
impl LiveCluster { pub(crate) fn custom_count(&self, kind: CustomKind) -> Option<u64>; }
```

- **Trigger**: `AppShell::show_screen(Kind(Crds))` asks; a run starts when 30 s passed since `refreshed_at` (or none ran); a new run replaces the task. Counts are **cluster-wide** (decision 31), so a scope change keeps them.
- **Run**: every kind in `crds.kinds` whose gate is not `Denied`, on the cluster runtime: `stream::iter(kinds).map(|kind| count_custom_objects(kind.resource())).buffer_unordered(4)` (one `limit=1` list per CRD), collected, then one `update` and one `notify`. A failure or `None` leaves that kind without a count; only the CRD name and the error text are logged.
- **Join** (`kind_join.rs`, 0012 pattern): `join_rows` arm `Crds` fills column `CRD_INSTANCES` with `Quantity { text: n, value: n, tone: None }` for the row whose name is the kind's `crd_name`, else `Absent`. `JoinInputs` gains `custom_counts: &HashMap<CustomKind, u64>` plus the kinds to map CRD names. Triggers: the CRD feed into the Crds explorer and the end of a count run. A test asserts `CRD_INSTANCES` names the Instances column.
- **Link.** In the CRDs table a known Instances count is a link-styled number (`kind_table.rs`, `instances_link`) that opens the instances (the Browse instances action); a click does not open the CRD drawer.
- **Sidebar**: `NavigationCounts` gains `custom: HashMap<CustomKind, usize>`; a custom item shows the live explorer count (scoped) when shown, else the cluster-wide counted one, else none.
- Ceilings: 300 CRDs cost 300 requests per run (4 at a time); a user without cluster-wide `list` gets no counts; Instances ignores the namespace picker.

## Stuck namespaces (step 5, decision 32)

### Cluster (`namespace.rs`)

```rust
pub struct NamespaceSummary { /* … */
    /// `metadata.deletionTimestamp`.
    pub deleting_since: Option<jiff::Timestamp>,
    /// Deletion conditions with status True, API order.
    pub deletion_conditions: Vec<NamespaceDeletionCondition> }
pub struct NamespaceDeletionCondition { pub name: String, pub reason: Option<String>, pub message: Option<String> /* ≤ 500 chars */ }
```

Kept types: `NamespaceDeletionDiscoveryFailure`, `NamespaceDeletionGroupVersionParsingFailure`, `NamespaceDeletionContentFailure`, `NamespaceContentRemaining`, `NamespaceFinalizersRemaining`. Exported from `lib.rs`.

### App

- `KindObject::Namespace(NamespaceSummary)`; `namespace_row` clones the summary into it (0012/0013 Namespaces joins and related subjects read the row name as today).
- `pub(crate) const STUCK_AFTER: jiff::SignedDuration = jiff::SignedDuration::from_mins(5);`
- **STUCK box** (`kind_diagnosis` arm, paint time): phase Terminating and `now - deleting_since > STUCK_AFTER` → title `STUCK`, tone Bad when any `*Failure` condition exists, else Warn; text `Terminating for {age}.` then each condition message in API order, joined by a space; no message → `The namespace reports no reason.`
- **Remaining resources** section (pre-built, Terminating only, after the Namespace section):

```rust
pub(crate) enum RemainingEntry { Resource { resource: String, count: u64 }, Finalizer { finalizer: String, count: u64 }, Unparsed(String) }
pub(crate) fn remaining_entries(conditions: &[NamespaceDeletionCondition]) -> Vec<RemainingEntry>;
```

| Condition | Message form (Kubernetes namespace controller) | Entries |
|---|---|---|
| `NamespaceContentRemaining` | `Some resources are remaining: kafkatopics.kafka.strimzi.io has 1 resource instances, pods. has 2 resource instances` | `Resource` per `{resource} has {n} resource instances` item (text after the first `": "`, split on `", "`); a trailing `.` (core group) is trimmed |
| `NamespaceFinalizersRemaining` | `Some content in the namespace has finalizers remaining: strimzi.io/topic-operator in 1 resource instances` | `Finalizer` per `{finalizer} in {n} resource instances` item |
| any other, or an item that does not match | — | `Unparsed(message)` |

Rows: `Resource` → `Field { resource (Mono), Text("{n} remaining") }`; `Finalizer` → `Field { "finalizer {finalizer}", Toned(Warn, "{n} objects") }`; `Unparsed` → `Note(text)`; no entries → `Note("The namespace reports no remaining content.")`.

- W7 deviations: object names (`kafkatopic/feature-x-events`) are not listed (types and finalizers only, no extra request); the "Show remaining resources" menu item is not rendered (the section shows them).
