# k8sBoard

Rust desktop app for administering Kubernetes clusters.

- UI: GPUI via GPUI Kit (`gpui-kit`)
- Kubernetes: `kube` (kube-rs) + `k8s-openapi`, on a dedicated tokio runtime
- Design: wireframes in `docs/k8sboard-wireframes.html`

## Crate layout

- `crates/cluster` — package `k8sboard-cluster`, lib `cluster`: kubeconfig loading, context selection, read-only cluster access.
- `crates/app` — package/binary `k8sboard`: GPUI Kit application shell.

Cargo virtual workspace at the root; shared versions, lints, and profiles live in the root `Cargo.toml`.

## Dev kubeconfig

- Use `monitor-uat-readonly.yml` (project root) with the explicit context `readonly@Monitor`. Its current-context `readonly@cluster.local` is invalid.
- It is read-only and contains a bearer token. NEVER print, cat, echo, or quote it. It is git-ignored.
- The UAT cluster runs Kubernetes v1.29.5.

## Environment rules

- Work only inside this project folder. Temp files go to `.tmp/`, caches to `.cargo-home/`. Do not use git worktrees.
- Use Git Bash + GNU coreutils for shell work, never PowerShell.
- Every find/grep/ls must target an explicit folder inside the project; never scan a drive.
- Before every cargo command, export in the same Bash call:

```bash
export CARGO_HOME="D:/TrungKFC-Research/Rust/k8sBoard/.cargo-home" TMP="D:/TrungKFC-Research/Rust/k8sBoard/.tmp" TEMP="D:/TrungKFC-Research/Rust/k8sBoard/.tmp" TMPDIR="D:/TrungKFC-Research/Rust/k8sBoard/.tmp"
```

- Kubernetes mutating calls go only through the `object_write.rs` allow-list and the named connect files (`pod_shell.rs`, `port_forward.rs`, `debug_shell.rs`), as specified in spec 0030; the user approved all mutating specs on 2026-10-02. Debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1`; agents never set it, and live checks on the read-only UAT cluster stay on the denied path. Do not commit unless asked.
- Keep docs short: split by topic into files ≤ ~120 lines with a README.md index.

## Quality gate

Before reporting work done:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Agents

The main session only orchestrates; see `docs/agents/workflow.md`. Agents live in `.claude/agents/`:

- `architect` (opus) — designs, writes specs to `docs/specs/NNNN-title/` folders; docs-only edits.
- `coder` (sonnet, high) — implements specs with tests and the quality gate.
- `coder-lite` (sonnet, low) — mechanical tasks, gate runs, read-only probes.
- `advisor` (fable) — read-only second-opinion review of specs and diffs.
- `ui-verifier` (sonnet, medium) — launches the app, screenshots it, and checks UI against the wireframes; read-only.

## Code style

@docs/agents/code-style/README.md
@docs/agents/code-style/project-rules.md
@docs/agents/code-style/format-and-naming.md
@docs/agents/code-style/structure.md
@docs/agents/code-style/design.md
@docs/agents/code-style/errors-async-docs.md
@docs/agents/code-style/testing.md
