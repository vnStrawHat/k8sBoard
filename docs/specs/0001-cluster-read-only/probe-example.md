# 0001 · Probe example (`crates/cluster/examples/probe.rs`)

[Back to index](README.md)

```text
cargo run -p k8sboard-cluster --example probe -- --kubeconfig <path> [--context <name>] [--namespace <name>]
```

## CLI

| Flag | Meaning |
|---|---|
| `--kubeconfig <path>` | required |
| `--context <name>` | optional; when omitted, the probe uses `resolve_context(None)` |
| `--namespace <name>` | optional; sets the `NamespaceScope` for pods and access review (default `All`) |
| `--help` | prints usage |

- Arguments are parsed by hand from `std::env::args()`, with no new dependency.
- An unknown flag or a missing value prints usage to stderr and exits with code 2.
- The probe uses `#[tokio::main]` (the `macros` and `rt-multi-thread` features are already enabled).
- Output goes through `writeln!` on a locked `std::io::stdout()`, because the workspace lint `clippy::print_stdout` rejects `println!`. Do not add an `#[allow]` for it.
- Errors go to stderr via `eprintln!`, followed by the full `source()` chain, one `caused by:` line per source.

## Sections (in order)

Every section after 2 runs even if an earlier section failed. A failed section prints `error: …` and marks the run as failed.

1. `kubeconfig`: the path, then one line per context: `name  cluster=…  user=…  namespace=…`. A `*` marks the current context, and the raw current-context value is printed.
2. `context`: the resolved context. If resolution fails with a `KubeconfigError`, print the chain and exit 1.
3. `server`: `git_version` and `platform`.
4. `metrics.k8s.io`: `available <gv>`, `unavailable <gv>: <reason>`, or `not installed`.
5. `access (<scope>)`: nine rows, each `allowed`, or `denied` plus the reason when one is given.
6. `namespaces (<n>)`: `NAME  PHASE  CREATED`.
7. `nodes (<n>)`: `NAME  STATUS  ROLES  TAINTS  VERSION  INTERNAL-IP  CREATED`.
   - STATUS is the readiness, plus `,SchedulingDisabled` when cordoned.
   - ROLES and TAINTS are comma-joined, or `<none>` when empty.
8. `pods (<n>)`:
   - a histogram of `PodStatus` display text, sorted by count descending, then by text;
   - the first 30 pods as `NAMESPACE  NAME  STATUS  READY  RESTARTS  NODE  CREATED`;
   - for up to 20 pods whose status is not `Running` or `Completed`, one indented line per container: `kind  name  state  ready  restarts  last-termination`.

Timestamps print as RFC 3339 (`jiff::Timestamp` `Display`).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | every section succeeded |
| 1 | context resolution failed, or any later section failed |
| 2 | usage error |

## Never print credentials

- Never print tokens, certificate or key data, passwords, exec arguments, or `Debug` output of kube types.
- Print only `ContextSummary` fields, domain summaries, and domain errors with their `source()` chain.
- The server address is **not** a secret. It may appear in the cause chain, for example as host:port in an `Unreachable` error.
- Acceptance criterion 7 checks the output for credential values only.
