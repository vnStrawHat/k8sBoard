# Cross-cutting decisions, dependencies, UAT data

[Back to index](README.md). Settle each decision before its first consumer spec. "Proposed" is the architect default; the user confirms.

## Decisions

| ID | Topic | Proposed default | First consumer |
|---|---|---|---|
| C1 | **Secret handling** (Secrets kind, YAML view, Helm values and manifests, env literals, `last-applied-configuration`, TLS parsing) | Summaries never keep values (names, sizes, type only). Values are fetched per object on an explicit action, held only in drawer state, masked by default, revealed for 30 s, then dropped. Copy works without reveal. Nothing secret is traced, logged, persisted, or put in the audit log. YAML of a Secret and Helm manifests mask `data`/`stringData`; Helm values are masked until revealed. Pod Env/Mounts show names and sources, not literal values. "Reveal all" is per drawer, not per list. Screenshot runs never reveal. Open: auto-clear the clipboard after N s? 0007 masks Secret `data`/`stringData`, manifest annotations (last-applied, kapp), and env literals in the cluster crate; env literals show only through a per-view "Env values" toggle (no 30 s timer); no Secret reveal in the YAML view. | 0007 |
| C2 | **Persistence location** of the shipped app (registry, UI state, presets, audit log) | OS config dir (`%APPDATA%\k8sBoard`, `~/.config/k8sboard`, `~/Library/Application Support/k8sBoard`). A `--config-dir <path>` flag (and env var) overrides it. **Flag:** the "work only in the project folder" rule binds agents, not the shipped app, but agents, coder-lite, and ui-verifier must always run the app with `--config-dir .tmp/...` so no run writes outside the project. Never store tokens or key data; only paths and context names | 0024 |
| C3 | **Enabling mutations** | The project rule needs user approval per feature. Ask once for the 0030 framework, then per spec (0031–0038). The kube `ws` feature is enabled only by 0035/0036. The 0001 read-only grep becomes an allow-list of named mutating call sites | 0030 |
| C4 | **Multi-cluster model** | One `ClusterSession` per selected cluster, each with its own watches; unselected clusters get a cheap health poll only (`/version` + node readiness every 60 s, while the switcher is open or every 5 min). Issue counts only for live sessions | 0026 |
| C5 | **Environment classification** | Guess from context/cluster name: `prod`, `prd` → PROD; `stg`, `stage`, `staging`, `uat` → STG; `dev`, `test` → DEV; `kind-`, `minikube`, `docker-desktop`, `k3d-`, `localhost` → LOCAL. Unknown → STG (asks for Enter, not a typed name). UAT `readonly@Monitor` is unknown → STG until the user sets it | 0024 |
| C6 | **New dependencies** | See the table below; each is approved in its spec | 0007 |
| C7 | **Lazy drawer content** | Sensitive or unbounded kinds (Secrets, Helm, custom resources) build drawer content from the selected summary on render, never pre-built `KindRow.sections` (0005 known ceiling). Events keep pre-built sections under the 2,000-event cap (0006 decision 12) | 0016 |
| C8 | **Write API style** | Server-side apply with field manager `k8sboard` for YAML edits (conflicts shown, force only with confirm); JSON merge patch for single-field actions (scale, cordon, suspend); `dryRun=All` before every apply; Eviction API for drain and evict | 0030 |
| C9 | **User-chosen exports** (logs, Overview report, Topology PNG, audit log) | Only through a save-file dialog the user confirms; no automatic file writes. This reverses 0004 decision 15 ("logs never written to disk") for explicit Export only, so it needs user consent | 0019 |
| C10 | **Audit log** | Append-only JSON lines in the C2 dir: time, cluster, user identity, action, object, field paths changed, optional note. Values of Secret fields are never recorded; diffs are recorded as field paths plus non-secret values only | 0030 |
| C11 | **Sidebar counts for every kind** | Counts only for kinds with a live watch (today's rule) plus a one-shot metadata list on group expand, refreshed on navigation; no always-on watches for counts | 0009 |
| C12 | **Helm writes** | Native rollback (re-apply the previous manifest, write a new release Secret) is large and fragile; proposed: run the user's `helm` CLI with the same kubeconfig/context, shown as a command preview. User decision | 0038 |
| C13 | **Always-on watch budget** | Issues and Overview need pods, nodes, events, and a few kinds watched for the whole session. Budget: idle RAM < 150 MB (wireframe principle) on a ~1,000-pod cluster; measure before 0020 merges | 0020 |

## New dependencies (each approved in its spec)

| Need | Candidates (to verify by advisor) | Spec |
|---|---|---|
| YAML serialization | `serde-saphyr` 1.3 (chosen in 0007; `serde_json` is a direct dependency of the cluster crate) | 0007 |
| Charts | GPUI Kit chart (stack table says it exists; verify ref lines, crosshair, gaps) or a custom `canvas` | 0010 |
| Cron schedules + time zones | a cron parser crate; `jiff` time-zone features (0004 open item 3) | 0012 |
| x509 not-after | an x509 parser crate | 0016 |
| Helm decoding | gzip (`flate2`), base64, `serde_json` | 0017 |
| Diff | `similar` (named in the stack table) | 0017, 0031 |
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
| `get nodes/proxy` | allowed | 0011 kubelet stats and node logs verifiable |
| `list secrets` | allowed | real Secret values reachable: C1 must land before 0016/0017; Helm data readable if releases exist |
| `create pods/exec`, `create pods/portforward` | denied | 0035–0037 render disabled; live checks need another cluster |
| NetworkPolicies, HPAs, quotas, PDBs, storage, RBAC kinds, EndpointSlices, CRDs | not probed yet | each spec adds `AccessCheck` variants and records results first |
| Objects that may not exist on UAT (HPAs, PDBs, cert-manager, Helm releases, CRDs) | unknown | specs need fixture-based unit tests and empty-state screenshots |
