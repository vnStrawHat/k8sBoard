# 0005 · Decisions

[Back to index](README.md). The architect chose these defaults; the user asked not to stop for questions.

## Data and crates

| # | Decision | Rationale |
|---|---|---|
| 1 | Batch 1 is the ten kinds in the README. Secrets are deferred | daily triage value; a Secret watch moves values into memory, so it needs the pending secret rules |
| 2 | One `*Summary` type per kind in `crates/cluster`, built from k8s-openapi types like `PodSummary` | keeps kube types private and the conversions unit-testable; the app gets plain data |
| 3 | Kubernetes semantics live in the cluster crate (`JobStatus`, selector text, port text). Tones and wording live in the app | the same split as `PodStatus` and `StatusTone` |
| 4 | `PodController` is renamed `ControllerRef` and moves to `workload.rs` | ReplicaSets and Jobs need the same owner type; per the style guide, no compatibility alias |
| 5 | One module and one `watch_<kind>s(scope)` method per kind (CronJob in `cron_job.rs`, separate from `job.rs`) through a private `scoped_api::<K>(scope)` that replaces `pods_api` | matches the 0002 API; one generic helper and no duplicated `Api` construction |
| 6 | Config map summaries keep key names and byte sizes, never values | the 0002 memory bound (values can be 1 MiB each) and the pending content rules |
| 7 | Labels are shown in every new drawer. Annotations are never kept. Template containers keep name, image, and ports only (never env, envFrom, command, args, volumeMounts) | annotations can hold `last-applied-configuration`, and env and args can hold plaintext secrets |
| 8 | Only `list` is checked per kind (10 new `AccessCheck` variants; `ALL` grows to 19). `watch` is not checked | RBAC grants list and watch together in practice; a denied watch already shows the list error state |

## App structure

| # | Decision | Rationale |
|---|---|---|
| 9 | One data-driven row model, `KindRow` (cells plus drawer sections), built by one pure `*_row` function per kind | one table delegate and one drawer renderer serve every kind; the builders are testable without GPUI. Ceiling: see "Known ceilings" |
| 10 | No trait. Per-kind variation is data (`ResourceKind` tables) plus one `match` in `ResourceKind::watch_rows` | the style guide: traits only when generic behavior needs them; enum data covers this |
| 11 | Rows are built inside the stream on tokio (`map` before `subscribe`) | the main thread only swaps a `Vec`, as for pods |
| 12 | **Lazy watch**: one explorer watch for the visible kind. It starts on navigation, is replaced on a kind switch, and is dropped when leaving to Pods or Nodes. There is no cache | memory stays bounded at 4 watches per session; a re-list takes well under a second |
| 13 | Namespaces get their own explorer watch, separate from the namespace picker's | one code path for every kind; about 20 objects |
| 14 | A namespace switch restarts the explorer watch only for namespaced kinds | Namespaces are cluster-scoped |
| 15 | One `TableState<KindTableDelegate>` entity is shared by all kinds. Its columns are rebuilt on a kind switch | one entity and one subscription instead of ten |
| 16 | Name is always the first, flexible column, with the namespace prefix muted as in Pods | consistent look; one width rule |

## Behavior

| # | Decision | Rationale |
|---|---|---|
| 17 | Columns follow `kubectl get` (wide where useful). Wireframe-only columns that need more data are dropped ([kind-columns.md](kind-columns.md)) | familiar to DevOps; no extra watches |
| 18 | Superseded by spec 0007: every drawer has a tab bar and the expand toggle | the 0006 Events and 0007 YAML tabs need them |
| 19 | Related pods come from the existing pods watch, matched by the controller owner reference. For a Deployment, the owner is a ReplicaSet named `{deployment}-{hash}`. A click opens the pod on the Pods screen | no new watch; the pod-template hash never contains `-`, so the match is exact |
| 20 | Each port row has a disabled **Forward** button whose tooltip is the port-forward gate reason | user decision; UAT denies `create pods/portforward` |
| 21 | Sidebar items are disabled only for `Known` plus denied. `Checking` and `Unknown` stay enabled | does not block navigation during the review; the error state still explains a 403 |
| 22 | Menus: W7 mutating items disabled with "Read-only mode"; "View YAML" on every kind, disabled with "YAML view comes in a later version"; Copy name; Delete last. Port-forward ▸ is gated for Deployments, StatefulSets, and Services | user decisions; users learn what exists; one wording for YAML across kinds |
| 23 | ReplicaSets with desired 0 are shown (muted), not hidden | there is no filter UI yet ([README](README.md) open item 5) |
| 24 | CronJob "Last schedule" is toned by outcome. "Next run" is deferred | no cron parser dependency in this spec |
| 25 | Screenshot screens: `<kind>` and `<kind>-drawer` for every kind. The drawer opens on row 0 | deterministic and simple |
| 26 | Owner cells and fields use kubectl's `{kind lowercased}/{name}`, e.g. `deployment/api` | matches `kubectl` output and W7 |
| 27 | App row modules by concept: `workload_rows.rs`, `batch_rows.rs` (Job and CronJob), `network_rows.rs`, `config_map_rows.rs`, `namespace_rows.rs` | one primary concept per module (style guide) |

## Implementation amendments (steps 2 and 3)

| # | Decision | Rationale |
|---|---|---|
| 28 | Port and Service-port text is port-first: `{port}/{protocol} · {name} · {container}`, `{port} → {target}/{protocol} · {name} · node {n}` | one truncated line with a tooltip still shows the port |
| 29 | Drawer labels are 136 px and truncate with a tooltip. `DetailRow::Stacked` (label above value) is used for ingress hosts and TLS hosts, never for ConfigMap keys | hosts are too long for a label column; a key must keep its size beside it |
| 30 | A ConfigMap drawer's section title is the static "Data". The key count is in the subtitle ("4 keys") | section titles are `&'static str` |
| 31 | `DetailRow::Note` is muted wrapping text: "No keys", "Values are not shown in this version", the selector-less Service note | explanations are not label and value pairs |
| 32 | ReplicaSet and DaemonSet "Ready" cells are a toned number, not `{ready}/{desired}` | Desired is its own column there |
| 33 | `--screen <kind>-drawer` settles when its row is selected or when the launch request found no row to select (`SettleInput.is_drawer_ready`) | an empty list previously waited out the 30 s timeout |
| 34 | A CronJob whose last run failed is Warn in both the "Last schedule" cell and the subtitle (the spec's cell said Bad) | one tone per state; the CronJob itself is healthy |

## Known ceilings

- **ConfigMap watch payload (decision 6).** The summary keeps no values, but the watcher still deserializes full objects transiently, up to the watcher page size: kube's default list page is 500 objects, and a config map can be up to 1 MiB. Upgrade path, only if memory spikes are measured: set a smaller `watcher::Config::page_size` for config maps. A metadata-only watch would lose the key names.
- **Pre-built drawer sections (decision 9).** Every snapshot rebuilds the sections for every row, so the cost is O(rows × section size) per batch. That is fine for this batch (hundreds of rows with small sections). It is **not** the pattern for high-churn or sensitive kinds:
  - Events: thousands of rows, frequent changes;
  - Secrets: values must never be pre-rendered.

  Those specs must build drawer content lazily from the selected summary instead of inheriting `KindRow.sections`.
