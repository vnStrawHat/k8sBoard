# Cross-cutting decisions, dependencies, UAT data

[Back to index](README.md). Settle each decision before its first consumer spec. "Proposed" is the architect default; the user confirms.

## Decisions

| ID | Topic | Proposed default | First consumer |
|---|---|---|---|
| C1 | **Secret handling** (Secrets kind, YAML view, Helm values and manifests, env literals, `last-applied-configuration`, TLS parsing) | Summaries never keep values (names, sizes, type only). Values are fetched per object on an explicit action, held only in drawer state, masked by default, revealed for 30 s, then dropped. Copy works without reveal. Nothing secret is traced, logged, persisted, or put in the audit log. YAML of a Secret and Helm manifests mask `data`/`stringData`; Helm values are masked until revealed. Pod Env/Mounts show names and sources, not literal values. "Reveal all" is per drawer, not per list. Screenshot runs never reveal. 0016: per-key reveal, Copy without reveal (private on Windows, cleared after 30 s with retries), zeroize ceiling; the clipboard question is settled. 0017: Helm values are masked by leaf (every string and number reads `<hidden>`), Reveal is per drawer for 30 s, manifests are masked per document and never revealed, notes are masked until revealed, and revealed copies go through the private clipboard. 0007 masks Secret `data`/`stringData`, manifest annotations (last-applied, kapp), and env literals in the cluster crate; env literals show only through a per-view "Env values" toggle (no 30 s timer); no Secret reveal in the YAML view. 0018: custom objects are shown like ConfigMaps plus key, kind, URL-userinfo, and password-format heuristics (`is_secret_key`, shared with 0014; secret-like kinds hide every scalar outside `status`); no reveal; summaries and fields hold only masked, capped values. | 0007 |
| C2 | **Persistence location** of the shipped app (registry, UI state, presets, audit log) | OS config dir (`%APPDATA%\k8sBoard`, `~/.config/k8sboard`, `~/Library/Application Support/k8sBoard`). A `--config-dir <path>` flag (and env var) overrides it. **Flag:** the "work only in the project folder" rule binds agents, not the shipped app, but agents, coder-lite, and ui-verifier must always run the app with `--config-dir .tmp/...` so no run writes outside the project. Never store tokens or key data; only paths and context names | 0024 |
| C3 | **Enabling mutations** | Approved by the user on 2026-10-02 (one approval for all mutating specs, 0030–0037; 0038 deferred). Debug builds still block writes unless `K8SBOARD_ALLOW_WRITES=1`, which agents never set. The kube `ws` feature is enabled only by 0035/0036. The 0001 read-only grep becomes an allow-list of named mutating call sites | 0030 |
| C4 | **Multi-cluster model** | One `ClusterSession` per selected cluster, each with its own watches; unselected clusters get a cheap health poll only (`/version` + node readiness every 60 s, while the switcher is open or every 5 min). Issue counts only for live sessions | 0026 |
| C5 | **Environment classification** | Guess from context/cluster name: `prod`, `prd` → PROD; `stg`, `stage`, `staging`, `uat` → STG; `dev`, `test` → DEV; `kind-`, `minikube`, `docker-desktop`, `k3d-`, `localhost` → LOCAL. Unknown → STG (a confirm dialog with a click, not a typed name). UAT `readonly@Monitor` is unknown → STG until the user sets it | 0024 |
| C6 | **New dependencies** | See the table below; each is approved in its spec | 0007 |
| C7 | **Lazy drawer content** | Sensitive or unbounded kinds (Secrets, Helm, custom resources) build drawer content from the selected summary on render, never pre-built `KindRow.sections` (0005 known ceiling). Events keep pre-built sections under the 2,000-event cap (0006 decision 12) | 0016 |
| C8 | **Write API style** | Decided by the user on 2026-10-02 (option 1 of 3, after research). YAML edits replace (`PUT`) the object with its base `resourceVersion` and field manager `k8sboard`, with no server-side apply and no Force path (0031 decision 1; a 409 leads to "Reload and keep my changes", a rebase by field path); JSON merge patch for single-field actions (scale, cordon, suspend); `dryRun=All` before every apply; Eviction API for drain and evict | 0030 |
| C9 | **User-chosen exports** (logs, Overview report, Topology PNG, audit log) | Only through a save-file dialog the user confirms; no automatic file writes. This reverses 0004 decision 15 ("logs never written to disk") for explicit Export only, so it needs user consent | 0019 |
| C10 | **Audit log** | Append-only JSON lines in the C2 dir: time, cluster, user identity, action, object, field paths changed, optional note. Values of Secret fields are never recorded; diffs are recorded as field paths plus non-secret values only | 0030 |
| C11 | **Sidebar counts for every kind** | Counts only for kinds with a live watch (today's rule) plus a one-shot metadata list on group expand, refreshed on navigation; no always-on watches for counts | 0009; Done (0012) |
| C12 | **Helm writes** | Native rollback (re-apply the previous manifest, write a new release Secret) is large and fragile; proposed: run the user's `helm` CLI with the same kubeconfig/context, shown as a command preview. User decision; 0038 deferred by the user (2026-10-02) | 0038 |
| C13 | **Always-on watch budget** | Settled by 0020 decisions 1–11: always-on core feeds plus eight condition feeds. Measured on UAT (scope All): idle RSS 113.5 MB after 5 min (budget 150 MB), 13 watches, `issue_evaluation_budget` 0.94 ms | 0020 (Done) |

## New dependencies (each approved in its spec)

| Need | Candidates (to verify by advisor) | Spec |
|---|---|---|
| YAML serialization | `serde-saphyr` 1.3 (chosen in 0007; `serde_json` is a direct dependency of the cluster crate) | 0007 |
| Charts | Own `Plot` implementation on the kit primitives (0010); no dependency | 0010 |
| Cron schedules + time zones | own robfig port + `jiff` `tzdb-bundle-always` (0012, done) | 0012 |
| x509 not-after | `x509-cert` 0.3 with `pem` 3 and `zeroize` (0016, done; Cargo.lock also records the inert `base64ct`) | 0016 |
| Helm decoding | `flate2` 1 + `base64` 0.22 + `serde-saphyr` `deserialize` for manifests (0017, done; all were already locked, so `Cargo.lock` only lists them under `k8sboard-cluster`) | 0017 |
| Diff | `similar` (named in the stack table); 0017 uses a path diff of the values, so `similar` waits for 0031 | 0031 |
| Regex log filter | `regex` | 0019 |
| Topology layout | hand-written Sugiyama vs a layout crate | 0022 |
| Config dirs and format | a config-dir crate; TOML or JSON | 0024 |
| Folder watching | a file-watcher crate or polling | 0025 |
| Fuzzy matching | `nucleo` (named in the stack table) | 0029 |
| WebSocket exec and port-forward | kube `ws` feature | 0035, 0036 |
| Terminal engine | `oneterm-vt`, git dependency pinned to a commit, `default-features = false` (already a commented line in the root `Cargo.toml`) | 0036 |
| Optional YAML LSP | `yaml-language-server` is an external Node process; proposed: skip LSP, use the kit editor's highlighting plus server dry-run validation | 0031 |

## UAT data availability (probe at `e80ed80`, `readonly@Monitor`, v1.29.5)

| Capability | UAT | Consequence |
|---|---|---|
| list pods, nodes, namespaces, the 10 workload/network/config kinds, events | allowed | 0006–0012 fully verifiable |
| `get pods/log` | allowed | 0019 verifiable |
| `metrics.k8s.io/v1beta1` | available | 0010 verifiable |
| `get nodes/proxy` | allowed | verified by the 0011 probe (summary 39 to 175 KB per node, cAdvisor about 0.9 MB); node logs 0019 |
| `list secrets` | allowed | real Secret values reachable: C1 landed with 0016 (0016 probe: 42 secrets, 8 TLS all parsed, earliest leaf not-after 2026-12-26); Helm data readable if releases exist; 0017 probe: 0 Helm releases on UAT (no `helm.sh/release.v1` Secret), so Releases is verified by fixture tests and the empty state |
| `create pods/exec`, `create pods/portforward` | denied | 0035–0037 render disabled; live checks need another cluster |
| PVCs, PVs, StorageClasses | allowed (0014 probe: 29 PVCs, 57 PVs, 2 StorageClasses) | 0014 verifiable |
| NetworkPolicies, HPAs, quotas, PDBs | allowed (0013 probe: 8 policies, 1 HPA, 0 quotas, 7 PDBs) | 0013 verifiable; quotas only by unit tests and the empty state |
| ServiceAccounts, Roles, ClusterRoles, RoleBindings, ClusterRoleBindings | allowed (0015 probe: 98 service accounts, 28 roles, 95 cluster roles, 31 role bindings, 82 cluster role bindings) | 0015 verifiable |
| EndpointSlices, CRDs | EndpointSlices allowed (0012); CRDs allowed (0018 probe: 72 CRDs, all Established, 183 printer columns and none unsupported; `list` of the first ten kinds allowed, counts 10, 0, 1, 1, 0, 0, 4, 0, 7, 0) | each spec adds `AccessCheck` variants and records results first |
| Objects that may not exist on UAT (HPAs, PDBs, cert-manager, Helm releases, CRDs) | unknown | specs need fixture-based unit tests and empty-state screenshots |
