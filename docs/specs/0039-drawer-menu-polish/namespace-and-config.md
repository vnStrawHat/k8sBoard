# 0039 · Namespace, ConfigMap, and ResourceQuota items

[Back to index](README.md) · Steps 1 and 4 · Decisions 12–16.

## ResourceQuotas: the stale `Edit` (step 1, W7 ResourceQuotas)

`resource_kind.rs` `RESOURCE_QUOTAS.read_only_actions` is `&[KindAction::named("Edit")]`, shown disabled (`NOT_SHIPPED_REASON`) next to a working `Edit YAML` (`edit_yaml_kind`). It becomes `&[]`. W7's `Edit · E` is Edit YAML (key E). Secrets keep their `Edit` (value editing is a later, mutating spec).

## ConfigMap restart hint (step 1, W7 ConfigMaps note)

```rust
// live_sections.rs, next to used_by_rows
/// The note under Used by when some user reads the ConfigMap through env: those values are read
/// once at container start. CronJob and Job owners are left out (each run starts fresh). Pure.
fn restart_hint(users: &[&UsedBy]) -> Option<String>;
```

| Users (`UsedBy.ways`) | Hint |
|---|---|
| none has `env` or `env from` (only `volume`), or only `cronjob/…` / `job/…` owners | `None` |
| 1–3 env readers | `Env values are read when a container starts: restart deployment/api to use a change. Mounted files update on their own (not with subPath).` |
| more | the first 3 owners, then `and {n} more` |

- Shown by `used_by_rows` as a muted `note` after the list and before `From pods in {scope}`.
- Text only: no button, no write (decision 13). The owners above it are links; their menus and key R restart them through the existing 0032 flow.
- The hint reads the same `config_map_users` join as Used by, in the ConfigMap's own cluster (`live` of the drawer subject).

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
- `access_review.rs`: `AccessCheck::ListLimitRanges` → `("list", "", "limitranges", None, true)`, in `ALL` (one more SSAR at connect; `all_checks_cover_distinct_permissions` count + 1).

### App (`related_objects.rs`, `cluster_session.rs`, `live_sections.rs`)

| Item | Today | Change |
|---|---|---|
| `RelatedList::ResourceQuotas(LiveList<ResourceQuotaSummary>)` | the Namespace drawer's one list | `NamespaceLimits { quotas: LiveList<ResourceQuotaSummary>, limit_ranges: LiveList<LimitRangeSummary> }` |
| `RelatedUpdate` | `ResourceQuotas` | + `LimitRanges(WatchUpdate<LimitRangeSummary>)`; `apply` routes each to its list |
| `RelatedList::resource_quotas()` | the quotas | `namespace_limits() -> Option<(&LiveList<ResourceQuotaSummary>, &LiveList<LimitRangeSummary>)>` |
| `RelatedObjects::start(subject, runtime, connection, cx)` | one stream | gains `access: &AccessState`; `NamespaceQuotas` → `stream::select(quotas, limit_ranges)`; the limit-range stream is left out when `Known` and `ListLimitRanges` is denied (no 403 retry loop) |
| `namespace_quota_rows` | quota rows or `No ResourceQuota` | then LimitRange rows: `Not permitted: list limitranges` (denied), `Loading limit ranges…`, `LimitRanges are unavailable`, `No LimitRange`, or one `wide_detail_row(name, text)` per LimitRange |

```rust
/// `Container: default cpu 500m, memory 512Mi · request cpu 100m · max cpu 2`; items joined by `; `,
/// empty maps left out, a LimitRange without limits reads `no limits`. Pure.
fn limit_range_text(limit_range: &LimitRangeSummary) -> String;
```

- The text cell truncates with the full text as tooltip (`truncated_text`). No link: k8sBoard has no LimitRange screen.
- Watch budget: an open Namespace drawer runs at most one more watch (limit ranges), stopped with the drawer like every related watch.
- Multi-cluster: the related watch runs in the drawer subject's slot only (0027 decision 22); nothing changes there.
