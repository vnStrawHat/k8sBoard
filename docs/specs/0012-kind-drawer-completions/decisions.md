# 0012 · Decisions

[Back to index](README.md). Architect defaults; the user asked not to stop for questions.

## Data sources

| # | Decision | Rationale |
|---|---|---|
| 1 | **Cron: own port** of robfig/cron v3 `ParseStandard` and `Next` in `cluster::cron_schedule`. No cron crate | the answer must equal the controller's: robfig's `Next` skips a spring-forward hour and repeats a fall-back hour, sets the star flag for `*` and `?` but not `*/n`, and ORs restricted day fields. `cron` (seconds field, AND rule), `croner` (L/W/#, its own DST and day rules), `saffron` (unmaintained) each differ somewhere, and a port needs no second time library next to jiff. The grammar is five small fields; tests pin each rule |
| 2 | **Time zones**: jiff feature `tzdb-bundle-always` (adds `jiff-tzdb`) | Windows has no system tzdb; one bundled database gives the same answer on all three CI runners. Cost: ~100 KiB, and rules can lag an OS update until our next release |
| 3 | Unset `timeZone` → UTC (`zone_name: None`), shown as "Cluster default (UTC assumed)" | the controller-manager's local zone is not visible through the API; managed clusters run UTC |
| 4 | `CronJobSummary.timetable` is parsed in the summarizer; the next run is computed **at paint time** | parse once on tokio; a paint-time `next_after(now)` never goes stale (µs per row) |
| 4a | `@every <d>` is supported: next = anchor (`lastScheduleTime`, else `creationTimestamp`) + k·delay; h/m/s integer groups only | robfig `ConstantDelaySchedule`; a few lines |
| 5 | **Endpoints**: one EndpointSlice watch per Services screen, same scope, alive only while Services is the explorer kind | the column needs every service, so a per-drawer list cannot fill it; slices are small once summarized |
| 6 | No core/v1 Endpoints fallback; denied slices give "—" cells and a drawer reason | Endpoints is deprecated (1.33); one source |
| 7 | Endpoint counting: skip `FQDN` slices and slices without the `kubernetes.io/service-name` label; dedupe by pod (else address); exclude terminating from the total. Nil `ready` reads true, nil `terminating` false (API doc); `serving` is intentionally unused (only `ready` decides traffic for a non-terminating endpoint) | dual-stack services would count twice; unlabeled slices belong to no service |
| 8 | **Used by** comes from live pods (env, envFrom, mounted volumes, projected sources), mapped to the top owner | W7 note "computed from pod spec"; no extra watch |
| 9 | Owner mapping: ReplicaSet `{d}-{hash}` → `deployment/{d}` (0005 hash rule); Job `{c}-{≥8 digits}` → `cronjob/{c}`; other controllers as is; none → `pod/{name}` | controller naming conventions; no extra lookup |
| 10 | `VolumeSource::Projected` carries its config map names | otherwise `kube-root-ca.crt` reads "unused" in every namespace |
| 11 | **Related watch**: one drawer-scoped watch per session, started with the object events watch by one `follow_drawer_subjects` after one 250 ms selection rest: Deployment → ReplicaSets by its label selector; CronJob → Jobs of its namespace; ConfigMap → that ConfigMap's value previews | live rollout progress; small server-filtered lists; one slot bounds memory |
| 12 | Revisions only for Deployments (from ReplicaSets); no ControllerRevisions | W7 shows revisions only on Deployments; no new RBAC or data |
| 13 | ConfigMap previews: a single line ≤ 120 chars as written, else `JSON · 412 B` or `text · 3 lines · 1.2 KiB`; binary → `binary · 2.0 KiB` | W7 Data shows values; 0005 decision 6 keeps list summaries value-free |
| 14 | One new access check, `ListEndpointSlices`; ReplicaSets, Jobs, ConfigMaps reuse their list checks | same rule as 0005 decision 8 |
| 15 | **Sidebar counts (C11)**: one `list` with `limit=1` and **no resourceVersion** per allowed kind and namespace, reading `remainingItemCount`; started by `finish_connect` and `finish_access_review` once per scope, then on navigation at most every 30 s from the start of the last run; 4 requests in flight (`buffer_unordered(4)`); a live list's count wins; unknown → no count | ~13 tiny GETs, no watch; C11 default. With `resourceVersion=0` the server answers from its watch cache, ignoring `limit` and sending every object without `remainingItemCount`; the review result tells which kinds are denied |

## App structure

| # | Decision | Rationale |
|---|---|---|
| 16 | `KindRow` keeps its source summary (`KindObject`) | WHY rules, related subjects, URLs, and menus need typed fields; summaries of these kinds are small |
| 17 | Sections may hold `DetailRow::Live(LiveContent)`, rendered at paint time from `KindObject` and the live lists | keeps the W7 section order; C7 lazy content; one renderer |
| 18 | **Cross-list cells** (Service Endpoints and status, ConfigMap Used by, Namespace Pods/CPU req/Memory req) are joined **on the main thread** into the explorer rows after each relevant update (`kind_join.rs`) | pods live on the main thread; a second pods watch on tokio would double the largest list; O(rows + pods) per 100 ms batch fits the 0009 budget |
| 19 | WHY boxes for Deployments, DaemonSets, Jobs, Services only, and only when a cause is found | roadmap list; no box during a normal rollout |
| 20 | Pod causes reuse 0008 `pod_diagnosis` with no events | one rule set; probe detail stays in the pod drawer |
| 21 | StatefulSet claims come from each pod's PVC mounts plus the template's storage; no PVC watch | mounts name the real claims; PVC status is 0014 |
| 22 | DaemonSet "Rollout by node" bars are `DetailRow::Bar` rows on the kit `Progress` (`value` 0–100, `color` = `tone_color`); `Bar` carries a percent, text, and optional tone | W7 section name; theme tokens only; 0013–0015 reuse the same row for quotas, usage, and HPA metrics, so it is general from the start |
| 23 | Job Attempts = the Job's pods, newest first, with the exit code; it replaces the Pods title | W7 Jobs; no new data |
| 24 | Disabled **Roll back** button on older revisions, tooltip "Read-only mode" | same pattern as Forward (0005 decision 20) |
| 25 | **Open URL**: `https` when a TLS entry covers the host (wildcards too), else `http`; plain paths only; wildcard hosts skipped; one item, or a submenu for several URLs (max 10); `cx.open_url` | local action, allowed; never another scheme |
| 26 | Links: ReplicaSet and Job owners, Ingress backends → Service, Revisions → ReplicaSet, Recent jobs → Job, Endpoints and Not ready → Pod, Used by → workload | W7 "related objects" |
| 27 | `reveal` clears the target table's filter (text, chips, preset) when it hides the target; clearing a filter also unticks the checked rows (`clear_checked`) | ReplicaSets start with Hide inactive (0009 decision 26), so a revision link would close at once (0009 decision 12) |
| 28 | Namespace columns: Pods = all pods in the namespace; requests = main + sidecar containers of pods whose tone is not Done (0010 rule); namespaces outside the scope read "—" | no extra watch; same rule as node requests |
| 29 | New launch screens: none; settle waits for the related watch like object events. ConfigMap screenshots on UAT use only `--filter kube-root-ca.crt` | `<kind>` and `<kind>-drawer` exist for every kind; `--filter` picks a row; that ConfigMap holds a public CA, so previews in screenshots expose nothing |
| 30 | W7 names: DaemonSet "Rollout by node", ReplicaSet "Template" section (hash, image), Deployment subtitle `· rev {n}` | match the wireframe |

## Supersedes

0005 decision 24 (Next run deferred), kind-columns deviations for Namespaces, CronJobs, Services, ConfigMaps (columns added), 0005 open items 1–4, 0009 open item 1.

## Known ceilings

- Joins rerun on every pods batch while Services, ConfigMaps, or Namespaces is shown. Measure with the 0009 `rebuild_view` trace before optimizing.
- Used by misses workloads without pods (README open item 1); owner names are conventions, not lookups.
- With N picked namespaces the Services screen runs N slice watches.
- `remainingItemCount` can still be absent (e.g. a future server serving paginated lists from its cache); those kinds show no count.

## UAT probe (step 1b, `readonly@Monitor`, Kubernetes v1.29.5)

| Check | Result |
|---|---|
| `list endpointslices` | allowed |
| endpointslices watch, `--watch-seconds 5` | `watch endpointslices: 1 snapshots, last 67 items, 0 failures` |
| `--counts` (13 kinds) | pods 104, nodes 4, namespaces 20, events 10, deployments 31, statefulsets 15, daemonsets 7, replicasets 99, jobs 5, cronjobs 0, services 71, ingresses 16, configmaps 88; none unknown or denied. Each equals the matching watch snapshot count, so `remainingItemCount` is present on v1.29.5 |
| `cron jobs` line (step 1a) | `last 0 items`: UAT has no CronJobs, so the `next ...` note is unit-tested only |
