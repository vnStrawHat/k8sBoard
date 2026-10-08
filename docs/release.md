# Releasing k8sBoard

## Cut a release

1. Bump `version` under `[workspace.package]` in the root `Cargo.toml` (and refresh `Cargo.lock`), commit, and merge to `main`.
2. Tag that commit and push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. The `Release` workflow builds the Windows archive and creates a **draft** GitHub release with generated notes.
4. Review the draft (notes, archives, `SHA256SUMS.txt`) and publish it.

The tag must equal the workspace version (`v0.1.0` for `0.1.0`); the `check-version` job fails the run otherwise. A failed run leaves no release, so fix the problem, delete the tag, and tag again.

## What CI checks

`.github/workflows/ci.yml` runs on every push to `main` and every pull request, on Windows only (Linux and macOS come later):

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p k8sboard --all-targets --features screenshot -- -D warnings`
- `cargo test --workspace --no-fail-fast`

## Artifacts

Built with `cargo build --release -p k8sboard --locked` (default features; `screenshot` and `lab-writes` never ship):

- `k8sboard-X.Y.Z-windows-x86_64.zip` (`k8sboard.exe`)
- `SHA256SUMS.txt`

Linux and macOS archives come later (the former matrix entries are kept as comments in `release.yml`). The archive also holds `third-party/Lilex-OFL.txt`, the licence of the embedded Lilex fonts. The top-level `LICENSE` (Apache-2.0) is included too.

## Known gaps

- Windows only for now. Linux is unverified: the code has never been built there, so the first Linux run may fail until the apt package list in both workflows is adjusted. macOS has no `.app` bundle yet.
- The Windows binary is unsigned (SmartScreen will warn); macOS would also need notarization.
