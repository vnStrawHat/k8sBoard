# Releasing k8sBoard

## Cut a release

1. GitHub: Actions -> Release -> Run workflow on `main`, choose the bump level (`patch`, `minor`, or `major`; default `patch`).
2. The workflow runs in this order:
   1. `prepare` reads the version from the root `Cargo.toml`, computes the next one, and fails if the tag `vX.Y.Z` already exists.
   2. `build` bumps the version **locally only** (so the binary carries it), builds the Windows binary, and packages the archive. Nothing is committed or tagged yet.
   3. `publish` runs only if `build` succeeded. It generates the notes with git-cliff, bumps the version again, downloads the archive, computes `SHA256SUMS.txt`, and commits `chore(release): vX.Y.Z` (`Cargo.toml` and `Cargo.lock`) locally as `github-actions[bot]`. Only then does it push the commit to `main`, and `gh release create --target <commit>` creates the tag and the published (not draft) release in one call. If that fails, the commit is rolled back and no tag exists.
3. Check the release page: notes, archive, `SHA256SUMS.txt`.

A failed `prepare` or `build` leaves the repository untouched: fix the problem and run the workflow again.

### Recovery if `publish` fails

A failed `publish` normally leaves nothing (the commit is rolled back and no tag exists), so run the workflow again.

If the rollback itself was rejected (branch protection forbids force-push), `main` holds `chore(release): vX.Y.Z` with no tag. Either delete that commit by hand, or create the release by hand from the failed run's `k8sboard-windows-x86_64` artifact:

```bash
sha256sum k8sboard-X.Y.Z-windows-x86_64.zip > SHA256SUMS.txt
git-cliff --unreleased --tag vX.Y.Z --strip header > RELEASE_NOTES.md   # or write the notes by hand
gh release create vX.Y.Z --target <commit> --title "k8sBoard vX.Y.Z" --notes-file RELEASE_NOTES.md k8sboard-X.Y.Z-windows-x86_64.zip SHA256SUMS.txt
```

### Requirements

- A push made with `GITHUB_TOKEN` does not start other workflows, so CI does not run on the release commit.
- Branch protection on `main` must allow `github-actions[bot]` to push directly.
- The rollback needs force-push permission for `github-actions[bot]` on `main`.

## Commit convention

The release notes are generated from commit subjects, so every commit on `main` uses `type(scope): subject` (scope optional):

| Type | Notes group |
| --- | --- |
| `feat` | Features |
| `fix` | Bug fixes |
| `perf` | Performance |
| `refactor` | Refactoring |
| `docs` | Documentation |
| `build`, `ci` | Build and CI |
| `chore`, anything else | Other |

A `!` after the type or scope (`feat(app)!: ...`) or a `BREAKING CHANGE` footer puts the commit in a "Breaking changes" group listed first. `chore(release)` commits are skipped. The grouping lives in `cliff.toml`.

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
