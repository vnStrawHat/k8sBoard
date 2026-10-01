# 0001 · Access review and capabilities

[Back to index](README.md) · Modules: `src/access_review.rs`, `src/metrics_api.rs`

## Access review (SelfSubjectAccessReview)

- **Non-mutating.** SSAR is a review API, which `docs/agents/code-style.md` §2 allows. It is the **only POST** in the crate.
- Implementation: `Api::<SelfSubjectAccessReview>::all(client).create(&PostParams::default(), ..)`, nine times, run concurrently with `futures::future::try_join_all`.
- **No partial reports.** The first request error fails the whole call. A returned report always has all nine entries, in `AccessCheck::ALL` order.
- A denial is data, not an error.

All nine checks use group `""` (core):

| Variant | verb | resource | subresource | namespaced | Display |
|---|---|---|---|---|---|
| `ListPods` | list | pods | — | yes | `list pods` |
| `GetPodLogs` | get | pods | log | yes | `get pods/log` |
| `CreatePodExec` | create | pods | exec | yes | `create pods/exec` |
| `CreatePodPortForward` | create | pods | portforward | yes | `create pods/portforward` |
| `ListSecrets` | list | secrets | — | yes | `list secrets` |
| `ListNodes` | list | nodes | — | no | `list nodes` |
| `GetNodeProxy` | get | nodes | proxy | no | `get nodes/proxy` |
| `ListEvents` | list | events | — | yes | `list events` |
| `WatchPods` | watch | pods | — | yes | `watch pods` |

Private pure functions:

- `resource_attributes(check: AccessCheck, scope: &NamespaceScope) -> ResourceAttributes`
  - Namespaced checks: `Named(ns)` → `namespace = Some(ns)`; `All` → `None` (all namespaces).
  - Cluster-scoped checks always use `None`.
- `access_decision(status: Option<SubjectAccessReviewStatus>) -> AccessDecision`
  - `allowed == true` → `Allowed`.
  - Otherwise `Denied { reason }`, where `reason` is `status.reason` if non-empty, else `evaluation_error` if non-empty, else `None`.
  - A missing status → `Denied { reason: None }`.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessCheck { ListPods, GetPodLogs, CreatePodExec, CreatePodPortForward,
                       ListSecrets, ListNodes, GetNodeProxy, ListEvents, WatchPods }
impl AccessCheck { pub const ALL: [AccessCheck; 9] = [/* declaration order */]; } // + Display

pub enum AccessDecision { Allowed, Denied { reason: Option<String> } }
pub struct AccessReview { pub check: AccessCheck, pub decision: AccessDecision }
pub struct AccessReport { pub reviews: Vec<AccessReview> } // one per ALL, in order
impl AccessReport { pub fn is_allowed(&self, check: AccessCheck) -> bool; }

impl ClusterConnection {
    /// Never returns a partial report.
    pub async fn review_access(&self, scope: NamespaceScope) -> Result<AccessReport, ClusterError>;
}
```

## Metrics API probe (discovery only)

1. `client.list_api_groups()`. An error here propagates as `ClusterError`.
2. Find the group named `metrics.k8s.io` with the private `metrics_group_version(&APIGroupList) -> Option<String>`. It takes `preferredVersion.groupVersion`, falling back to the first entry of `versions`. If the group is absent → `NotInstalled`.
3. `client.list_api_group_resources(gv)`:
   - `Ok` → `Available { group_version }`.
   - Any error, timeout included → `Unavailable { group_version, reason }`, where `reason` is the `Display` text of the classified `ClusterError`.

Step 3 matters because an `APIService` can stay registered while metrics-server is down. The group then appears in `/apis`, but the group-version call returns 503. The probe never reads metric values.

```rust
pub enum MetricsApi {
    Available { group_version: String },
    Unavailable { group_version: String, reason: String },
    NotInstalled,
}

impl ClusterConnection {
    pub async fn metrics_api(&self) -> Result<MetricsApi, ClusterError>;
}
```
