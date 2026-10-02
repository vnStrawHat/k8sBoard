# 0012 · Drawer sections, links, Open URL

[Back to index](README.md) · Steps 2–4b · Modules: `*_rows.rs` (static sections and `Live` placeholders), `live_sections.rs` (new: paint-time content), `kind_drawer.rs`, `related_pods.rs`, `resource_actions.rs`

## Section order per kind (**new** in bold; `[Live]` = `DetailRow::Live`)

| Kind | Overview body, top to bottom |
|---|---|
| Deployments | subtitle gains `· rev {revision}` after the namespace (from `KindObject::Deployment`). WHY · Replicas · Selector · Containers · Ports · Conditions · **Revisions [Live]** · Pods · Labels |
| StatefulSets | Replicas · Selector · Containers · Ports · Volume claim templates (+ **Retention** row) · **Pods by ordinal** · Labels |
| DaemonSets | WHY · **Rollout by node** (renamed from Rollout; **Ready** and **Updated** `Bar` rows first, then the count fields) · **Not ready [Live]** · Node selector · Selector · Containers · Ports (` · host {n}`) · Pods · Labels |
| ReplicaSets | Replicas (Owner is a **Link**) · Selector · **Template** (replaces Containers: `pod-template-hash` → the label value, mono, or "—"; then `Image` → image when there is one container, else `{container}` → image per container) · Pods · Labels |
| Jobs | WHY · Status (Owner is a **Link**; **Active deadline** `{n}s`, **TTL after finish** `{n}s` rows) · Conditions · Containers · **Attempts** (pods) · Labels |
| CronJobs | **Next runs [Live]** · Schedule · Runs · **Recent jobs [Live]** · Containers · Labels |
| Services | WHY · Service · Ports · Selector · **Endpoints [Live]** · Labels |
| Ingresses | Ingress (Default backend a **Link** when a service) · Rules (backend a **StackedLink** when a service: the host and path above a link, because they are too long for the label column) · TLS · Labels |
| ConfigMaps | **Data [Live]** · **Used by [Live]** · Labels |

Links use `DetailRow::Link` (0008; `DetailRow::StackedLink` for the Ingress rules) with `ResourceKey::of_object("Service", ns, name)` and `of_owner`.

## Live content (`live_sections.rs`)

```rust
/// The rows of one live section. `None` from a lookup means "not loaded".
pub(crate) fn live_rows(content: LiveContent, row: &KindRow, live: &LiveCluster, now: Timestamp, cx: &Context<AppShell>) -> Vec<AnyElement>;
```

| Content | Source | Rows (max) | Empty / loading / failed |
|---|---|---|---|
| Revisions | related ReplicaSets owned by the Deployment (`owner` = Deployment/name) | `rev {n} · {tag}` · `{age} ago` + ` · current` (Warn when not all ready, else Ok) · `{ready}/{desired}`; others get a disabled **Roll back** ghost button; click → ReplicaSet (10, then `+n more`) | "No ReplicaSets" / "Loading revisions…" / "Revisions are unavailable" |
| NextRuns | `CronJobSummary.timetable` | [cron-schedule.md](cron-schedule.md) display (3) | suspended / error notes |
| RecentJobs | related Jobs whose `owner` is CronJob/name, newest `created_at` first | name (mono) · status toned (job tone) · ` · {duration}`; click → Job (10) | "No jobs kept" / "Loading jobs…" / "Jobs are unavailable" |
| NotReadyPods | owned pods not fully ready | `{node}` · Bad "node NotReady" when the node's readiness is not Ready, else the pod status label; click → Pod (20) | "All pods are ready" / pods notes as 0005 |
| Endpoints | `service_health` slices | one port: `{ip}:{port} · {pod}`; several: a "Ports" field `8080/TCP, 9090/TCP` then `{ip} · {pod}`; Ok "ready", Bad "not ready", Done "terminating"; click → Pod (50) | "No endpoints" / "Loading endpoints…" / "Not permitted: list endpointslices" or the error |
| UsedBy | `config_map_users` for this namespace and name | `{owner}` link · `{ways joined ", "}` (20); muted note "From pods in {scope}" | "Not used by any pod in {scope}" / pods notes |
| ConfigMapData | summary keys + related `ConfigMapValues` for this object | Immutable row (static); `{key}` → preview mono (`Line` as is, else `JSON · 412 B`, `text · 3 lines · 1.2 KiB`, `binary · 2.0 KiB`); before load: `{key}` → size (today's rows) | "No keys"; failed: size rows plus note "Values are unavailable: {error}" |

- Image tag: the first container's image after the last `:` that follows the last `/`; else the whole image.
- Revision order: numeric revision descending; a ReplicaSet without a revision sorts last. "current" = revision equal to the Deployment's.
- Roll back button: `Button::new(("roll-back", ix)).label("Roll back").xsmall().ghost().disabled(true).tooltip("Read-only mode")`.
- DaemonSet bars: `Bar { label, percent: percent(done / total) (0 when total is 0), text: "{done} / {total}", tone: Some(replica_tone(done, total)) }`; renderer in [row-model.md](row-model.md).

## Pods section changes (`related_pods.rs`, step 3)

The existing private `PodRowDetail` grows two variants and a lifetime; no new type. `pods_section(owner, object: &KindObject, live, cx)` picks it with `fn pod_row_detail<'a>(owner: &PodOwner, object: &'a KindObject) -> PodRowDetail<'a>` (pure, tested); the node drawer passes `&KindObject::Plain`. Title and order follow the detail:

| `PodRowDetail` | Used by | Title | Order | Row detail |
|---|---|---|---|---|
| `StatusOnly` (existing) | Deployments, ReplicaSets | `Pods {n}` | snapshot | status |
| `StatusAndNode` (existing) | DaemonSets | `Pods {n}` | snapshot | status · node |
| `NamespaceAndStatus` (existing) | Node drawer | `Pods {n}` | snapshot | namespace · status |
| `StatusAndClaims(&'a [ClaimTemplate])` (new) | StatefulSets | `Pods by ordinal {n}` | ordinal | status · `{claim} {storage}` for each PVC mount named `{template}-{pod}` |
| `Attempt` (new) | Jobs | `Attempts {n}` | newest first | status · `exit {code}` of the first main container's current or last termination |

## Menus (`resource_actions.rs`)

| Kind | Item | Rule |
|---|---|---|
| ReplicaSets (step 3) | **Go to owner** after View YAML | enabled when `of_owner` gives a key; else disabled "No owner" |
| Ingresses (step 4b) | **Open URL** after View YAML | `ingress_urls(summary)`: none → disabled "No host to open"; one → item; several → submenu of URLs (10) |

`ingress_urls` (pure, `network_rows.rs`): for each rule with a host (skip hosts starting with `*`), host must be ASCII letters, digits, `.`, `-`; path kept only when it is made of `A–Z a–z 0–9 / - _ . ~`, else `/`; scheme `https` when a TLS entry lists the host or a matching `*.suffix`; deduped in rule order. Opening: `cx.open_url(&url)`; nothing is logged but the host.
