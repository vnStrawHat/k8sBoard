# 0007 · Files to touch

[Back to index](README.md). Based on 0006 merged (step 1 at `f5f682b`) (0007 edits 0006's drawers, menus, and launch screens). **S** is the step; each step passes the gate on its own, and nothing lands before its first user.

## Cargo

| S | File | Change |
|---|---|---|
| 1 | root `Cargo.toml` | `[workspace.dependencies]`: `serde_json = "1"`, `serde-saphyr = { version = "1.3", default-features = false, features = ["serialize"] }` |
| 1 | `crates/cluster/Cargo.toml` | `serde_json.workspace = true`, `serde-saphyr.workspace = true` |
| 3 | root `Cargo.toml` | `gpui-kit = { version = "0.7", features = ["tree-sitter-yaml"] }` |

Step 1 must not add a `[[package]]` to `Cargo.lock` (check `git diff Cargo.lock`: only dependency edges change). Step 3 adds exactly five packages: four tree-sitter crates (`tree-sitter`, `tree-sitter-yaml`, `tree-sitter-json`, `tree-sitter-language`; all compile C through `cc`, already used by `ring`) and the transitive `streaming-iterator`. Any other new package is a deviation to report.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/object_yaml.rs` (new) | `ObjectKind`, `ObjectRef`, `EnvValues`, `ObjectYaml`, `object_yaml`, `api_resource`, `to_masked_yaml` and its mask helpers |
| `src/object_yaml_tests.rs` (new) | unit tests ([test-plan.md](test-plan.md)) |
| `src/lib.rs` | `mod object_yaml;` and its four exports |
| `examples/probe.rs` | `--yaml`, `USAGE`, module doc |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/drawer.rs` | `DrawerTab` (replaces `PodDrawerTab`), `drawer_tabs`, `shown_tab`, `drawer_tab_bar`; `DrawerHeader.expand` no longer optional |
| 2 | `src/pod_drawer.rs` (+ tests) | shared tab bar; delete `tab_bar` |
| 2 | `src/node_drawer.rs` | `&DrawerState`, tab bar, Events tab (section removed), expand |
| 2 | `src/kind_drawer.rs` | `&DrawerState`, tab bar, Events tab (section removed), expand |
| 2 | `src/workspace.rs` | `render_drawer` passes `&self.drawer` |
| 2 | `src/app_shell.rs` | `show_screen` resets the tab; `PodDrawerTab` → `DrawerTab`; launch tab setup through `drawer_tab()` |
| 2 | `src/launch_options.rs` (+ tests) | `PodDrawer(tab)`, `NodeDrawer(tab)`, `KindDrawer(kind, tab)`, `-events` names, `drawer_tab`, `USAGE` |
| 2 | `src/screenshot.rs` | match on the new variants |
| 3 | `src/yaml_view.rs` (new) | `YamlView`, `YamlRequest`, `object_ref`, `yaml_subject`, `fetch_status`, `shows_env_toggle`, `Render`. Inline tests |
| 3 | `src/drawer.rs` | `DrawerTab::Yaml`, tab lists, `DrawerBody`, `DrawerState.yaml`, `DRAWER_SUBJECT_DELAY` (250 ms, moved from 0006) |
| 3 | `src/pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs` | YAML tab body; `DrawerBody` |
| 3 | `src/app_shell.rs` | `sync_yaml_view`, `open_yaml`, `is_yaml_loading`, table delegates get `cx.weak_entity()`; `EVENT_SUBJECT_DELAY` deleted, the events debounce uses `DRAWER_SUBJECT_DELAY` |
| 3 | `src/config_map_rows.rs` (+ tests) | the Note "Values are not shown in this version" becomes "Values are in the YAML tab" |
| 3 | `src/resource_actions.rs` (+ tests) | `ViewYaml`, `view_yaml_item`, `shell` in `pod_menu` and `node_menu`, delete `YAML_DEFERRED_REASON` |
| 3 | `src/pod_table.rs`, `src/node_table.rs` | `shell: WeakEntity<AppShell>` field, passed to the menu |
| 3 | `src/resource_kind.rs` | `KindSpec.object: ObjectKind`, `object()`; `object_kind()` derives from it |
| 3 | `src/launch_options.rs` (+ tests) | `-yaml` names, `USAGE` |
| 3 | `src/screenshot.rs` (+ tests) | `is_drawer_ready(.., is_content_pending)` |
| 3 | `src/main.rs` | `mod yaml_view;` |

All new app items are private or `pub(crate)`. The app still has no kube or serde dependency.

## Docs (in the step that changes the behavior)

| S | Change |
|---|---|
| 1 | 0001 `probe-example.md`: the `--yaml` line |
| 2 | 0003 `drawer.md`: `DrawerTab`, tabs in every drawer; 0005 `decisions.md` decision 18 points to 0007; 0006 `object-events.md` "Placement" points to the Events tab |
| 2 | 0003 `screenshot-hook.md`: `node-events`, `<kind>-events` |
| 3 | 0003 `actions.md` and 0005 decision 22: View YAML enabled; 0005 decision 31: the ConfigMap note text; 0006 `object-events.md`: the delay constant's new home |
| 3 | 0003 `screenshot-hook.md`: `pod-yaml`, `node-yaml`, `<kind>-yaml` |
| 3 | `docs/roadmap/`: flip the YAML rows in `inventory-*.md` and the status table; C1 and C6 rows note 0007's choices |
