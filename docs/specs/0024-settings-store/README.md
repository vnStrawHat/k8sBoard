# 0024 — Settings store, cluster registry, environments

Status: implemented (steps 1–5) and verified; amended after the advisor review (M1–M3, S1–S7, N1–N6), HEAD `95aac91`. Crates: `crates/cluster` (merged kubeconfig loading) and `crates/app`. Local only: the app writes its own config file; nothing is written to a cluster. Settles C2 and C5 and the C6 row "Config dirs and format". Wireframes: W1 (env badge and title-bar border), W2 (Clusters form fields: data model only).

## Goal

- A versioned **settings file** (`settings.json`) in the OS config dir, with `--config-dir` / `K8SBOARD_CONFIG_DIR` overrides, atomic writes, a `.bak` on corruption, and a notice.
- A **cluster registry**: user-added kubeconfig files and per-context overrides (display name, environment, read-only lock, default namespace) plus the last-used cluster. Paths and names only, never credentials.
- **`KUBECONFIG` multi-entry merge** with kubectl semantics (first file wins); registry-added files load standalone.
- **Environments** PROD/STG/DEV/LOCAL guessed from names (C5), theme-token colors, an env badge on the cluster switcher, and an env-colored 3 px top border on the title bar.
- **Persisted prefs wired now**: theme, table sort and hidden columns (0009), default namespace ("Set as default"). Later specs add their own sections through the same API.

## Non-goals

- The Settings window and any editing UI (0025); env groups, health, Ctrl 1–9 in the switcher (0026); multi-cluster (0027).
- Enforcing the read-only lock (0030); cluster colors other than the env color; density; dock height.
- Live reload of hand edits, multi-instance locking, migrations from older schema versions (none exist).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: `Kubeconfig::load(&[PathBuf])` merge (kind/apiVersion pre-check), `ContextSummary.source`, skipped files | 1–3 |
| 2 | Settings model, store, `AppSettings` global (writer, quit flush), `--config-dir`, theme pref, notices | 1–6 |
| 3 | Registry, environments, chain + standalone files, last-used start, title-bar badge and top border | 1, 2, 7–9 |
| 4 | Table prefs (0009) read and write | 1, 2, 10 |
| 5 | Default namespace: "Set as default namespace" and its use at start/switch; full ui-verifier run | 1, 2, 11, 12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with one-line rationale |
| [settings-store.md](settings-store.md) | location, format, schema, load and recovery, atomic write and gate, writer and quit flush, `AppSettings` API, notices, secret rules |
| [cluster-registry.md](cluster-registry.md) | merged kubeconfig API, chain and standalone files, registry model, profile, start selection, default namespace |
| [environments.md](environments.md) | `Environment`, C5 guessing rules, theme tokens, title-bar badge and top border |
| [persisted-prefs.md](persisted-prefs.md) | theme and table prefs (wired now); reserved sections for 0016, 0019, 0020, 0022, 0025, 0030, 0035 |
| [files-to-touch.md](files-to-touch.md) | files per step, Cargo change, agent-run rule |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [x] 2. Every test in [test-plan.md](test-plan.md) for the step exists under that name and passes offline. Tests write only under `std::env::temp_dir()` (the project `.tmp/` in agent runs).
- [x] 3. `KUBECONFIG=a;b` (`:` on Unix) lists the contexts of both files; a duplicate context name resolves to the first file; an unreadable or incompatible entry is skipped with a notice and earlier files stay merged. A registry file loads standalone: its same-named context appears as a separate switcher item labeled with its file name.
- [x] 4. `Cargo.lock` gains no new package: `dirs` 6 and `serde_json` are already locked (`git diff Cargo.lock` shows only the `k8sboard` dependency lines).
- [x] 5. A corrupt `settings.json` is renamed to `settings.json.bak`, the app starts with defaults, and the title bar shows the notice. A file with a newer `version` loads but is never overwritten.
- [x] 6. `settings.json` never holds kubeconfig content: the serialized keys are exactly the allow-list in [settings-store.md](settings-store.md). `grep -rnE "(trace|debug|info|warn|error)!.*[?%] *(settings|loaded|entry|registry)" crates/app/src` finds nothing (no `Debug`/`Display` capture of settings values in tracing).
- [x] 7. `readonly@Monitor` with no registry entry shows an **STG** badge and an amber title-bar top border; with a seeded entry `"environment": "production"` it shows **PROD** and the danger-colored top border.
- [x] 8. The 0003 color-literal grep stays clean: env colors come from theme tokens only.
- [x] 9. Without `--context`, the app reopens the last-used cluster when it is still loaded; with `--kubeconfig X` and no `--context`, a `last_used` in X beats X's `current-context`, and a `last_used` from another (registry) file never overrides an explicit `--kubeconfig`. `--context`, `--namespace`, `--theme` still override.
- [x] 10. A sort or hidden column set on a screen is restored on the next run (seeded-file screenshot + writer unit test).
- [x] 11. "Set as default namespace" on a Namespaces row stores the namespace for the active cluster; the next start or switch opens it.
- [x] 12. Every agent run of the app passes `--config-dir .tmp/config` (or a sub-folder); debug builds default to `<workspace>/.tmp/config` anyway (decision 4).

## Open items

1. 0025 decides how "Paste kubeconfig YAML" stores pasted content: a separate kubeconfig file under `<config>/kubeconfigs/`, never inside `settings.json`.
2. Two running instances overwrite each other's changes (last writer wins). Add a lock file if users run several windows as separate processes.
3. Hand edits made while the app runs are overwritten by its next write. 0025 may add reload on file change.
4. Registry matching compares absolute paths; a moved kubeconfig orphans its entries (kept, harmless). 0025 can offer "relink".
