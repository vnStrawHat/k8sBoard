# Project-specific rules

## 2. Project-specific rules

- **English-only** in code, comments, docs, commit messages, and UI strings.
- **Mutating Kubernetes calls only through the spec 0030 write path.** create/update/patch/delete go only through `crates/cluster/src/object_write.rs` (its `WriteOperation` allow-list). exec/attach/portforward go only through the named connect files (`pod_shell.rs`, `port_forward.rs`, `debug_shell.rs`). clippy `disallowed-methods` enforces this, with the exceptions named in 0030. The user approved all mutating specs on 2026-10-02. Debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1`; agents set it only with the kind lab kubeconfig `kind-lab.yml` (see `tools/kind/README.md`), never with the UAT one. Non-mutating review APIs (SelfSubjectAccessReview / SelfSubjectRulesReview) are allowed.
- **Never log or display kubeconfig secrets** (tokens, client keys, certificate data). Secret values are hidden by default in the UI.
- **Never block the GPUI main thread on network I/O.** kube-rs runs on a dedicated tokio runtime; results reach the UI through channels and `cx.spawn`; batch watch events before `cx.notify()`.
- **UI colors come from the GPUI Kit theme** (no hardcoded colors); semantic status colors use named theme tokens.
- **Terminal emulation** uses `oneterm-vt` (git dependency pinned to a commit, `default-features = false`), not `alacritty_terminal`.
- **Errors:** libraries use `thiserror` domain errors; the `k8sboard` binary may use `anyhow` at the top level.
- **Quality gate** before reporting work done:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
- **Shell/tooling rules for agents:**
  - Use Git Bash + coreutils (no PowerShell).
  - Scoped searches only (always target an explicit folder inside the project).
  - Stay inside the project folder: temp files go to `.tmp/`, caches to `.cargo-home/`.
  - Export `CARGO_HOME`, `TMP`, `TEMP`, `TMPDIR` (see `CLAUDE.md`) before any cargo command.
