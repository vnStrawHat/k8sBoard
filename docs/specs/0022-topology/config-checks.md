# 0022 · Config checks

[Back to index](README.md) · Step 1 (rules), step 2 (chip dropdown, Problems only) · Module: `topology_checks.rs` (new, tests in module). Pure; `build_topology` calls it before filtering.

## Types

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CheckRule { ServiceNoPods, IngressMissingService, MissingConfigMap, MissingSecret, MissingClaim,
    ClaimNotBound, HpaMissingTarget, ObjectDiagnosis }
impl CheckRule { pub(crate) fn chip_label(self, count: usize) -> String; }   // table below
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigCheck {
    pub(crate) rule: CheckRule,
    pub(crate) node: NodeId,            // where it is drawn (a ghost for missing objects)
    pub(crate) tone: StatusTone,        // Bad or Warn
    pub(crate) text: String,            // full sentence: dropdown row and tooltip
}
pub(crate) fn graph_checks(parts: &GraphParts, inputs: &TopologyInputs) -> Vec<ConfigCheck>;
```

Checks are sorted by tone (Bad first), then rule, then `node`. There is one check per `(node, rule)`.

## Rules owned by Topology (W11 pin 2)

| Rule | When | Node | Tone | Text | Chip label (1 / n) |
|---|---|---|---|---|---|
| ServiceNoPods | selector set, not `ExternalName`, pods Ready, `service_health(service, pods, None).matching_pods == Some(0)` (the 0012 slice core) | `NoPods { service }` with edge Service → ghost (RoutesTo) | Bad | `Service {s} matches no pods (selector {terms}).` | `1 Service matches no pods` / `{n} Services match no pods` |
| IngressMissingService | a backend service is not listed (Services Ready) | `Missing { Service }` | Bad | `Ingress {i} routes to missing Service {s}.` | `1 Ingress to a missing Service` / `{n} Ingress routes to missing Services` |
| MissingConfigMap | a ref target is not listed (ConfigMaps Ready) | `Missing { ConfigMap }` | Warn | `{owner} references missing ConfigMap {name}.` | `1 missing ConfigMap` / `{n} missing ConfigMaps` |
| MissingSecret | the same for Secrets (TLS and pull secrets included) | `Missing { Secret }` | Warn | `{owner} references missing Secret {name}.` | `1 missing Secret` / `{n} missing Secrets` |
| MissingClaim | the same for PVCs | `Missing { PersistentVolumeClaim }` | Bad | `{owner} mounts missing PVC {name}.` | `1 missing PVC` / `{n} missing PVCs` |
| ClaimNotBound | phase `Pending` older than `PVC_PENDING_GRACE` (5 min, 0020), or `Lost` | the PVC | Warn / Bad | `PVC {name} is Pending for {age}.` / `PVC {name} lost its volume {volume}.` | `1 PVC not bound` / `{n} PVCs not bound` |
| HpaMissingTarget | `hpa.target` is not listed (its feed Ready) | `Missing { target kind }` | Warn | `HPA {h} scales missing {Kind} {name}.` | `1 HPA without a target` / `{n} HPAs without targets` |

- `{owner}` is the aggregated source node's `{Kind} {name}`. `{terms}` is `Selector::terms()` joined with `, `.
- Missing Secret and ConfigMap are Warn, because optional refs cannot be told apart (README open item 1). A missing PVC is Bad, because the pod cannot start.
- A rule whose feed is not `Ready` does not run, and its targets draw as `not checked` (decision 4).

## Reused object diagnosis (rule `ObjectDiagnosis`)

For each `Object` node of a kind with rules, call `kind_diagnosis(&row.object, &DiagnosisInputs { pods, nodes, service, tls_secrets, now, .. })` (0012; 0013–0016 fields; every other field `None`):

| Input | Value |
|---|---|
| `pods` | owned pods (via the controller index); Services: their matching pods |
| `service` | `Some(service_health(service, pods, None))`, using the same slice core as ServiceNoPods |
| `tls_secrets` | the Secrets rows filtered to `kubernetes.io/tls`, only when Ready |

A `Some(box)` sets the node tone to the worse of `box.tone` and the row tone. It adds a check `{Kind} {name}: {title in sentence case}.`, with chip label `1 object problem` / `{n} object problems`. Ingress `CERTIFICATE` boxes about a *missing* secret are skipped, because MissingSecret covers them; expiry boxes stay. Service V1 is skipped for the same reason (ServiceNoPods).

Pods are toned only by `pod_status_label` and add no check (decision 16).

## Coverage note

`topology_coverage(rows) -> Option<String>`. It is `None` when every enabled feed is Ready. Otherwise it reads, for example, `Not checked: secrets (not permitted). Loading: services.` Chip-disabled kinds are not listed. The note shows as a muted line under the toolbar.

## Checks chip and dropdown (step 2)

| State | Chip text |
|---|---|
| no checks | hidden |
| every check has one rule | `rule.chip_label(n)` (W11: `1 Service matches no pods`) |
| several rules | `{n} config problems` |

The chip is toned by the worst check. It is a kit dropdown button listing every check (a tone dot plus `text`; at most 50, then `+{k} more`). Picking one sets `pending_focus` to the node: center and select it. Ghost tooltips show `text`.

## Problems only (step 2, decision 17)

`problem_ids` = the nodes with a check or a Bad/Warn tone. The kept set is `problem_ids` plus their direct neighbours. A pod group counts when its tone is Warn. The toggle uses `toggle_button` (`workspace.rs`, widened to `pub(crate)`). With no problems, the canvas shows `No problems in {ns}.`
