# 0001 — Cluster crate: read-only foundation

Status: approved for implementation (user decisions and advisor review applied). Crate: `crates/cluster` (package `k8sboard-cluster`, lib `cluster`). Wireframes: W1–W5.

## Goal

Give the future GPUI app one small, read-only entry point to a Kubernetes cluster. It loads a kubeconfig from an explicit path, lists and resolves contexts with actionable errors, and opens a connection. One-shot queries return UI-shaped domain types: server version, namespaces, nodes (W5), and pods with containers (W4/W4b). It also probes `metrics.k8s.io` and nine RBAC permissions. A `probe` example validates everything against UAT without printing credentials. Pod and node status logic is pure, unit-tested, and matches kubectl 1.32.

## Non-goals

- Watches and reflectors (0002; see [async-contract.md](async-contract.md)), metric values, and any change to `crates/app`.
- **No mutating API.** That rules out logs, exec, attach, port-forward, node shell, cordon, drain, delete, and edit.
- Kubeconfig discovery or merging, cloud scans, in-cluster config, and display formatting (ages, the "—" placeholder, colors, "Readiness failed").
- Ephemeral containers, images, ports, probes, resources, events, and owner references.

## Files

| File | Contents |
|---|---|
| [files-to-touch.md](files-to-touch.md) | module layout, `lib.rs`, Cargo changes, API-wide rules |
| [kubeconfig.md](kubeconfig.md) | loading, contexts, resolution, `KubeconfigError` |
| [connection.md](connection.md) | `ClusterConnection`, server version, timeouts, paging, `ClusterError` mapping |
| [namespaces-and-nodes.md](namespaces-and-nodes.md) | `NamespaceScope`, namespace and node summaries (W5) |
| [pods.md](pods.md) | pod and container summaries, ready column (W4/W4b) |
| [pod-status.md](pod-status.md) | kubectl 1.32 status algorithm and typed mapping |
| [access-and-capabilities.md](access-and-capabilities.md) | SSAR access probe, `metrics.k8s.io` discovery |
| [async-contract.md](async-contract.md) | runtime ownership, Send/Sync, cancellation, room for 0002 |
| [probe-example.md](probe-example.md) | `examples/probe.rs` CLI and output |
| [test-plan.md](test-plan.md) | fixture; kubeconfig, connection, namespace, node, access, metrics, integration tests |
| [test-plan-pods.md](test-plan-pods.md) | pod and pod-status tests |

## Acceptance criteria

- [x] 1. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` pass, with no new `#[allow]`.
- [ ] 2. Every test named in both test-plan files exists under that name and passes.
- [ ] 3. `lib.rs` matches [files-to-touch.md](files-to-touch.md), and nothing else is `pub`.
- [ ] 4. Read-only guard: — superseded by spec 0030 (mutating calls are allowed through the object_write.rs allow-list)
  - The only calls of a mutating or raw `kube` method are the ones named in the 0030 exception table ([write-path.md](../0030-guardrails-write-path/write-path.md)): clippy `disallowed-methods` (root `clippy.toml`) fails on any other. The SelfSubjectAccessReview and SelfSubjectRulesReview `create` (non-mutating review objects, `Api::<SelfSubject…Review>::all`) are the `access_review.rs` row. The old scoped grep of `crates/cluster/{src,examples}` stays as documentation: it lists `access_review.rs` and `object_write.rs` (and one `store.delete` of the 0002 reflector store, which is not a `kube` call).
  - No `kube` or `k8s_openapi` type appears in a public signature.
  - `kube` features in `crates/cluster/Cargo.toml` and the root `Cargo.toml` do not include `ws`.
- [ ] 5. `probe -- --kubeconfig monitor-uat-readonly.yml` (no `--context`) exits 1. Its stderr names `current-context 'readonly@cluster.local' not found` and lists `readonly@Monitor`.
- [ ] 6. With `--context readonly@Monitor`, the probe prints:
  - `v1.29.5`, a `metrics.k8s.io` line, and a nine-row access table;
  - the namespaces, nodes, and pods sections, or an `error:` line naming `Forbidden` for any section RBAC denies.

  coder-lite reports the access matrix and the metrics state verbatim.
- [ ] 7. No credential values in the probe's stdout or stderr. A script checks this with `grep -F -c`, loading the values into variables without printing them, and reports counts only. Every count must be 0.
- [x] 8. `crates/app` is unchanged, and the workspace builds.

## Decisions (resolved)

1. Ready counts main + sidecar containers, like kubectl (`3/4`).
2. "Readiness failed" is a UI label. The crate returns `Running` with ready `0/1`.
3. With no role labels, `roles` is empty. The UI shows "—" and does not infer `worker`.
4. Auth-plugin error detail is suppressed. A later spec may add an allow-listed hint, for example "run aws sso login".
5. Timeouts are fixed at 10 s connect and 30 s request. Per-cluster settings come with the Settings spec.

## Open items

None blocking. Moved to later specs: the redacted auth hint, per-cluster timeouts, the "Readiness failed" rule (pods view), and app runtime wiring (app spec).
