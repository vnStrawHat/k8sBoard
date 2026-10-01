# Format and naming

## 1. Style & format

- Run `cargo fmt` before committing. Config lives in `.rustfmt.toml`.
- `cargo clippy --workspace --all-targets -- -D warnings` **must pass** before merging.
- Naming: `snake_case` (function, variable, module), `PascalCase` (type, trait, enum variant), `SCREAMING_SNAKE_CASE` (const).
- **English-only** — all code comments, doc comments (`///`), and any written content in the codebase **must be in English**. Do not write Vietnamese (or any other non-English language) in code. This is a hard rule with zero exceptions.
- Group imports, separated by a blank line:

```rust
use std::sync::Arc;

use gpui_kit::*;

use crate::state::AppState;
```

### Naming

* Name files, modules, types, and functions after business concepts.
* Prefer descriptive names over generic names.

  Prefer:

      Cluster
      Kubeconfig
      Drawer
      PortForward

  Instead of:

      Manager
      Processor
      Helper
      Common
      Util

* Avoid abbreviations unless they are widely understood.
* Name functions after what they do, not how they do it.
* Name boolean variables so they read naturally in conditions.

