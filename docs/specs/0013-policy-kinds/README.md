# 0013 — Policy kinds: NetworkPolicies, HPAs, ResourceQuotas, PDBs (read-only)

Status: amended after advisor review (HEAD `81497ba`); architect defaults, the user asked not to stop for questions. Crates: `crates/cluster` (step 1), `crates/app` (steps 2–3). Requires the amended 0012 merged (`KindObject`, `DetailRow::{Live, Bar}`, `KindCell::Quantity { tone }`, `cluster::Selector`, `kind_join.rs`, `related_objects.rs`, `kind_diagnosis.rs`, counts) and 0010 (`quantity.rs`, `usage_format.rs`). Wireframes: W7 `k("NetworkPolicies")`, `k("HPAs")`, `k("ResourceQuotas")`, `k("PDBs")`, Namespace drawer "Quota". Applies C1, C7, C11.

## Goal

Four new explorer kinds on the 0005 data-driven explorer (one `KindSpec`, one row builder, one lazy watch each, list SSAR per kind, Events and YAML tabs for free, 0009 toolkit, 0012 counts):

- **NetworkPolicies**: rules rewritten as sentences, **Affects** pod count;
- **HPAs**: target link, min/max, metric bars, scaling events, AT MAX box;
- **ResourceQuotas**: usage columns and bars, **Blocked creations** from quota rejections; Namespace drawer **Quota** section;
- **PDBs**: allowed disruptions, **Selected pods**, BLOCKS DRAIN box (rule in the cluster crate for the 0034 drain preview).

## Non-goals

NetworkPolicy **Test traffic** and Topology (0023, 0022); any edit, New, Delete (disabled); PDB "Show selected pods" menu item (decision 14); LimitRanges; HPA links from the target's drawer; reverse NetworkPolicy lookup from a pod.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: four summaries and watches, `quantity_ratio`, `watch_failed_creates`, 4 `AccessCheck`s, 4 `ObjectKind`s, probe. **Run the UAT probe first** and fill [decisions.md](decisions.md) "UAT probe" | 1, 2, 3, 4 |
| 2 | App: shared model changes ([app-model.md](app-model.md)), **NetworkPolicies** and **PDBs** | 1, 2, 3, 5, 6, 7 |
| 3 | App: **HPAs**, **ResourceQuotas**, Namespace Quota section, related subjects; full ui-verifier run | 1, 2, 3, 5, 6, 7 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale, ceilings, UAT probe table |
| [cluster-api.md](cluster-api.md) | step 1: summaries, watches, access checks, probe |
| [app-model.md](app-model.md) | steps 2–3: kind specs, `KindObject`, W7 parts not rendered, diagnosis arms, joins, related subjects, menus |
| [network-policy-and-pdb.md](network-policy-and-pdb.md) | step 2: columns, cells, status, sections, sentences, BLOCKS DRAIN |
| [hpa-and-quota.md](hpa-and-quota.md) | step 3: columns, cells, status, sections, boxes, Namespace Quota |
| [files-to-touch.md](files-to-touch.md) | modules per step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. No `Cargo.lock` change.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. No kube or k8s-openapi type in a public signature; the crate spawns no task; the 0001 read-only grep finds only the SSAR `create`; new requests are `list`/`watch` only. No summary keeps annotations.
- [ ] 4. On UAT the probe prints 4 new watch lines, 4 new access lines, and 4 new count lines; results are in [decisions.md](decisions.md) "UAT probe". The AC7 credential script reports 0.
- [ ] 5. On UAT each allowed kind shows live rows and a drawer with Overview, YAML, Events tabs; a denied kind is disabled with "Not permitted: list …".
- [ ] 6. The 0003 AC4 color-literal grep is clean; bars use the kit `Progress` and `tone_color`.
- [ ] 7. The step's screenshots exist (empty states where UAT has no objects); the ui-verifier reports no high-severity defect against W7.

## Open items

1. LimitRange row of the Namespace Quota section needs a LimitRange watch (W7 shows "LimitRange none").
2. HPA scaling events show the API message as written; W7's `6 → 9` needs the previous size, which events do not carry.
3. Quota rejections are found by event text (`exceeded quota: {name}`); a localized or changed server message would hide them.
