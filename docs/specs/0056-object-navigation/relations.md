# 0056 · Relations: Ingress -> Service -> Pod, and the Pod drawer (H4, M16)

[Back to index](README.md)

## Data sources and cost

| Feature | Source | New API load |
|---|---|---|
| Ingresses table `Backends` column | `IngressSummary.rules[].service`, `default_service` (already in the row) | none |
| Service drawer `Exposed by` | new related subject `ServiceIngresses { namespace }`: `watch_ingresses(Named(ns))` | **1 watch** (ingresses of one namespace) while a Service drawer is open, debounced like the other drawer watches |
| Endpoints pod links | existing endpoint slices companion | none (style only) |
| NetworkPolicy `Pods` section | `live.pods` (always watched) + `policy.pod_selector` | none |
| Pod `Deployment` row | pod label `pod-template-hash` + controller ReplicaSet name | none |
| Pod `Service account` link, `Labels`, `Volumes` | `PodSummary` (service_account, labels, containers[].mounts) | none |
| Pod `Services` section | new related subject `PodServices { namespace }`: `watch_services(Named(ns))` | **1 watch** (services of one namespace) while a Pod drawer is open |
| Env / Mounts Secret and PVC links | `container_detail.rs` targets | none |

Both new related subjects are derived from the drawer **key** alone (namespace, name), not from the explorer row, so they also start over Topology. `selected_related_subject` matches `ResourceKey::Pod` and `Kind { Services, .. }` before its row lookup. `denied_related_check`: `ListServices` / `ListIngresses`; denied shows `Not permitted: list services` / `list ingresses` in the section. A `RelatedList::Services` / `RelatedList::Ingresses` variant each; the update enum gets the matching arms.

No Endpoints screen: EndpointSlices belong to a Service, and the Service drawer's Endpoints section plus the Services table `Endpoints` column already answer "which pods serve this". Open question Q4.

## Ingresses table: `Backends` column (0005 kind columns)

- Columns become `Class, Hosts, Backends, Address, TLS, Age`; `Backends` 160 px, left.
- Cell text: distinct backend service names in rule order, then the default backend's service, joined `, ` (no ports: the drawer's Rules section has them). A resource backend shows `Kind/name`. None: absent (`—`).
- Built in `ingress_row` (`network_rows.rs`); pure helper `ingress_backends(&IngressSummary) -> Vec<String>`, reused by `Exposed by`.

## Service drawer: `Exposed by` (after `Endpoints`)

- Section `Exposed by`, a `DetailRow::Live(LiveContent::ExposedBy)` placeholder from `service_row`.
- Rows: the Ingress name as a link (`follow_link`), then muted the routes that reach this Service: `api.example.com/v1, /health`; the default backend reads `default backend`. Pure `exposing_ingresses(service, ingresses) -> Vec<ExposingIngress { name, routes }>`, matching `rule.service == service.name || default_service == service.name`, same namespace.
- Notes: `Loading ingresses…`, `No ingress routes to this service`, `Ingresses are unavailable` + message, `Not permitted: list ingresses`. Cap 20 rows, then `+N more`.

## NetworkPolicy drawer: `Pods` section (after `Applies to`)

- `DetailRow::Live(LiveContent::SelectedPods)` from `network_policy_row`; `live_rows` gets the `(SelectedPods, NetworkPolicy)` arm.
- Generalise the PDB code: `selected_pods_rows(namespace, selector, live, cx)` so both kinds share it (same notes: `Loading pods…`, `No pods match`, `3 pods · 2 healthy`, cap, `+N more`). A policy that selects everything lists the namespace's pods.

## Pod drawer Overview (M16)

Order: WHY box, `Pod` section, Conditions, Containers, then the new `Services`, `Volumes`, `Labels`.

| Row / section | Content |
|---|---|
| `Service account` (existing row) | link to the ServiceAccount in the pod's namespace |
| `Controlled by` (existing) | unchanged: `ReplicaSet/api-7d9f8c` |
| `Deployment` (new row, under Controlled by) | link `api`, only when `deployment_of_pod` resolves: controller kind `ReplicaSet`, pod label `pod-template-hash=H`, ReplicaSet name `D-H`, `D` non-empty. Only the Deployment controller sets that label, so no list is needed; an orphaned ReplicaSet leads to a Deployments screen without the row (normal reveal drop). Lives in `kind_row.rs` beside `is_deployment_replica_set`. |
| `Services` section | services of the pod's namespace whose selector matches the pod's labels (reuse `Selector::of_labels`, same rule as `matching_pods`; selector-less services never match). Row: Service name link, muted `ClusterIP · 80/TCP, 443/TCP`. Notes: `Loading services…`, `No service selects this pod`, `Services are unavailable`, `Not permitted: list services`. |
| `Volumes` section | one row per volume that a container mounts, first-seen order, deduped by volume name: label = volume name, value = source. `configmap/x`, `secret/y`, `pvc/z` are links; `emptyDir`, `hostPath /p`, `downwardAPI`, `projected` (+ its config map and secret names, text), `volume` stay text. Note under the title: `Volumes no container mounts are in the YAML tab` only when the pod has none listed. |
| `Labels` section | `chips("pod-labels", &pod.labels)` (copyable, as the kind drawers do). |

Annotations: not shown (Q3). `PodSummary` never reads them; adding them to a 3,000-pod watch costs memory for one drawer. The YAML tab has them.

## Container detail (Env, Mounts)

- `config_map_target` becomes `source_target(kind, namespace, name)`; Secrets (`secret/x`, envFrom secret, secret key ref) and PVC mounts get targets. Remove the stale comment "Secrets and claims have no screen yet".
- Links open with `follow_link`, so a denied Secrets list shows the notification instead of an empty screen.
