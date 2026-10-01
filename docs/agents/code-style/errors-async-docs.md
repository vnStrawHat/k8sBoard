# Errors, async, and documentation

### Error handling

* Avoid `unwrap()`, `expect()`, and `panic!()` in production code.
* Prefer propagating errors with `?`.
* Add meaningful context when propagating errors.
* Return domain-specific errors whenever practical.
* Error messages should explain what failed and why.
* Handle errors where enough context exists to produce a useful message.

### Async and concurrency

* Keep async boundaries explicit.
* Spawn background tasks only when ownership and lifetime are well understood.
* Avoid holding locks across `.await`.
* Share immutable data whenever possible.
* Keep synchronization scopes as small as practical.

### Documentation and comments

* Comments should explain why, not what.

  Good:

      // The API server requires monotonically increasing resource versions.

  Bad:

      // Increment the counter.

* Document assumptions, invariants, and ownership expectations when they are not obvious.
* Every `unsafe` block must explain why it is safe and which invariants are maintained.
* Remove outdated comments when changing code.

