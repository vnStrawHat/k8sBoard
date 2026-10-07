# 0020 · Rules: workloads, policy, storage, namespaces, certificates

[Back to index](README.md) · Step 2 · Module: `issue_kind_rules.rs` (+ tests in module). Inputs: the Ready condition feeds as `&[KindObject]` ([feeds.md](feeds.md)) and the namespaces list. Every finding's action is Open; object = the summary's kind, namespace, and name.

## Objects through `kind_diagnosis` (decision 14)

```rust
/// One finding per object whose WHY box fires with no pods: only condition rules (*) can.
fn object_finding(kind: ResourceKind, object: &KindObject, nodes: &[NodeSummary], now: Timestamp) -> Option<Finding>;
```

`DiagnosisInputs { pods: &[], nodes, service: None, tls_secrets: None, now, .. }` (other 0013–0018 fields `None`). Reason = the box title in sentence case (`ROLLOUT STALLED` → `Rollout stalled`); cause = the box text.

| Feed | Boxes that can fire (spec) | Severity | Grace |
|---|---|---|---|
| Deployments | D1 ROLLOUT STALLED, D2 REPLICA FAILURE (0012) | tone → severity | — |
| DaemonSets | S3 NODES WITHOUT A POD, S4 MISSCHEDULED (0012) | tone → severity | `ROLLOUT_GRACE` (every DaemonSet finding) |
| Jobs | J1 BACKOFF LIMIT REACHED, J2 DEADLINE EXCEEDED, J3 JOB FAILED (Bad), J4 FAILED ATTEMPTS (Warn) (0012) | tone → severity | `ROLLOUT_GRACE` for Warn (J4 is the only Warn Job box) |
| HPAs | METRICS UNAVAILABLE, CANNOT SCALE, SCALING INACTIVE, AT MAX REPLICAS (0013) | Warning (decision 19) | — |
| PDBs | BLOCKS DRAIN (0013) | Warning | — |
| ResourceQuotas | AT QUOTA (0013) | Warning | — |
| PVCs | VOLUME LOST (0014) | tone → severity | — |

Grace is keyed by (kind, tone), so no box title is matched. Rule ids: `KindRollout` (Deployments, DaemonSets), `KindJob`, `KindAutoscaler`, `KindDisruptionBudget`, `KindQuota`, `KindClaim`. Onset: the box condition's `changed_at` when the summary has it, else first-seen.

A test asserts, per feed kind, that `pods: &[]` never yields the pod-derived boxes (D3, S1, S2).

## Engine-own rules (no WHY box exists)

| Rule | Feed | When | Severity | Reason | Cause |
|---|---|---|---|---|---|
| QuotaNearLimit | ResourceQuotas | no AT QUOTA, an item with `quota_tone(quota_ratio(item))` Warn (≥ 90 %) | Warning | `Quota near limit` | `{resource} {quota_text} ({pct}).` (first such item) + ` {n} more near their limit.` |
| PvcPending | PVCs + Warning events | phase `Pending` **and** a Warning event on the claim; grace `PVC_PENDING_GRACE` from `created_at` | Warning | `Pending` | `Pending for {age}. {reason}: {message line}` |

PvcPending stays quiet while the Warning feed is not Ready (`events` `None`), and for `WaitForFirstConsumer` claims (Normal events only): they wait for a pod by design.

## Namespaces (step 2: needs 0018 step 5)

| Rule | When | Severity | Reason | Cause |
|---|---|---|---|---|
| NamespaceStuck | `kind_diagnosis(&KindObject::Namespace(summary.clone()), ..)` returns STUCK (0018); only Terminating namespaces are cloned | tone → severity | `Stuck terminating` | box text |

## Certificates (TLS secrets feed; 0016 contract)

| Rule | When (`expiry_state(leaf, now)`) | Severity | Reason | Cause |
|---|---|---|---|---|
| CertExpired | `Expired` | Critical | `Cert expired` | `Expired {date} ({n} ago).` |
| CertExpiring | `ExpiringSoon` (≤ `EXPIRY_WARNING`) | Warning | `Cert expiring` | `Expires in {n} ({date}).` (W3 wording) |

- Input = `KindObject::Secret(summary)` with `SecretDetails::Certificate { chain }`, leaf = `chain[0]`; `NoCertificate` and `NotYetValid` produce nothing (decision 23).
- Object = the Secret; target = the Secrets row (its drawer shows the CERTIFICATE box). Menu label `Open secret` (W3 "Open Secret").
- Every TLS secret counts, used or not (open item 2).

## Order inside `evaluate`

pods → nodes → `NamespaceStuck` → `KindRollout` → `KindJob` → `KindClaim` → `PvcPending` → `KindAutoscaler` → `KindDisruptionBudget` → `KindQuota` → `QuotaNearLimit` → certificates → `ServiceNoPods` → `VolumeFull` → events. `IssueRule` variants are declared in this order.

## ServiceNoPods (round 3, N14)

A Service whose selector matches no pod of its namespace while an Ingress path or default backend names it: Critical, `No matching pods`, cause `No pod in shop has the labels app=web-v2; Ingress shop routes to it.` (`Ingress a and 2 more route to it.`), onset the Service's creation, grace 2 min, action Open. A Service with no selector or of type ExternalName, one nothing routes to, and any Service while the pods, the Services, or the Ingresses feed has not loaded give nothing. The Services and Ingresses are two more condition feeds (`CONDITION_KINDS` has 10 kinds).

## Not built (no rule here)

StatefulSets, ReplicaSets, CronJobs (their pods and Jobs carry the problem), Services V2 (no ready endpoints), Ingress CERTIFICATE (the Secret issue covers it), PV RELEASED/RECLAIM FAILED, RBAC VERY BROAD/CLUSTER ADMIN (0015 hygiene, not outages).
