# 0038 · `helm` command (cluster crate)

[Back to index](README.md) · Step 1 · Modules: `helm_command.rs` (new) + `helm_command_tests.rs`, `dns_name.rs` (new, moved), `kubelet_stats.rs`, `connection.rs`, `access_review.rs`, `lib.rs`, `Cargo.toml`, root `clippy.toml`. Decisions 4–12.

## Verified APIs

| API | Where |
|---|---|
| `tokio::process::Command::{env, env_remove, kill_on_drop, creation_flags (windows), output}` | `tokio-1.53.1/src/process/mod.rs:444, 504, 664, 675` (feature `process`; no new package, coder confirms `Cargo.lock`) |
| `helm rollback <RELEASE> [REVISION]`, `--dry-run` (v3 bool; v4 `none|client|server`), `--timeout 5m0s` per operation | `helm` release-3.19 `cmd/helm/rollback.go`; helm.sh docs v4.3.0 |
| `helm uninstall RELEASE_NAME`, `--dry-run` (both), `--timeout 5m0s` | helm.sh docs v4.3.0 |
| History pruning on every new record: `Storage.Create` → `removeLeastRecent`; a failed delete aborts the create | `helm` release-3.19 `pkg/storage/storage.go` |

## API (`pub use` in `lib.rs`; no kube type in a signature)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmCli { path: PathBuf /* absolute */, version: HelmVersion }  // only `detect_helm` builds it
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct HelmVersion { pub major: u8, pub minor: u16, pub patch: u16 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelmCliState { Found(HelmCli), Missing, Unsupported { version: String } }
pub async fn detect_helm() -> HelmCliState;          // resolve on PATH, then `helm version --template {{.Version}}`, 5 s
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum HelmAction { Rollback { to: u32 }, Uninstall }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmRequest { namespace: String, release: String, from: u32, action: HelmAction }
impl HelmRequest {
    /// `None` unless `is_dns_label(namespace)`, `is_dns_subdomain(release)` with ≤ 53 chars,
    /// and a rollback target lower than `from`.
    pub fn new(namespace: &str, release: &str, from: u32, action: HelmAction) -> Option<Self>;
    pub fn access_checks(&self) -> &'static [AccessCheck];                  // decision 16
    pub fn commit_args(&self, cli: &HelmCli, context: &str) -> Vec<String>; // `helm` + the commit argv; the app quotes it
}
pub struct HelmOutcome { pub mode: WriteMode, pub elapsed: Duration }
impl ClusterConnection {
    /// The only way the app runs helm. Called only from `write_flow.rs`.
    pub async fn helm(&self, cli: &HelmCli, request: &HelmRequest, mode: WriteMode) -> Result<HelmOutcome, HelmError>;
}
```

- `dns_name.rs` (new, `pub(crate)`): `is_dns_label` and `is_dns_subdomain` (today's `kubelet_stats::is_node_name`, renamed), moved with their tests; `kubelet_stats.rs` imports them.
- `ClusterConnection` gains `kubeconfig_files: Arc<[PathBuf]>` (0024 `Kubeconfig` sources) and `has_proxy_url: bool`, both set in `open` (paths and a flag; no secret).

## Resolving `helm` (pure `resolve_helm(path_var: Option<&OsStr>, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf>`)

- Walks `std::env::split_paths(PATH)` in order; skips empty and relative entries. Candidate per entry: `helm` on Unix, `helm.exe` on Windows; **never** `.bat` or `.cmd` (CVE-2024-24576 argument injection), never the executable's directory or the current directory.
- The first `is_file` hit is the absolute path stored in `HelmCli`; every spawn (detection included) uses it. No match → `Missing`.

## argv (pure `helm_args(cli, request, context, mode) -> Vec<OsString>`)

| Action | Mode | v3 | v4 |
|---|---|---|---|
| Rollback | DryRun | `rollback {r} {to} --namespace={ns} --kube-context={ctx} --dry-run` | same with `--dry-run=client` |
| Rollback | Commit | `rollback {r} {to} --namespace={ns} --kube-context={ctx}` | same |
| Uninstall | DryRun | `uninstall {r} --namespace={ns} --kube-context={ctx} --dry-run` | same |
| Uninstall | Commit | `uninstall {r} --namespace={ns} --kube-context={ctx}` | same |

Flag values use the `--flag=value` form, so a value can never be read as a flag. Never `--debug`, `--set`, `--values`, `--kubeconfig`, `--kube-token`, or any other flag. Arguments are separate `OsString`s, never through a shell. The dry-run checks only the release record (helm 4 `--dry-run=server` on rollback also returns before any Kubernetes call, so it adds nothing; decision 7).

## Environment (pure `helm_env(files, has_proxy_url) -> HelmEnv { set, remove }`, applied to every spawn, detection included)

| Variable | Rule |
|---|---|
| `KUBECONFIG` | `files` joined with `std::env::join_paths` (`;` on Windows, `:` elsewhere); detection sets none |
| `HELM_DRIVER` | `secret` (0017 reads only Secret storage) |
| `HELM_NO_PLUGINS` | `1`: no plugin code runs inside our helm calls |
| removed | `HELM_KUBECONTEXT`, `HELM_NAMESPACE`, `HELM_KUBETOKEN`, `HELM_KUBEAPISERVER`, `HELM_KUBEASUSER`, `HELM_KUBEASGROUPS`, `HELM_KUBECAFILE`, `HELM_KUBEINSECURE_SKIP_TLS_VERIFY`, `HELM_KUBETLS_SERVER_NAME`, `HELM_DEBUG` |
| removed unless `has_proxy_url` | `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy`, `ALL_PROXY`, `all_proxy`, `NO_PROXY`, `no_proxy` (mirror of `connection.rs` `proxy_for`) |
| everything else | inherited (exec auth plugins such as `aws`, `gke-gcloud-auth-plugin`, `kubelogin` need it) |

## Process (private `run_with`, generic over the spawner)

`fn run_with<S>(spawn: S, invocation: HelmInvocation) -> impl Future<Output = Result<ProcessResult, ProcessFailure>>`. Production `S`: `tokio::process::Command::new(&cli.path)`, args, env, `stdin(null)`, then **`output()`**, which drains stdout and stderr concurrently (a bounded read of one pipe would block helm on the other). Each output is truncated to its first 64 KiB **after** collection. On Windows `.creation_flags(CREATE_NO_WINDOW)` with `const CREATE_NO_WINDOW: u32 = 0x0800_0000;` inline, `#[cfg(windows)]` on that call only; no `windows-sys` dependency. Tests pass a closure returning scripted results.

| Invocation | `kill_on_drop` | Cap | At the cap |
|---|---|---|---|
| detect | `true` | 5 s | killed; `Unsupported { "no answer" }` |
| DryRun | `true` | 2 min | killed; `Failed` |
| Commit | **`false`** | 15 min | **not killed**: `OutcomeUnknown`; the `output()` future moves to a detached `tokio::spawn` that keeps draining until helm exits, then drops the result |

Killing a committing helm would leave a `pending-rollback` / `uninstalling` record stuck and block later operations; dropping the pipes would kill it too (SIGPIPE on Unix). That detached drain is the only `spawn` in the module. Ceiling: helm's `--timeout 5m0s` applies per operation (each hook, each wait), so a commit may outlive 15 min; the result is then unknown and History shows the truth.

Order in `helm`: policy `Blocked` → `WritesBlocked` (nothing spawned) → spawn → classify.

## Errors (`HelmError`, `thiserror`; pure `classify(mode, exit, stderr)`)

| Variant | When | `Display` |
|---|---|---|
| `WritesBlocked` | debug build, no opt-in | 0030 text |
| `Missing` | resolution found nothing, or spawn `NotFound` | `helm was not found on PATH` |
| `Denied { message }` | stderr has `forbidden` | `not permitted: {message}` |
| `NotFound` | `release: not found`, `has no deployed releases` | `the release no longer exists` |
| `Busy` | `another operation (install/upgrade/rollback) is in progress` | `another Helm operation is in progress` |
| `OutcomeUnknown` | Commit cap reached, or a pipe broke after spawn | `no answer in time; helm may have applied the change` |
| `Failed { code, message }` | other non-zero exit | DryRun: `helm dry-run failed: {message}`; Commit: `helm reported a failure; some resources may have changed: {message}` |
| `Spawn { kind }` | other spawn errors | `helm could not be started ({kind})` |

`message` (pure `error_line`): the last line starting with `Error:` (helm 3), else the `error="…"` value of the last slog line with `level=ERROR` (helm 4), else the last non-empty line; control characters stripped; ≤ 500 chars. Both shapes have fixtures (open item 3).

## Safety

- No `tracing::` call receives stdout, stderr, the environment, or argv values beyond release, namespace, and context names; traces carry action, release, namespace, mode, exit code, elapsed.
- Rollback and uninstall pass no values: chart values never leave the cluster through k8sBoard (C1). `HelmError` has a manual `Debug` that omits `message`.
- The 0030 allow-list table gains rows `helm rollback` and `helm uninstall` (external process, dry-run yes, 0038). `clippy.toml` `disallowed-methods` gains `std::process::Command::new` and `tokio::process::Command::new` (reason "processes run only through helm_command.rs"); the one exception is the production spawner. 0030 README AC 1 then lists four named exceptions (SSAR, `object_write.rs`, kubelet GETs, `helm_command.rs`), plus the connect call sites of 0035–0037.
- New `AccessCheck`s (namespaced, core): `CreateSecrets`, `UpdateSecrets`, `DeleteSecrets`.
