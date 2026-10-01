# Project-specific rules

## 2. Project-specific rules

- **English-only** in code, comments, docs, commit messages, and UI strings.
- **Kubernetes access is read-only for now.** Production code must not call create/update/patch/delete/exec/attach/portforward APIs until a feature explicitly requires it and the user approves. Non-mutating review APIs (SelfSubjectAccessReview / SelfSubjectRulesReview) are allowed.
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
