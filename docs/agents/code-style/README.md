# Code Conventions — k8sBoard

> Every rule here is **mandatory** when writing Rust for the k8sBoard project.
Rules are split by topic:

- [format-and-naming.md](format-and-naming.md) — formatting, imports, English-only, naming.
- [project-rules.md](project-rules.md) — read-only Kubernetes, secrets, main thread, theme colors, terminal, errors, quality gate, tooling.
- [structure.md](structure.md) — general principles, crate/module/file organization, public APIs.
- [design.md](design.md) — types, functions, ownership, collections, state, performance.
- [errors-async-docs.md](errors-async-docs.md) — error handling, async and concurrency, documentation and comments.
- [testing.md](testing.md) — unit and integration test rules.

### Final checklist

* Before considering work complete, ensure:

  - `cargo fmt` passes.
  - `cargo clippy` passes without introducing unnecessary `#[allow]` attributes.
  - `cargo test` passes.
  - New behavior is covered by tests.
  - New code follows existing crate conventions.
  - Public APIs remain minimal.
  - No unnecessary files, modules, or abstractions were introduced.

