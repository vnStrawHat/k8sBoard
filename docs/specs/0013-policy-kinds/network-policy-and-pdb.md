# 0013 · NetworkPolicies and PDBs (step 2)

[Back to index](README.md) · Modules: `network_policy_rows.rs` (new) + tests in module, `policy_rows.rs` (new; PDB here, HPA and quota in step 3) + `policy_rows_tests.rs`, `kind_join.rs`, `live_sections.rs`, `kind_diagnosis.rs`. Common cell rules (Name first, Age last, `Absent` = muted "—") are 0005's. W7 `meta` lines and top buttons (Test traffic, New) are not rendered ([app-model.md](app-model.md)).

## NetworkPolicies

| Column | Width | Cell |
|---|---|---|
| Pod selector | 220 | `Mono(terms joined ", ")`; `selects_everything` → muted `Text("(all pods)")` |
| Policy types | 130 | isolated directions: `Ingress`, `Egress`, `Ingress, Egress`; none → `Absent` (defensive only: API defaulting always isolates Ingress) |
| Affects | 90 r | joined: `Quantity { "{n} pods" ("1 pod"), n, tone: Warn when 0 }`; pods not ready → `Absent` |
| Age | 70 r | as 0005 |

Status: builder Ok `{policy types}` (Info would count as Unhealthy in the 0009 chip); the join replaces it with Ok `"{n} pods"` or Warn "Selects no pods". A policy that isolates nothing (defensive only) reads Done "No isolation".

Sections (Overview, top to bottom):

| Section | Rows |
|---|---|
| Applies to | `Chips(terms)`; everything → `Note("All pods in {namespace}")` |
| Allow ingress from | only when ingress is `Allowed`: one `Stacked` row per peer of each rule (peer text above, ports text below; a `Field` label column truncates the sentences); a rule with no peers gives one row "any source"; `Allowed(empty)` → `Note("Denies all ingress to the selected pods")` |
| Allow egress to | same for egress; no peers → "any destination"; empty → `Note("Denies all egress from the selected pods")` |
| Labels | as 0005 |

### Sentences (pure, `network_policy_rows.rs`)

```rust
fn peer_text(peer: &PolicyPeer) -> String;
fn ports_text(ports: &[PolicyPort]) -> String;
```

| Peer | Text |
|---|---|
| `Pods { namespaces: None, pods: Some(s) }` | `pods {terms}`; everything → `all pods in this namespace` |
| `Pods { namespaces: None, pods: None }` | `all pods in this namespace` (defensive: the cluster crate drops a peer with neither selector) |
| `Pods { namespaces: Some(n), pods: None }` | one term `kubernetes.io/metadata.name={x}` → `namespace {x}`; everything → `all namespaces`; else `namespaces {terms}` |
| `Pods { namespaces: Some(n), pods: Some(s) }` | `{pods part} in {namespaces part}`, e.g. `pods app=worker in namespace jobs`, `all pods in all namespaces` |
| `IpBlock` | `{cidr}`, plus ` except {a, b}` |

| Ports | Text |
|---|---|
| empty | `all ports` |
| one | `port 8080/TCP`; a named port `port http/TCP`; a range `ports 8000-9000/TCP`; no port `all TCP ports` |
| several | `ports 80/TCP, 443/TCP` |

Terms join with `, `.

## PDBs

| Column | Width | Cell |
|---|---|---|
| Min available | 110 | `Text` or `Absent` |
| Max unavailable | 135 | `Text` or `Absent` |
| Allowed disruptions | 155 r | by `disruption_state()`: `Allowed(n)` → Ok `n`; `Blocked` → Bad `0`; `NoPods` → Done `0` |
| Age | 70 r | |

Status: `Allowed(n)` → Ok "{n} disruptions allowed" ("1 disruption allowed"); `Blocked(SyncFailed)` → Bad "Budget not computed"; other `Blocked` → Bad "0 disruptions allowed"; `NoPods` → Done "Selects no pods".

The same `disruption_state()` also drives the per-pod preview of the drain dialog (0034 `drain_plan.rs`).

Sections:

| Section | Rows |
|---|---|
| WHY | BLOCKS DRAIN box (below) |
| Budget | Min available, Max unavailable (each only when set), Healthy `{current_healthy} of {expected_pods} (needs {desired_healthy})`, Allowed disruptions (toned as the cell), Unhealthy eviction (`{policy}` or "IfHealthyBudget (default)"); when `is_status_stale`: `Note("Status describes an older version of this budget; the controller has not caught up")` |
| Selector | `Chips(terms)`; `None` → `Note("No selector: selects no pods")`; everything → `Note("All pods in {namespace}")` |
| Selected pods | `Live(SelectedPods)` |
| Conditions | 0005 rows (`DisruptionAllowed` with reason) |
| Labels | |

### BLOCKS DRAIN (`kind_diagnosis.rs`, Bad)

| Cause | Text |
|---|---|
| `SyncFailed` | `The disruption controller cannot compute this budget ({message}). Evictions of the selected pods are refused, so draining a node that runs them will wait.` |
| `UnhealthyPods` | `Only {current_healthy} of {expected_pods} pods are healthy and {budget}. Draining any node that runs these pods will wait.` |
| `NoRoom` | `{budget} and all {expected_pods} pods must stay up, so no pod can be evicted. Draining any node that runs these pods will wait until the budget changes.` |
| `NoRoom`, one pod | `{budget} and the only pod must stay up, so it cannot be evicted. Draining the node that runs this pod will wait until the budget changes.` |

`{budget}` = `minAvailable is {v}` when set, else `maxUnavailable is {v}`, else "the budget allows no disruption". Title `BLOCKS DRAIN`, no pod link.

### Selected pods (Live)

- Pods of the PDB's namespace with `selector.matches(&pod.labels)`; `None` selector → none.
- Healthy = the pod's `Ready` condition is true.
- Order: unhealthy first, then name. Row: name (mono) · Ok "healthy" / Bad "unhealthy"; click → `reveal` the pod. At most 50, then `+{n} more`.
- First line: muted `{n} pods · {h} healthy` ("1 pod" for one; section titles are static, 0005 decision 30).
- Pods `Loading` / `Failed` / none: "Loading pods…" / "Pods are unavailable" / "No pods match".

### Implementation notes

- The Budget row label is "Unhealthy eviction" (the longer label truncated in the drawer).
- The sidebar group that holds the active screen starts open (`is_section_open`); the kit keeps the toggle state after the first render, so this applies at launch.
