# Design, ownership, and performance

### Types and design

* Prefer domain-specific types over primitive values whenever practical.
* Prefer structs over tuples for structured data.
* Prefer enums over boolean parameters.

  Prefer:

      enum SaveMode {
          Normal,
          Force,
      }

  Instead of:

      save(force: bool)

* Prefer composition over unnecessary abstraction.
* Avoid introducing traits until multiple implementations or generic behavior are required.
* Constructors should return fully initialized, valid objects.
* Avoid partially initialized state.
* Group related parameters into configuration structs when function signatures become difficult to understand.
* Small duplication is preferable to premature abstraction.

### Functions

* Keep functions focused on one responsibility.
* Extract helper functions when logical steps have meaningful names.
* Prefer early returns over deeply nested control flow.

  Prefer:

      if !condition {
          return;
      }

      do_work();

* Prefer pattern matching when working with enums or state machines.
* Avoid long parameter lists. Introduce a domain type when appropriate.
* Prefer explicit control flow over clever one-liners.

### Ownership and borrowing

* Prefer borrowing over cloning.
* Clone only when ownership requires it or when it significantly simplifies the implementation.
* Keep mutable borrows as short as possible.
* Prefer immutable data. Keep mutable state localized.
* Avoid unnecessary shared ownership with `Rc` or `Arc`.
* Pass dependencies explicitly rather than relying on global state.

### Collections

* Choose collection types intentionally based on access patterns.
* Prefer iterator adapters when they improve readability.
* Prefer explicit loops when iterator chains become difficult to understand.
* Avoid allocating intermediate collections unless necessary.
* Prefer `HashMap`, `BTreeMap`, `Vec`, and other standard collections unless a specialized collection provides clear value.

### State management

* Minimize mutable state.
* Keep state ownership obvious.
* Avoid global mutable state.
* Prefer explicit state transitions over implicit side effects.
* Make invalid states difficult to represent.

### Performance

* Measure before optimizing.
* Prefer readable code over micro-optimizations.
* Optimize algorithms before optimizing syntax.
* Avoid unnecessary allocations and cloning in hot paths.

