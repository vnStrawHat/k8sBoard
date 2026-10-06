# 0005 · Kind drawers

[Back to index](README.md) · Module: `kind_drawer.rs` (renderer); the sections come from the `*_row` builders

## Frame (reuses `drawer.rs`)

- `drawer_frame(header, None, body, DRAWER_WIDTH, cx)`: there is no tab bar (Monitor, YAML, and Events are deferred) and no expand toggle (`expand: None`).
- Header:
  - `kind_badge: kind.badge()`, the name, the ⋯ menu ([navigation-and-actions.md](navigation-and-actions.md)), and ✕;
  - subtitle: `toned_text(row.status)`, then muted text `· {namespace}` (namespaced only) and `· created {age} ago` (`created_text`).
- `kind_menu_button` re-reads the row from `live.explorer` by key when it opens, like `node_menu_button`.
- Body order: `row.sections` in order, then **Pods** (if `related_pods` is set), then **Labels** (always, chips or "—").
- `drawer.rs` change: `detail_row(label: impl Into<SharedString>, …)` instead of `&'static str`, because keys and condition names are dynamic.

## Rendering `DetailRow`

| Row | Element |
|---|---|
| `Field { label, value }` | `detail_row(label, cell)`. `Text` and `Mono` are `truncated_text` with a tooltip. `Age` reads `{timestamp} ({age} ago)`, like the node drawer's Created. `Duration` uses `format_age` |
| `Chips(terms)` | wrapping `h_flex` of chips: `px_1p5`, `rounded(theme.radius)`, `bg(theme.muted)`, mono `text_xs`. "—" when empty |
| `Port { text }` | `h_flex`: the text (mono, flex_1, truncated), then the **Forward** button (below) |

## Forward button (decision 20)

```rust
Button::new(("forward", index)).label("Forward").icon(Icon::new(IconName::ArrowLeftRight /* Lucide arrow-left-right */))
    .xsmall().ghost().disabled(true).tooltip(reason)
```

- `reason` is `action_availability(ResourceAction::PortForward, &live.access)`, which is always `Disabled`:
  - on UAT: "Not permitted: create pods/portforward";
  - when allowed: "Not available in read-only mode".
- The kit applies `tooltip` to disabled buttons (checked in `gpui-component` 0.7 `button.rs`).

## Sections per kind (built by the row builders)

| Kind | Sections, in order. `Field` labels; `[chips]`; `{ports}` |
|---|---|
| Deployments | **Replicas**: Desired, Ready, Up-to-date, Available, Strategy (`RollingUpdate · max surge 25% · max unavailable 25%`), Revision, Paused (only when paused). **Selector** [chips]. **Containers**: `{name}` → image (Mono). **Ports** {`{container} · {name} {port}/{protocol}`}, omitted when there are none. **Conditions**: `{type}` → Toned `True` (Ok) or `False · {reason}` (Warn) |
| StatefulSets | **Replicas**: Ready and Up-to-date bars of Desired, then Desired, Current, Service, Strategy, Pod management. **Selector**. **Containers**. **Ports**. **Volume claim templates**: `{name}` → `{storage} · {class} · {modes}` (absent parts skipped) |
| DaemonSets | **Rollout**: Ready and Up-to-date bars, then Desired, Current, Available, Misscheduled (only when > 0, Warn), Strategy. **Not ready** (the pods that are not ready; the whole section is hidden once the loaded pods are all ready). **Node selector** [chips]. **Selector**. **Containers**. **Ports**. No Conditions section (the controller never writes any) |
| ReplicaSets | **Replicas**: Desired, Current, Ready, Owner (`deployment/api` format), Revision. **Selector**. **Containers** |
| Jobs | **Status**: Status (toned), Completions, Parallelism, Active, Succeeded, Failed (Bad when > 0), Backoff limit, Started (Age), Duration, Owner. **Conditions**. **Containers** |
| CronJobs | **Schedule**: Schedule (Mono), Time zone (or "Cluster default"), Suspend, Concurrency policy, Starting deadline (`{n}s`), History limits (`{s} succeeded · {f} failed`). **Runs**: Last schedule (Age, toned), Last success (Age), Active jobs (names joined, or "—"). **Containers** |
| Services | **Service**: Type, Cluster IP, External, Selector-less note ("No selector: endpoints are managed manually") when the selector is empty. **Ports** {`{name} {port} → {target}/{protocol}`, plus ` · node {nodePort}`}. **Selector** [chips] |
| Ingresses | **Ingress**: Class, Address, Default backend. **Rules**: `{host or *}{path}` → backend (Mono). **TLS**: `{hosts joined}` → `secret {name}` |
| ConfigMaps | **Data** `{n} keys`: `{key}` → `{format_bytes}`, plus ` · binary` when binary; "No keys" when empty. Immutable (only when true). A muted note "Values are not shown in this version" after the list |
| Namespaces | **Namespace**: Status (toned), Created (Age) |

- `format_bytes(n)`: `412 B`, `2.0 KiB`, and `1.1 MiB` (1024-based, one decimal at KiB and above).
- Workloads' **Containers** list only main containers of the template, in spec order (`TemplateContainer`: name, image, ports; never env or args).
- Step 2 implements the Namespaces and Deployments rows of this table, plus the shared renderers. Step 3 adds the rest.

## Pods section (decision 19)

- Shown for `related_pods: Some(owner)`: Deployments, StatefulSets, DaemonSets, ReplicaSets, and Jobs. The pods are `live.pods.items().filter(owns_pod)`, read at render time, so they stay live.
- Title: `Pods {n}`. If `live.pods` is `Loading`: "Loading pods…". If it is `Failed`: "Pods are unavailable". If n = 0: "No pods".
- Order: StatefulSets by ordinal, using the pure `fn ordinal(pod_name, set_name) -> Option<u32>` (the `-{n}` suffix; `None` sorts last). The others keep the snapshot order (name).
- Each row (`id(("related-pod", ix))`, `cursor_pointer`, hover `theme.muted`) shows: the name (mono, truncated) · `pod_status_label` toned · the node, muted, for DaemonSets.
- Click: `AppShell::reveal_pod(ResourceKey::of_pod(pod), cx)` ([navigation-and-actions.md](navigation-and-actions.md)).
- At most 50 rows are shown, then a muted `+{n} more`. That bounds render cost on large DaemonSets.
