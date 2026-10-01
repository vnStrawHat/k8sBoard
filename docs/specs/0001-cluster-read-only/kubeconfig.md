# 0001 · Kubeconfig: loading, contexts, resolution

[Back to index](README.md) · Module: `src/kubeconfig.rs`

## Behavior

| Topic | Rule |
|---|---|
| Load | `Kubeconfig::load(path)` calls `kube::config::Kubeconfig::read_from(path)`, which is synchronous `std::fs` I/O. Do **not** replace it with `read_to_string` + `from_yaml`: `read_from` makes relative `certificate-authority`, `client-certificate`, `client-key`, `tokenFile`, and exec command paths absolute. |
| Shared constructor | `load` and the unit tests both go through a private `Kubeconfig::from_document(path, document)`. |
| Contexts | Computed once at load, in file order. Entries without a `context:` body are skipped. Duplicate names keep the first occurrence, which matches kube's lookup. |
| `current_context()` | Returns the raw `current-context` value. An empty string is treated as unset. The value may name a missing context, as the UAT file does. |
| `resolve_context(Some(name))` | That context, else `ContextNotFound { origin: Requested }`. |
| `resolve_context(None)` | Uses `current-context`, else `ContextNotFound { origin: CurrentContext }`. If there is no current-context, returns `NoContextSelected`. |
| Available list | Both errors list context names in file order. An empty list renders as `(none)`. |
| UAT case | No `--context`: `current-context 'readonly@cluster.local' not found …; available contexts: readonly@Monitor`. |

## Error mapping

The private function `load_error(path: &Path, error: kube::config::KubeconfigError) -> KubeconfigError` maps load failures:

| kube error | Domain error |
|---|---|
| `ReadConfig(io, _)` | `Read { path, source: io }` |
| any other variant (YAML parse, multi-document merge mismatch) | `Parse { path }` with **no source**. The parser message can quote the failing line, and that line may hold a token. |

## Credential hygiene

- `Kubeconfig` implements `Debug` by hand and prints only the path and context names.
- `ContextSummary` holds names only. `cluster` and `user` are kubeconfig entry names, never a URL or credentials.

## Public API

```rust
/// A kubeconfig file loaded from an explicit path. Holds credentials.
#[derive(Clone)] // Debug is manual: path + context names only
pub struct Kubeconfig { /* path: PathBuf, document: kube::config::Kubeconfig, contexts: Vec<ContextSummary> */ }

impl Kubeconfig {
    /// Blocking file I/O: call it off the UI thread.
    pub fn load(path: &Path) -> Result<Self, KubeconfigError>;
    pub fn path(&self) -> &Path;
    pub fn contexts(&self) -> &[ContextSummary];
    pub fn current_context(&self) -> Option<&str>;
    pub fn resolve_context(&self, requested: Option<&str>) -> Result<&ContextSummary, KubeconfigError>;
}

pub struct ContextSummary {
    pub name: String,
    pub cluster: String,          // kubeconfig cluster entry name
    pub user: Option<String>,     // kubeconfig user entry name
    pub namespace: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextOrigin { Requested, CurrentContext } // Display: "context" / "current-context"

#[derive(Debug, thiserror::Error)]
pub enum KubeconfigError {
    #[error("cannot read kubeconfig '{}'", .path.display())]
    Read { path: PathBuf, #[source] source: std::io::Error },
    #[error("kubeconfig '{}' is not a valid kubeconfig YAML document", .path.display())]
    Parse { path: PathBuf },
    #[error("{origin} '{requested}' not found in kubeconfig '{}'; available contexts: {}",
            .path.display(), context_list(.available))]
    ContextNotFound { path: PathBuf, requested: String, origin: ContextOrigin, available: Vec<String> },
    #[error("kubeconfig '{}' has no current-context and no context was requested; available contexts: {}",
            .path.display(), context_list(.available))]
    NoContextSelected { path: PathBuf, available: Vec<String> },
}

fn context_list(names: &[String]) -> String; // private: ", "-joined, or "(none)"
```

## Out of scope

- `$KUBECONFIG` and `~/.kube/config` discovery.
- Merging several files.
- Folder watching.
- Cloud scans (W2).
- In-cluster configuration.
