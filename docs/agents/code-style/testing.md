# Testing

### Testing

* Add tests for every new behavior.
* Add a regression test for every bug fix.
* Keep unit tests in the same module as the code they verify.

  Prefer:

      resource_table.rs
      resource_table_tests.rs

  or, for small modules:

      resource_table.rs
          #[cfg(test)]
          mod tests { ... }

  Instead of:

      tests/
          resource_table.rs

* Unit tests should remain part of the module they test so they can access private items (`use super::*`) without expanding the crate's public API.
* Do not change item visibility (`pub`, `pub(crate)`, etc.) solely to make code testable.
* Keep production files focused. When unit tests become substantial, move them into a sibling `*_tests.rs` file using:

      // resource_table.rs
      #[cfg(test)]
      mod resource_table_tests;

* Use the `tests/` directory only for integration tests that exercise the crate through its public API.
* Integration tests should verify interactions between modules/crates rather than internal implementation details.
* Organize tests by feature or module. Avoid large catch-all test files.
* Each test should verify one behavior.
* Prefer deterministic tests.
* Extract reusable fixtures only after they become repetitive.

