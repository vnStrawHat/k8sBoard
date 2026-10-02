# 0026 · Decisions

[Back to index](README.md). Defaults picked without a user round-trip (user instruction).

| # | Decision | Rationale |
|---|---|---|
| 1 | **Break before make**: release the old session (shell, tables, dock, drawer state), then create the new `ClusterSession` in a `cx.defer` callback, which runs after `release_dropped_entities` in the same effect flush | C13 memory budget; no two watch sets at once; coordinator rule |
| 2 | Kept across a switch: the screen (Pods, Nodes, kind), the dock height, column sort and hidden columns (0024 prefs). Reset: drawer, selection, checked rows, filters (0009 decision 11), log tabs, namespace picker, pending launch screen and reveal | W1 "click a name to switch entirely"; log streams and drawer subjects belong to the old cluster |
| 3 | Namespace scope is remembered **per cluster in memory** (`HashMap<ClusterRef, NamespaceScope>`) for the app session; start order: memory > 0024 `default_namespace` > today's default | returning to a cluster lands where the user was; not persisted (YAGNI) |
| 4 | `last_used` is written when the session becomes **Live**, not when it starts (amends 0024 decision 22) | a broken cluster is not reopened on the next launch |
| 5 | A failed connect keeps the target selected and shows the existing workspace error view plus **"Back to {previous}"** (decisions 21, 22) | the user can retry or return in one click; the previous cluster is not kept warm (decision 1) |
| 6 | The switcher is a controlled kit **`Popover`** (pattern of `namespace_picker.rs`), not a `DropdownMenu` | it needs a text input, a segment, and rich rows |
| 7 | Rows reuse `cluster_groups` (0025 step 3 in `cluster_form.rs`, or `cluster_registry.rs` when 0026 lands first; files-to-touch.md) | one ordering for Settings, switcher, and Ctrl 1–9 |
| 8 | `Ctrl 1…9` index the **unfiltered** display order (W1 hints), computed when pressed | stable numbers while typing in the filter |
| 9 | Health probes run only **while the switcher is open**, only for auth kinds that need no external program (token, token file, client certificate, basic, none); exec and auth-provider rows probe on **Check** | C4 cost; an exec plugin may open a browser login or MFA prompt just from opening a menu |
| 10 | Probe = `ClusterConnection::open` + `server_version()` (latency times the version call only) with a 5 s timeout, `buffer_unordered(4)` over `ProbeTarget`s on tokio, fed through `ClusterRuntime::subscribe`; results cached 60 s; closing clears `running` | bounded fan-out; closing the popover drops the subscription and aborts probes |
| 11 | The active cluster's health comes from its session (Live, Interrupted, Connecting, Failed), never from a probe | no duplicate connection to the cluster in use |
| 12 | Health line texts: `Live · {n} ms`, `Reachable · {n} ms`, `Unreachable` + Retry, `Checking…`, `Not checked` + Check, `Connecting…`, `Interrupted` | W1 health summary without issue counts (0020) |
| 13 | "Connected" segment = rows whose last known health is Live or Reachable | W1 `Connected 8`; unknown rows are not counted as connected |
| 14 | Filter matches label, context name, env badge, and source file name, case-insensitive substring; highlight starts on the first match | small lists; no fuzzy dependency (0029 owns fuzzy) |
| 15 | In the popover: `down`/`up` move the highlight, `enter` switches, `escape` closes, bound in `ClusterSwitcher` and `ClusterSwitcher > Input` (registered after kit bindings); handlers never propagate | the filter input keeps focus; deterministic over the kit `Input` and `Popover` keys |
| 16 | `Ctrl Shift C` and `Ctrl 1…9` live in 0028's `WINDOW` context; if 0028 has not merged, they are bound in `app_shell::bind_keys` and 0028 moves them | chords work in text fields (0028 rule 2) |
| 17 | Status bar: `API {git_version} · {n} ms`, n = the `/version` round trip measured during connect | roadmap 0026; no periodic latency polling |
| 18 | Unit actions `SwitchToCluster1` … `SwitchToCluster9` (no parameterized action) | 0028 decision: unit actions; no serde/JsonSchema on actions |
| 19 | Switching to the active cluster is a no-op; switching while connecting cancels the pending connect | existing `is_active` rule; dropping the `Connecting` task aborts it |
| 20 | A target whose context left its kubeconfig (catalog reload) shows the notice `'{label}' is no longer in its kubeconfig` and no switch | no half-started session |
| 21 | `previous` is always the cluster you came from (set on every switch whose target differs from the current one) | one rule; "Back" after a failed switch returns to the last cluster you were on |
| 22 | "Back to {previous}" is offered only when `previous` still resolves in the catalog | no button that cannot work |
| 23 | Row-menu closures hold `WeakEntity<ClusterSession>` (pod and kind drawers today hold strong clones) | rendered closures of the last frame must not keep a released session alive |
| 24 | Pure row model in `cluster_switcher_rows.rs`; state and render in `cluster_switcher.rs` | testable without GPUI; keeps the render file focused |
| 25 | Focus: `Popover::track_focus(filter)`; the kit focuses it on open and restores the previous focus on close | the kit overwrites any focus set before its open transition |
