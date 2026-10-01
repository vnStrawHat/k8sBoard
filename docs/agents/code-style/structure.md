# Structure and organization

### General principles

* Prioritize correctness, readability, and maintainability over cleverness or premature optimization.
* Write idiomatic Rust. Prefer standard library types and patterns whenever practical.
* Follow the existing architecture and coding style of the crate. Consistency is more valuable than personal preference.
* Prefer extending existing code over introducing new abstractions.
* Keep changes localized. Avoid unrelated refactoring while implementing a feature or fixing a bug.
* Optimize only after profiling identifies a real bottleneck.
* Windows-shaped strings are never built with `std::path`: `Path`/`PathBuf` use the *host's* separator and component rules, so a Windows path joined or split on the Linux or macOS CI runner comes out wrong — build and read it as a string with `\` spelled out.
* `#[cfg(windows)]` goes on the FFI call, not on the decision around it: a pure helper reachable only from a platform-gated body is dead code on the other runners, and its tests run on one OS instead of three. `#[allow(dead_code)]` there means the split is in the wrong place.
* A helper used only from `cfg(windows)` code is gated `#[cfg(any(windows, test))]` (so its tests still run on every OS) or given a cross-platform caller — never `#[allow(dead_code)]`.
* Do not preserve backward compatibility. Remove obsolete paths instead of adding compatibility layers, fallbacks, or migrations

### Crate organization

* A crate should represent a single domain or capability.

  Prefer:

      cluster
      kubeconfig
      workload
      terminal

  Instead of:

      models
      services
      helpers

* Prefer extending an existing crate before creating a new one.
* Dependencies should flow in one direction. If two crates depend on each other, extract the shared functionality into a new crate.
* Keep crate boundaries clear. Avoid exposing implementation details across crates.
* Extract reusable functionality into a dedicated crate only after multiple crates need it.

### Module organization

* Organize modules around domain concepts rather than implementation details.

  Prefer:

      resource_table.rs
      resource_table_columns.rs
      resource_table_tests.rs

  Instead of:

      columns.rs
      tests.rs
      helpers.rs

* A module should represent one primary concept.
* Prefer extending an existing module before creating a new one.
* Avoid creating folders that contain only a single source file.
* Do not use `mod.rs` unless the module naturally contains multiple related files.
* Keep related types, implementations, helper functions, and tests close together.

### File organization

* A file should have one primary responsibility.
* Split files by concept rather than by implementation type.
* Split large files because responsibilities diverge, not because they exceed an arbitrary number of lines.

  Prefer:

      workload.rs
      workload_logs.rs
      workload_status.rs

  Instead of:

      workload_part1.rs
      workload_part2.rs

* Keep `lib.rs` and `mod.rs` focused on module declarations and public exports. Business logic belongs elsewhere.
* Prefer local helper functions over creating `helper.rs`, `common.rs`, or `utils.rs`.
* If helper code becomes reusable across multiple modules, extract a dedicated module with a descriptive name.

### Public APIs

* Keep public APIs intentionally small.
* Prefer private visibility by default.
* Prefer `pub(crate)` over `pub` when wider visibility is unnecessary.
* Design APIs that are difficult to misuse.
* Expose behavior instead of internal implementation details.

