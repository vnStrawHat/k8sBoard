# 0007 · Decisions

[Back to index](README.md). Defaults chosen by the architect; C1 is the orchestrator decision in force.

## Data, masking, and the cluster crate

| # | Decision | Rationale |
|---|---|---|
| 1 | One-shot **GET as `DynamicObject`**, keyed by a new crate enum `ObjectKind` mapped to `ApiResource::erase::<K>()` | one code path for every kind; keeps unknown fields like `kubectl get -o yaml`; 0016 (Secret) and 0018 (CRDs) add a variant or an `ApiResource`, not a new method |
| 2 | **Masking and serialization happen in the cluster crate**, on tokio, before anything reaches the app. The app only ever sees masked YAML text | the strongest C1 guarantee: no raw object or unmasked value crosses the crate boundary; same split as summaries that drop values (0005 decisions 6–7) |
| 3 | **YAML serializer: `serde-saphyr` 1.3** (pure Rust, maintained). Already in `Cargo.lock` at 1.3.0 (via gpui-kit's i18n) and 0.0.29 (via kube-client), so no new crate is downloaded | `serde_yaml` is deprecated; `serde_yml` has a poor maintenance record; `serde_norway`/`serde_yaml_ng` would be new crates with `unsafe` libyaml ports |
| 4 | `serde_json` becomes a direct dependency of the crate (already in the tree via kube) | the masking walk works on `serde_json::Value`; `Value::sort_all_objects` gives kubectl's alphabetical key order whatever `preserve_order` feature unification decides |
| 5 | Always strip `metadata.managedFields`. **No toggle** in the drawer | the drawer wireframe hides it with no control; W10 (0031 Edit YAML) owns the "Hide managedFields" toggle; managedFields hold field paths only, so this is about noise, not secrets |
| 6 | **Keep `status`** | read-only triage needs conditions and container states; matches `kubectl get -o yaml` and the roadmap entry (the wireframe comment "status hidden" is W10 editor behavior) |
| 7 | Mask the values of the manifest annotations (`kubectl.kubernetes.io/last-applied-configuration`, `kapp.k14s.io/original`, `kapp.k14s.io/original-diff`) in every `metadata.annotations` at any depth, so pod and job templates are covered | each embeds a whole applied manifest, including Secret `data` and env literals; templates copy annotations from the applied object |
| 8 | Any object whose `kind` is `Secret`: every value under `data` and `stringData` is masked; keys stay. **ConfigMap `data`/`binaryData` are shown on purpose** | C1; keyed on the response's `kind`, not on `ObjectKind`, so 0016 and Helm release Secrets (0017) are masked with no extra code. C1 treats ConfigMaps as non-secret (kubectl shows them); 0005 kept values out of the summaries for memory, not secrecy, and here they are fetched on demand for one object. Step 3 changes the 0005 drawer note "Values are not shown in this version" to "Values are in the YAML tab" |
| 9 | **Env literals are masked by default**: `env[].value` of every item in any `containers`, `initContainers`, or `ephemeralContainers` array under `spec` (any depth, so Pod, workload templates, and CronJob job templates). `valueFrom` refs stay | env literals often hold credentials; C1 shows env as names and sources only (0008), so the YAML view must not undo that; screenshots of pods stay credential-free |
| 10 | A per-view **"Env values" toggle** refetches with env shown. It lasts until toggled off, a subject or tab change, or drawer close. No 30 s timer | without it env values are visible nowhere in the app, unlike kubectl; env literals are not Secret objects, so the C1 per-key 30 s reveal does not apply; a timer would refetch and reflow the editor under the reader |
| 11 | **No reveal of Secret data or manifest annotations in the YAML view.** Per-key 30 s reveal lives in the 0016 Secret Overview "Data" section | per-key reveal inside a code editor needs span mapping; never holding raw values in the view is simpler and safer. Copy always copies what is shown |
| 12 | Every masked value becomes the plain string `<hidden>`. When at least one value is hidden, the YAML starts with `# k8sBoard hid {n} values as <hidden>.` | one marker; invalid base64, so a copied Secret cannot be applied by accident; the comment survives Copy |
| 13 | Raw objects never reach `tracing`, `Debug`, disk, or errors: serializer failures map to a fixed message, and `ObjectYaml` has no `Debug` derive | C1, R1 |

## RBAC and lifecycle

| # | Decision | Rationale |
|---|---|---|
| 14 | **No new access check for `get`.** View YAML is always enabled; a 403 shows inline in the tab | the drawer only opens when `list` is allowed, and roles grant get/list/watch together in practice; 13 more SSARs per scope change cost more than a rare inline error (same stance as 0005 decision 8, 0006 decision 7) |
| 15 | Fetch when the YAML tab is shown, never in the background. Leaving the tab, changing subject, or closing the drawer drops the view, its text, and any running request | C1 "fetched on demand, in memory only"; one request in flight at most |
| 16 | No auto-refresh from watch events. A Refresh button and "Fetched 12s ago" (age read at paint time, like `KindCell::Age`) | the roadmap asks for one-shot; a watch per drawer would be a second live object store |
| 17 | The view is created in `AppShell::render` (`sync_yaml_view`), like `open_pending_logs`, under two invariants ([yaml-view.md](yaml-view.md)): the sync never notifies, and `YamlView::new` never notifies synchronously | `EditorState` needs a `Window`; `change_selection` and `reveal` have none; one sync point covers every transition; a notify during render would loop |
| 17a | The first fetch of a view waits `DRAWER_SUBJECT_DELAY` (250 ms, shared with 0006's object-events debounce, moved to `drawer.rs`); Refresh and the env toggle start at once | arrowing through rows on the YAML tab must not send one GET per row; explicit clicks must feel immediate |
| 17b | The env toggle flips only when its refetch succeeds | the button always describes the text on screen |

## View and tabs

| # | Decision | Rationale |
|---|---|---|
| 18 | The code view is the **kit `Editor`, read-only**, `text_xs`, with `gpui-kit` feature `tree-sitter-yaml` (adds `tree-sitter`, `tree-sitter-yaml`, `tree-sitter-json`, `tree-sitter-language`, all C built with `cc`) | virtualized rows, selection and Ctrl+C, line numbers, folding, and Ctrl+F search for free; highlight colors come from the theme's `highlight_theme`; W10 and the stack table name this editor, so 0031 reuses it. The logs `MessageScroller` has no text selection or search |
| 19 | Search = the editor's built-in Ctrl+F panel; no extra UI | zero code |
| 20 | Tabs everywhere, in wireframe order without Monitor (0010): Pod `Overview · Containers n · YAML · Events n`; Node and kinds `Overview · YAML · Events n`; an event's own drawer `Overview · YAML` | W4, W7; 0006's Node and kind Events **sections move into the Events tab** |
| 21 | One `DrawerTab` enum replaces `PodDrawerTab`. The tab survives a subject change inside a screen; `show_screen` resets it to Overview | arrowing through rows keeps the tab (0003); a new screen or a `reveal` starts on Overview |
| 22 | Node and kind drawers get the ⤢/⤡ expand toggle | YAML lines are long; 640 px is the existing expanded width |
| 23 | "View YAML" is added to the Pod and Node menus (view group, last) and enabled everywhere. It opens the drawer on the YAML tab. The `Y` key binding waits for 0028 | W4/W5/W7 menus; the keyboard map has its own spec |

## Known ceilings

- **Editor size.** The kit editor documents a limit of about 50K lines. Real objects stay far below it (the API server caps an object near 1.5 MiB; the largest UAT node YAML is measured in the step 3 live check). Upgrade path: plain text or a truncation notice above the limit.
- **Session identity.** `sync_yaml_view` compares only `ObjectRef`. This relies on the invariant that every context or namespace switch closes the drawer first (`start_session`, `set_namespace`); a future switch path must keep it.
- **Large objects** (ConfigMaps near 1 MiB, nodes with many images) are parsed by tree-sitter on the main thread once per fetch. Measure on UAT (live check); upgrade path: plain text above a size limit.
- **Refresh resets the scroll** to the top (`set_value`). Keeping the position needs the kit's scroll API.
- `command`/`args` literals are not masked (open item 2).
