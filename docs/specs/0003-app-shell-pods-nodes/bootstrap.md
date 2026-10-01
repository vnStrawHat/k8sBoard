# 0003 · Bootstrap: flags, kubeconfig, runtime, session

[Back to index](README.md) · Modules: `main.rs`, `launch_options.rs`, `cluster_runtime.rs`, `cluster_session.rs`

## Flags (hand-parsed from `std::env::args()`, like the probe; no clap)

| Flag | Default | Notes |
|---|---|---|
| `--kubeconfig <path>` | see discovery below | |
| `--context <name>` | kubeconfig `current-context` | an invalid current-context gives an error state, not an exit |
| `--namespace <name>` | all namespaces if allowed, else the context namespace | see "Initial namespace" below |
| `--theme light\|dark` | follow the system | `Theme::change(ThemeMode::…, None, cx)`, else `Theme::sync_system_appearance(None, cx)` |
| `--screen pods\|nodes\|pod-drawer\|pod-containers\|node-drawer` | `pods` | initial screen; also used by the hook |
| `--screenshot <path.png>` | none | needs the `screenshot` feature, otherwise exit 2 with "rebuild with --features screenshot" |
| `--help` | | usage to stderr, exit 0 |

An unknown flag, a missing value, or an invalid enum value prints usage to stderr and exits 2. This happens before GPUI starts.

```rust
pub(crate) struct LaunchOptions { kubeconfig: Option<PathBuf>, context: Option<String>,
    namespace: Option<String>, theme: Option<ThemeChoice>, screen: Screen, screenshot: Option<PathBuf> }
pub(crate) enum ThemeChoice { Light, Dark }
pub(crate) enum LaunchRequest { Run(LaunchOptions), Help }
pub(crate) fn parse_launch_options(args: impl Iterator<Item = String>) -> Result<LaunchRequest, String>;
/// --kubeconfig, else the first entry of KUBECONFIG (std::env::split_paths), else <home>/.kube/config.
pub(crate) fn kubeconfig_path(flag: Option<PathBuf>, kubeconfig_env: Option<OsString>,
    home: Option<PathBuf>) -> Option<PathBuf>;   // None -> error state "no kubeconfig found"
```

- `home` comes from `std::env::home_dir()`. Only the first `KUBECONFIG` entry is used, because merging is a non-goal (0001). Say so in the error state if there are more.
- `Screen` lives in `app_shell.rs`. Its `pod-drawer`/`pod-containers`/`node-drawer` values map to `Screen::Pods`/`Nodes` plus a drawer request.

## `main`

1. Init tracing, parse options, and build the tokio runtime as in [0002 app-subscription](../0002-live-resource-watch/app-subscription.md). Keep it on the stack.
2. `application().run(…)`:
   - `gpui_kit::init`, then apply the theme;
   - `cx.set_global(ClusterRuntime { handle })`;
   - open the window;
   - call `cx.quit()` when the last window closes.
3. Window: `WindowOptions { window_bounds: centered 1320×900, ..TitleBar::window_options() }`. The size matches the wireframe reference render. The kit `TitleBar` draws the caption buttons on Windows.
4. Exit code: 0, or the screenshot result ([screenshot-hook.md](screenshot-hook.md)). `anyhow` is allowed only here.

## Kubeconfig and context

1. `AppShell::new` sets `KubeconfigState::Loading` and runs `Kubeconfig::load(&path)` on `cx.background_spawn`. This is blocking file I/O and must stay off the main thread and off tokio.
2. Loaded: pick the context with `kubeconfig.resolve_context(options.context.as_deref())`.
   - `Ok(summary)`: `start_session(summary)`.
   - `Err(e)`: there is no session. The workspace shows `e`, whose text lists the available contexts, and the switcher lists them.
3. Failed: the workspace error shows the `KubeconfigError` Display. The switcher is empty and disabled.

## `ClusterSession` lifecycle

| Step | Runs on | Result |
|---|---|---|
| `connect(kubeconfig, summary, requested_namespace, cx)` | tokio, in one task: `open`, then `server_version`, then the initial scope choice (below) | `Live { scope, access }` or `Failed { message }` |
| on `Live` | `subscribe` × 3: `watch_namespaces()`, `watch_pods(scope)`, `watch_nodes()` | each `LiveList` starts `Loading` |
| `set_scope(scope)` | drop the pods subscription and re-subscribe; pods become `Loading`; re-run `review_access(scope)` (old task dropped, so aborted) | `AccessState` `Checking`, then `Known`/`Unknown` |
| `retry()` (button in `Failed`) | `connect` again with the same inputs | |
| context switch (`AppShell`) | drop the session entity; clear selection; close the drawer; `connect` the new context | all old tasks abort |

## Initial namespace (pure `fn initial_scope`)

```rust
/// `all_namespaces_access` is None when `review_access(All)` failed.
fn initial_scope(requested: Option<&str>, all_namespaces_access: Option<&AccessReport>,
    default_namespace: &str) -> NamespaceScope;
```

| Input | Scope | Access review |
|---|---|---|
| `--namespace ns` | `Named(ns)` | `review_access(Named(ns))` only |
| none; `review_access(All)` allows `ListPods` | `All` | the `All` report is reused |
| none; `ListPods` denied for `All` | `Named(connection.default_namespace())` | then `review_access(Named(..))` |
| none; `review_access(All)` errored | `Named(connection.default_namespace())` (needs the fewest rights) | `Unknown { message }` |

- `default_namespace()` is the context namespace, or `default` (0001).

## Notes

- `Failed` messages are `ClusterError`/`KubeconfigError` `Display` strings, plus the first `source()` line when present. 0001 guarantees these hold no secrets, so they may be shown.
- `user` is the kubeconfig user **entry name** (`ContextSummary.user`), not a credential. It appears in the status bar.
- A closed watch receiver before the first snapshot sets the list to `Failed { message: "watch stopped unexpectedly" }`.
