# 0003 — App shell, Pods and Nodes (first GPUI screen)

Status: approved for implementation (user decisions and advisor review applied). Crates: `crates/app` (main work) and a small addition to `crates/cluster`. Requires [0002](../0002-live-resource-watch/README.md). Wireframes: anatomy, W1, W4, W4b, W5, tokens.

## Goal

Replace the hello window with the k8sBoard shell:

- a title bar with a cluster switcher, a namespace picker, and a read-only lock;
- a grouped navigation sidebar;
- live, virtualized Pods and Nodes tables with themed status colors;
- an overlay drawer: a pod drawer with Overview and Containers (master-detail) tabs, and a node drawer;
- a right-click menu and a drawer ⋯ menu with read-only actions, RBAC-gated;
- a status bar, plus loading, empty, and error states;
- a dev-only screenshot hook for the ui-verifier.

## Non-goals (deferred)

- **Dock, logs, and shell.** "View logs" is shown disabled ("Logs open in the dock, coming in a later version").
- **View YAML** moves to a later spec (the drawer YAML tab). That spec must meet these secret-handling rules, because `env[].value` literals and the `last-applied-configuration` annotation can hold plaintext secrets:
  - show the YAML only on an explicit user action;
  - never log it, and never write it to disk;
  - Copy is the only export.
- Settings window (W2), env colors, multi-cluster aggregation, and the Cluster column. Multi-select is designed in [shell-layout.md](shell-layout.md) but not built.
- Mutating actions. Edit, restart, evict, delete, attach, exec, port-forward, cordon, and drain are never executed. Shell, Port-forward, Cordon, and Drain appear only as disabled items.
- Overview (W3), Issues, Topology, other kinds, Events, the palette, search, filter chips, column sort, and multi-row selection.
- Monitor, YAML, and Events drawer tabs; container sub-tabs, ports, resources, probes, and the "WHY" box; CPU and Memory columns.
- Kubeconfig merging (only the first `KUBECONFIG` entry is used), proxy support, and persisted UI state.

## Files

| File | Contents |
|---|---|
| [app-structure.md](app-structure.md) | module layout, Cargo, entity graph, state types |
| [bootstrap.md](bootstrap.md) | CLI flags, kubeconfig discovery, runtime, session lifecycle, initial namespace |
| [shell-layout.md](shell-layout.md) | title bar, navigation, workspace header, status bar, states |
| [tables.md](tables.md) | Pods/Nodes columns, status tones, ages, selection |
| [drawer.md](drawer.md) | overlay drawer, pod Overview/Containers, node drawer |
| [actions.md](actions.md) | context and ⋯ menus, RBAC gating, Copy name |
| [cluster-additions.md](cluster-additions.md) | new `PodSummary`/`ContainerSummary` fields |
| [screenshot-hook.md](screenshot-hook.md) | `--screenshot` mechanism on Windows, and how to verify it |
| [screenshot-script.md](screenshot-script.md) | `--script`: scripted keys, clicks, and several shots in one run |
| [test-plan.md](test-plan.md) | unit tests and the ui-verifier checklist |

## Acceptance criteria

- [x] 1. The quality gate passes. `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings` also passes. No new `#[allow]`.
- [ ] 2. Every test in [test-plan.md](test-plan.md) exists under that name and passes.
- [ ] 3. `crates/app/Cargo.toml` has no `kube` or `k8s-openapi` dependency. `screenshot` is not in `default`. The 0001 read-only grep still finds only the SSAR `create`. — superseded by 0030 (write allow-list and named connect files replace the read-only grep)
- [x] 4. `crates/app/src` has no hex, `rgb(`, or `hsla(` color literals. All colors come from `cx.theme()` (scoped grep).
- [ ] 5. `cargo run -p k8sboard -- --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor` starts on all namespaces (UAT allows list pods) and shows live pods (about 104) and nodes (4). Namespace and context switching work with no main-thread stall.
- [ ] 6. Without `--context`, the app opens with an error state that names the invalid current-context, and the switcher lists `readonly@Monitor`.
- [ ] 7. Screenshot hook: all five screens produce valid PNGs in light and dark ([screenshot-hook.md](screenshot-hook.md)). The ui-verifier reports no high-severity defects against W4/W4b/W5.
- [ ] 8. On UAT, Shell and Port-forward are disabled with "Not permitted: create pods/exec" and "Not permitted: create pods/portforward" (`resource_actions` tests plus a user spot-check).

## Future refinements

- A "Readiness failed" grace period: it currently also shows during a probe's initial delay.
- An age ticker, if stale ages are noticed.
