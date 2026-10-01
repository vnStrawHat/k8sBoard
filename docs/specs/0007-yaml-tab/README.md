# 0007 — Drawer YAML tab (read-only)

Status: amended after advisor review (10 items). Crates: `crates/cluster` and `crates/app`. Requires 0006 merged (0006 step 1 is `f5f682b`). Wireframes: W4 and W7 drawer tabs, W7 `View YAML` menu items. W10 (Edit YAML) is out of scope. Applies C1 (secret handling) and C6 (YAML serializer).

## Goal

- Every drawer (Pod, Node, every explorer kind, an event) gets a **YAML** tab: one GET on tab open, masked, serialized like `kubectl get -o yaml` (sorted keys, `status` kept, `managedFields` removed), shown in the kit's read-only code editor with highlighting, line numbers, and Ctrl+F search; **Refresh**, **Copy**, and "Fetched 12s ago".
- Node and kind drawers get the tab bar (Overview · YAML · Events) and the expand toggle; 0006's Events sections move into the Events tab.
- **View YAML** is enabled in every row and ⋯ menu and opens the drawer on the YAML tab.
- C1: Secret `data`/`stringData`, manifest annotations (last-applied, kapp originals) at any depth, and env literals (until "Env values" is toggled) are masked **in the cluster crate**; the app never holds raw values.

## Non-goals

Editing, diff, dry-run, and Apply (W10, 0031); a managedFields toggle (0031); revealing Secret values or manifest annotations (0016); the `Y` key (0028); the Monitor tab (0010); auto-refresh or watching the object; YAML export to a file; masking `command`/`args`.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: `object_yaml.rs` (kinds, refs, GET, masking, serialization), probe `--yaml` | 1, 2, 3, 4, 5 |
| 2 | App tab bars: `DrawerTab`, shared tab bar, Node and kind tabs with the Events tab, expand toggles, `-events` launch screens | 1, 2, 6, 8, 9 |
| 3 | App YAML tab: kit `tree-sitter-yaml`, `YamlView` (debounced first fetch), lifecycle, menus, `-yaml` screens, full ui-verifier run | 1, 2, 3, 4, 6a, 7, 8, 9 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale (dependency, masking, env toggle, RBAC, editor, tabs), ceilings |
| [cluster-object-yaml.md](cluster-object-yaml.md) | crate API, fetch, masking rules, serializer options, probe |
| [drawer-tabs.md](drawer-tabs.md) | step 2: `DrawerTab`, tabs per drawer, Events tab move, expand, launch screens |
| [yaml-view.md](yaml-view.md) | step 3: `YamlView` entity, async contract, toolbar, editor, drawer body, lifecycle |
| [menus-and-screens.md](menus-and-screens.md) | step 3: View YAML gating and menus, `-yaml` screens, settle, screenshot safety |
| [files-to-touch.md](files-to-touch.md) | modules and Cargo changes per step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, and so does `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes, offline.
- [ ] 3. No kube, k8s-openapi, or serde type in a public signature; the app gains no kube or serde dependency. The 0001 read-only grep still finds only the SSAR `create`. The crate never spawns tasks.
- [ ] 4. Secret safety: `object_yaml.rs` and `yaml_view.rs` contain no `tracing::` call; `ObjectYaml` has no `Debug`; serializer errors carry a fixed message; no file is written. Masking tests prove distinctive Secret, manifest-annotation, and env values are absent from the text.
- [ ] 5. Step 1 adds no `[[package]]` to `Cargo.lock`. Probe `--yaml` on UAT prints two `yaml` lines with `managedFields absent` and no `VISIBLE`; the 0001 AC7 credential script reports 0.
- [ ] 6. Node and kind drawers have tabs and ⤢; their Events show only in the Events tab; `show_screen` resets the tab to Overview (review).
- [ ] 6a. Step 3 adds exactly five `[[package]]` entries to `Cargo.lock`: four tree-sitter crates (`tree-sitter`, `tree-sitter-yaml`, `tree-sitter-json`, `tree-sitter-language`; all compile C through `cc`) and the transitive `streaming-iterator`.
- [ ] 7. The two `sync_yaml_view` invariants in [yaml-view.md](yaml-view.md) hold (review). On UAT, the YAML tab of a pod, a node, a deployment, and an event loads, refreshes, and copies; env literals read `<hidden>` until Env values is on. At most one YAML request is in flight, the first waits 250 ms, and the view is dropped when the tab is left.
- [ ] 8. The step's screenshots exist; the ui-verifier reports no high-severity defect against W4/W7 drawers.
- [ ] 9. The 0003 AC4 color-literal grep is clean.

## Open items

1. Clipboard auto-clear after N seconds (C1 open question) is not done; Copy copies the masked text only.
2. `command`/`args` literals can hold credentials and are shown, as in kubectl. Mask them too if the user wants.
3. Manifest annotations are never revealable here; if users need it, add a 30 s "Reveal" that refetches with it shown (C1 pattern).
