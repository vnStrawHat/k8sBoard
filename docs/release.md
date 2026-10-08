# Releasing k8sBoard

## Cut a release

1. Bump `version` under `[workspace.package]` in the root `Cargo.toml` (and refresh `Cargo.lock`), commit, and merge to `main`.
2. Tag that commit and push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. The `Release` workflow builds the three archives and creates a **draft** GitHub release with generated notes.
4. Review the draft (notes, archives, `SHA256SUMS.txt`) and publish it.

The tag must equal the workspace version (`v0.1.0` for `0.1.0`); the `check-version` job fails the run otherwise. A failed run leaves no release, so fix the problem, delete the tag, and tag again.

## What CI checks

`.github/workflows/ci.yml` runs on every push to `main` and every pull request, on Windows, Ubuntu 22.04, and macOS:

- `cargo fmt --all -- --check` (Ubuntu only)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p k8sboard --all-targets --features screenshot -- -D warnings`
- `cargo test --workspace --no-fail-fast`

## Artifacts

Built with `cargo build --release -p k8sboard --locked` (default features; `screenshot` and `lab-writes` never ship):

- `k8sboard-X.Y.Z-windows-x86_64.zip` (`k8sboard.exe`)
- `k8sboard-X.Y.Z-macos-aarch64.tar.gz` (bare binary, no `.app` bundle yet)
- `k8sboard-X.Y.Z-linux-x86_64.tar.gz`
- `SHA256SUMS.txt`

Each archive also holds `third-party/Lilex-OFL.txt`, the licence of the embedded Lilex fonts. A top-level `LICENSE` is included once the repo has one (it has none yet, although `Cargo.toml` declares Apache-2.0).

## Known gaps

- Linux is unverified: the code has never been built there, so the first Linux run may fail until the apt package list in both workflows is adjusted.
- Binaries are unsigned and not notarized (Windows SmartScreen and macOS Gatekeeper will warn).
