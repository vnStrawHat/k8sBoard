# 0007 · Test plan

[Back to index](README.md). **S** is the step that adds the test. Tests are deterministic and offline; cluster fixtures are `serde_json::json!` objects; each test checks one behavior.

## `crates/cluster` (step 1, `object_yaml_tests.rs`)

| Test | Checks |
|---|---|
| `object_kind_names_and_scopes` | every variant's `name()` and `is_namespaced()` (table) |
| `api_resources_match_kinds` | group, version, plural per kind (e.g. `apps`/`v1`/`deployments`, `batch`/`v1`/`cronjobs`, `networking.k8s.io`/`v1`/`ingresses`, core `""`/`v1`/`events`) |
| `object_ref_requires_namespace_exactly_for_namespaced_kinds` | Pod without and Node with a namespace → `None`; the valid pairs → `Some` |
| `managed_fields_are_removed` | |
| `manifest_annotations_are_hidden` | the three `MASKED_ANNOTATIONS` keys at top level → `<hidden>`, counted; other annotations unchanged |
| `manifest_annotations_are_hidden_in_templates` | the same keys under `spec.template.metadata.annotations` and a CronJob `spec.jobTemplate.spec.template.metadata.annotations` |
| `secret_values_are_hidden_and_keys_kept` | `data` and `stringData`; distinctive values absent from `text` |
| `secret_rule_ignores_api_version` | `kind: Secret` with a non-core `apiVersion` is masked too |
| `config_map_data_is_shown` | ConfigMap `data` and `binaryData` untouched (decision 8) |
| `env_values_are_hidden_in_pod_spec` | `containers`, `initContainers`, `ephemeralContainers`; `valueFrom` kept; `hidden_env_values` counts them |
| `env_values_are_hidden_in_nested_templates` | Deployment `spec.template`, CronJob `spec.jobTemplate.spec.template` |
| `env_values_are_shown_on_request` | `EnvValues::Shown` → values present, count 0 |
| `status_is_kept` | |
| `keys_are_sorted_like_kubectl` | `apiVersion`, `kind`, `metadata`, `spec`, `status` order, and nested keys sorted, for an input built in another order |
| `multi_line_strings_use_literal_blocks` | `|` |
| `long_single_line_strings_stay_on_one_line` | a 200-char string with spaces is one line |
| `header_counts_hidden_values` | `# k8sBoard hid 3 values as <hidden>.` and the `1 value` form |
| `no_header_without_hidden_values` | |

## `crates/app`

| S | File | Test |
|---|---|---|
| 2 | `drawer.rs` | `drawer_tabs_follow_the_wireframe_order` (pod, node, kind, event) |
| 2 | `drawer.rs` | `shown_tab_falls_back_to_overview` |
| 2 | `launch_options_tests.rs` | `drawer_screens_parse_with_their_tab` (pod-drawer, pod-containers, pod-events, node-events, deployments-events) |
| 2 | `launch_options_tests.rs` | existing `kind_screens_parse_from_plural_slugs` and drawer tests updated to the new variants |
| 3 | `drawer.rs` | `drawer_tabs_follow_the_wireframe_order` updated with YAML |
| 3 | `yaml_view.rs` | `object_ref_maps_pods_nodes_and_kinds` (Namespaces cluster-scoped, Deployments namespaced, Events) |
| 3 | `yaml_view.rs` | `yaml_subject_only_when_the_yaml_tab_is_shown` (no selection; Overview; Yaml; an event key) |
| 3 | `yaml_view.rs` | `fetch_status_by_state` (the four rows of the table) |
| 3 | `yaml_view.rs` | `env_toggle_shows_when_values_are_hidden_or_shown` |
| 3 | `resource_actions_tests.rs` | `view_yaml_is_enabled_in_every_access_state` (Checking, Unknown, Known with every check denied) |
| 3 | `resource_kind.rs` | `object_kinds_round_trip` (0006) still passes through `object()` |
| 3 | `launch_options_tests.rs` | `yaml_screens_open_the_yaml_tab` (pod-yaml, node-yaml, deployments-yaml) |
| 3 | `screenshot.rs` | `drawer_waits_for_its_content` (renamed 0006 test; content pending blocks) |
| 3 | `config_map_rows_tests.rs` | the existing note assertion reads "Values are in the YAML tab" |

## Live checks (coder-lite)

| S | Check |
|---|---|
| 1 | `probe --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --yaml`: two `yaml` lines, `managedFields absent`, last-applied `hidden` or `absent` (never `VISIBLE`). Then the 0001 AC7 credential script: every count 0 |
| 1 | `git diff --stat Cargo.lock` lists no new `[[package]]` |
| 2 | screens `pod-events`, `node-events`, `deployments-events`, `node-drawer`, `deployments-drawer`, `events-drawer` into `.tmp/ui-shots/` with the 0003 PNG checks |
| 3 | screens `pod-yaml`, `node-yaml`, `deployments-yaml`, `events-yaml`; record build time added by tree-sitter, the largest node YAML line count, and its time to show (decisions, ceilings) |
| 3 | `git diff Cargo.lock`: new `[[package]]` entries are exactly `tree-sitter`, `tree-sitter-yaml`, `tree-sitter-json`, `tree-sitter-language` |

## ui-verifier checklist

- Tabs: Pod `Overview · Containers n · YAML · Events n`; Node and Deployment `Overview · YAML · Events n`; event drawer `Overview · YAML`. Underline style, same padding as the header. ⤢ on every drawer.
- Node and Deployment Events tabs show the 0006 rows; Overview has no Events section.
- YAML tab: toolbar with "Fetched … ago", Copy and Refresh icons, Env values only on pod and workload YAML; a mono editor with line numbers and theme highlighting (keys and values in different theme colors); no `managedFields`; `status` present; env literals read `<hidden>` and the header comment is first.
- Row and ⋯ menus: View YAML enabled on pods, nodes, every kind, and events; no "later version" text anywhere.
- Light and dark: no contrast defects. Files `.tmp/ui-shots/0007-<screen>-<theme>.png`; dark for `pod-yaml` and `deployments-yaml`.
- **Deliberate deviations (not defects):** `status` is shown, unlike the W7 YAML placeholder comment "managedFields and status hidden" (decision 6); menus say "View YAML" everywhere, including kinds whose W7/W4 menu says "Edit YAML" (read-only; Edit YAML arrives with 0031, decision 23); no Monitor tab (0010); no `Y` hint (0028).
