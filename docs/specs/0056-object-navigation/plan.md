# 0056 · Implementation plan

[Back to index](README.md). Eleven steps in three lanes. Each step is one coder session, merges on its own, passes the quality gate, and adds the tests named here (pure helpers get unit tests; `cargo test` runs on all three OSes).

## Lanes and dependencies

| Step | Depends on | Shared files with other lanes |
|---|---|---|
| A1 | — | `app_shell.rs` (A3, B3a), `main.rs` (C3) |
| A2 | A1, B1 | `drawer.rs` (B1, A3), drawer files (B2, B3a, B3b: one new parameter) |
| A3 | A1, B1 (`open_link`) | `app_shell.rs` (B3a), `drawer.rs` (`open_link` body only) |
| B1 | — | `drawer.rs` (A2, A3) |
| B2 | B1 | `pod_drawer.rs` (A2, B3a) |
| B3a | B2 | `app_shell.rs`, `cluster_session.rs` (C3) |
| B3b | B3a | `cluster_session.rs`, `live_sections.rs` (B4) |
| B4 | B3b | `live_sections.rs` |
| C1 | — (can start at once) | none |
| C2 | — | none (cluster crate) |
| C3 | C1, C2, B3b | `cluster_session.rs`, `main.rs` |

Rules for two parallel coders (coder 1 = A1, C1, C2, A3, A2, C3; coder 2 = B1, B2, B3a, B3b, B4):
- `app_shell.rs`: A1, A3 and B3a edit different functions (A1 `reveal_object` / new `go_back`, A3 new `follow_link`, B3a `selected_related_subject` only). Whoever merges second rebases; no step reformats or moves another's function.
- `cluster_session.rs`: B3a/B3b add related-list arms, C3 adds the name index; C3 rebases after B3b.
- A2 adds one parameter to `pod_drawer` / `kind_drawer` / `node_drawer`; a B step that lands later rebases onto it (trivial).
- A1 and C3 each add one `mod` line to `main.rs`.

## Steps

| Step | Scope | Files | Tests |
|---|---|---|---|
| **A1** History core + keys | `NavigationHistory`, `Place`, record in `reveal_object` (skip self-reveal), `go_back` / `go_forward`, restore through the setters, resets; `GoBack` / `GoForward`, `Alt+Left` / `Alt+Right`, terminal and input `NoAction` (after checking the kit key contexts), shortcut rows | new `navigation_history.rs` (+ `_tests.rs`), `app_shell.rs`, `keymap.rs`, `keyboard_navigation.rs`, `main.rs` | record / back / forward / cap 50 / clear; forward cleared by a new record; self-reveal not recorded; shell tests: back restores screen, selection, tab, container and container tab, filter; origin without selection; Topology restore (no `pending_reveal`, drawer once feeds deliver); action pairs via `when_selected` not recorded |
| **A2** Drawer header controls | `DrawerNavigation`, Back button with label and tooltip, Prev / Next + `12 of 40`, hidden on Topology / editor | `drawer.rs`, `workspace.rs`, `pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs`, `port_forward_page.rs` (default), `navigation_history.rs` (`row_position`) | `row_position` (none, 1 of 1, last, past end); back label truncation and screen-title fallback |
| **A3** `follow_link` | `link_step` (Reveal / Denied / OutOfScope) using `cluster_session::scope_includes`, notifications; `open_link` calls it | `navigation_history.rs`, `app_shell.rs`, `drawer.rs` (`open_link` body only) | `link_step`: allowed, denied known, checking, cluster-scoped, outside `Named` / `Several`, `All`; notification texts |
| **B1** One link style | `link_style` helper and `open_link` chokepoint; endpoints, not-ready, revisions, recent jobs, selected pods, mounted-by, related pods, kind-drawer WHY link | `drawer.rs`, `live_sections.rs`, `related_pods.rs`, `kind_drawer.rs` | `endpoints_content` keeps the pod key and splits address / pod text; existing tests stay green |
| **B2** Pod Overview, no new watch | Service account link, `Deployment` row (`deployment_of_pod`), `Volumes`, `Labels`; Secret / PVC targets in Env and Mounts | `pod_drawer.rs`, `kind_row.rs`, `container_detail.rs` (+ their `_tests.rs`) | `deployment_of_pod` (hash label match, mismatch, missing label, non-RS owner, empty prefix); volume rows dedupe and sources; env / mount targets for configmap, secret, pvc, empty name |
| **B3a** Pod `Services` | `RelatedSubject::PodServices`, `RelatedList::Services`, `ListServices` denial, key-based subject for Pod keys in `selected_related_subject`; Pod `Services` section | `related_objects.rs`, `cluster_session.rs`, `app_shell.rs` (one fn), `pod_drawer.rs` | `services_selecting(pod, services)` (match, selector-less, other namespace); subject for a Pod key on any screen; `watch_count` / `watched_kinds` include it; denied subject not started |
| **B3b** Service `Exposed by` | `RelatedSubject::ServiceIngresses`, `RelatedList::Ingresses`, `ListIngresses` denial; `LiveContent::ExposedBy` section | `related_objects.rs`, `cluster_session.rs`, `app_shell.rs` (same fn, one arm), `kind_row.rs`, `network_rows.rs`, `live_sections.rs` | `exposing_ingresses` (rule, default backend, other namespace, resource backend); subject for a Service key |
| **B4** Ingress Backends, NetworkPolicy Pods | `ingress_backends`, column; `selected_pods_rows(namespace, selector)` shared by PDB and NetworkPolicy | `network_rows.rs`, `resource_kind.rs`, `network_policy_rows.rs`, `live_sections.rs`, `kind_join.rs` only if a column index moves | `ingress_backends` order / dedupe / default / resource backend; TLS join still fills the right cell |
| **C1** Palette layer 1 | `Subject::Named`; condition feed objects filtered by `scope_includes`, live feeds only; dedupe; hint names live feeds | `palette_search.rs`, `command_palette.rs` (+ tests) | a feed Deployment matches; out-of-scope feed object skipped; `Off` / `Wait` feed not searched or named; on-screen row wins over the feed copy |
| **C2** `list_object_names` | paging, 5,000 cap, scope fan-out | new `crates/cluster/src/object_names.rs` (+ `_tests.rs`), `lib.rs` | fake API: two pages joined via continue token; cap sets `is_truncated` and stops paging; `Several` lists each namespace; params carry `limit=500` and no `resourceVersion` |
| **C3** Palette name index | `NameIndex`, `request_name_index`, `wants_run`, failure stamping, task abort, stale-scope discard, palette trigger, texts | new `name_index.rs` (+ tests), `main.rs`, `cluster_session.rs`, `palette_search.rs`, `command_palette.rs` | `wants_run` (never, fresh, stale, in flight, other scope, **after a failure within 120 s: no run**); denied kinds skipped; stale-scope result discarded; texts; fixture test: ranking over ~9,000 subjects (3,000 pods + feeds + index) stays under the 4 ms budget in release (`palette ranked` trace or a timed test marked `#[ignore]` run by the coder) |

## Live check (ui-verifier, UAT read-only cluster)

After A2, B3b, C3: Ingress -> Service (Exposed by back-link) -> endpoint Pod -> Node, then `Alt+Left` × 3 back to the Ingress with its filter; palette finds a Service by name; Pod drawer Prev / Next walk a filtered Pods list.
