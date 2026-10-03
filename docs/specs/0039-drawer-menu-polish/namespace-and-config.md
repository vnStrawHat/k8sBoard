# 0039 · Namespace, ConfigMap, and ResourceQuota items

[Back to index](README.md) · Steps 1 and 4 · Decisions 12–16.

## ResourceQuotas: the stale `Edit` (step 1, W7 ResourceQuotas)

`resource_kind.rs` `RESOURCE_QUOTAS.read_only_actions` is `&[KindAction::named("Edit")]`, shown disabled (`NOT_SHIPPED_REASON`) next to a working `Edit YAML` (`edit_yaml_kind`). It becomes `&[]`. W7's `Edit · E` is Edit YAML (key E). Secrets keep their `Edit` (value editing is a later, mutating spec).

## ConfigMap restart hint (step 1, W7 ConfigMaps note)

```rust
// live_sections.rs, next to used_by_rows
/// The note under Used by when some user reads the ConfigMap through env: those values are read
/// once at container start. CronJob and Job owners are left out (each run starts fresh), and so are
/// bare `pod/{name}` owners (a pod without a controller is not restarted; it is replaced). Pure.
fn restart_hint(users: &[&UsedBy]) -> Option<String>;
```

| Users (`UsedBy.ways`) | Hint |
|---|---|
| none has `env` or `env from` (only `volume`), or only `cronjob/…`, `job/…`, or bare `pod/…` owners | `None` |
| 1–3 env readers | `Env values are read when a container starts: restart deployment/api to use a change. Mounted files update on their own (not with subPath).` |
| more | the first 3 owners, then `and {n} more` |

- Shown by `used_by_rows` as a muted `note` after the list and before `From pods in {scope}`.
- Text only: no button, no write (decision 13). The owners above it are links; their menus and key R restart them through the existing 0032 flow.
- The hint reads the same `config_map_users` join as Used by, from the `live` data of the session in view.

## LimitRange row (step 4, W7 Namespaces Quota section)

### Cluster crate (`crates/cluster/src/limit_range.rs`, new)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitRangeSummary { pub namespace: String, pub name: String, pub limits: Vec<LimitRangeLimit> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitRangeLimit {
    pub kind: String,                              // `type`: Container, Pod, PersistentVolumeClaim
    pub default: BTreeMap<String, String>,         // resource → quantity as written
    pub default_request: BTreeMap<String, String>,
    pub max: BTreeMap<String, String>,
    pub min: BTreeMap<String, String>,
}
impl ClusterConnection {
    /// Watches LimitRanges in `scope`; batched snapshots ordered by (namespace, name). Read-only.
    pub fn watch_limit_ranges(&self, scope: NamespaceScope) -> impl Stream<Item = WatchUpdate<LimitRangeSummary>> + Send + 'static;
}
pub(crate) fn limit_range_summary(limit_range: &LimitRange) -> LimitRangeSummary;
```

- Same shape as `watch_resource_quotas` (`summary_watch`, `scoped_apis`); `lib.rs` exports the two types.
- `access_review.rs`: `AccessCheck::ListLimitRanges` → `("list", "", "limitranges", None, true)`, in `ALL`: one more SSAR at connect per namespace of the scope, as every namespaced check (`all_checks_cover_distinct_permissions` count + 1).

### App (`related_objects.rs`, `cluster_session.rs`, `live_sections.rs`)

| Item | Today | Change |
|---|---|---|
| `RelatedList::ResourceQuotas(LiveList<ResourceQuotaSummary>)` | the Namespace drawer's one list | `NamespaceLimits { quotas: LiveList<ResourceQuotaSummary>, limit_ranges: LiveList<LimitRangeSummary> }` |
| `RelatedUpdate` | `ResourceQuotas` | + `LimitRanges(WatchUpdate<LimitRangeSummary>)`; `apply` routes each to its list |
| `RelatedList::resource_quotas()` | the quotas | `namespace_limits() -> Option<(&LiveList<ResourceQuotaSummary>, &LiveList<LimitRangeSummary>)>` |
| `RelatedObjects::start(subject, runtime, connection, cx)` | one stream | gains `gates: NamespaceListGates` (below); `NamespaceQuotas` → `stream::select` of the streams whose gate is open; a closed gate leaves its stream out (no 403 retry loop) |
| `denied_related_check(subject, access)` (one check per subject; `app_shell.rs` drops a denied subject, `live_sections.rs` prints it) | `NamespaceQuotas` → `ListResourceQuotas` | `NamespaceQuotas` → `None`; its two lists are gated by `namespace_list_gates` instead, and `app_shell.rs` drops the subject only when **both** gates are closed |
| `namespace_quota_rows` | `Not permitted` for the whole section, or quota rows / `No ResourceQuota` | each half reads its own gate, as in the table below |

```rust
// cluster_session.rs, next to denied_related_check
/// Each list of the Namespace drawer's Quota section, gated by its own check. `Known` and denied
/// closes a gate; `Checking`, `Unknown`, and allowed leave it open (the server answers). Pure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NamespaceListGates { pub(crate) quotas: bool, pub(crate) limit_ranges: bool }
pub(crate) fn namespace_list_gates(access: &AccessState) -> NamespaceListGates;
```

| `ListResourceQuotas` | `ListLimitRanges` | Watch | Quota section reads |
|---|---|---|---|
| open | open | both streams | quota rows (or `No ResourceQuota`), then LimitRange rows (or `No LimitRange`) |
| open | closed | quotas only | quota rows, then `Not permitted: list limitranges` |
| closed | open | limit ranges only | `Not permitted: list resourcequotas`, then LimitRange rows |
| closed | closed | none (subject dropped) | `Not permitted: list resourcequotas`, then `Not permitted: list limitranges` |

An open half shows `Loading quotas…` / `Loading limit ranges…` until its first snapshot and `… are unavailable` when its watch failed, independently of the other half. Loaded LimitRanges read one `wide_detail_row(name, text)` each.

```rust
/// `Container: default cpu 500m, memory 512Mi · request cpu 100m · max cpu 2`; items joined by `; `,
/// empty maps left out, a LimitRange without limits reads `no limits`. Pure.
fn limit_range_text(limit_range: &LimitRangeSummary) -> String;
```

- The text cell truncates with the full text as tooltip (`truncated_text`). No link: k8sBoard has no LimitRange screen.
- Watch budget: an open Namespace drawer runs at most one more watch (limit ranges), stopped with the drawer like every related watch.
