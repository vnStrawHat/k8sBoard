# 0001 · Connection: client, deadlines, paging, errors

[Back to index](README.md) · Module: `src/connection.rs`

## `ClusterConnection::open(kubeconfig, context)`

1. `kubeconfig.resolve_context(Some(context))?`. Unknown names fail as `KubeconfigError::ContextNotFound`.
2. `kube::Config::from_custom_kubeconfig(document.clone(), &KubeConfigOptions { context: Some(name), cluster: None, user: None }).await`. An error maps to `InvalidConfig`.
3. Set `config.connect_timeout = Some(CONNECT_TIMEOUT)`.
   - Do **not** set `read_timeout`: 0002 watches need long reads.
   - Do **not** call `apply_debug_overrides`.
4. `kube::Client::try_from(config)`. An error maps to `InvalidConfig`. This step spawns kube's `tower::Buffer` worker via `tokio::spawn`, so `open` is `async` and must be polled on the app's tokio runtime.
5. Store the client, the context name, and `config.default_namespace`.

`open` does no network I/O. It may read certificate or key files synchronously, which is fine on a tokio worker. The first query connects, so a bad server shows up as `Unreachable` there. W2 "Test connection" is `server_version()`, timed by the caller.

## Public API

```rust
#[derive(Clone)] // cheap; clones share the HTTP client. Debug is manual: context only.
pub struct ClusterConnection { /* client: kube::Client, context: String, default_namespace: String */ }

impl ClusterConnection {
    pub async fn open(kubeconfig: &Kubeconfig, context: &str) -> Result<Self, ClusterError>;
    pub fn context(&self) -> &str;
    pub fn default_namespace(&self) -> &str; // context namespace or "default"
    pub async fn server_version(&self) -> Result<ServerVersion, ClusterError>; // GET /version
}

pub struct ServerVersion {
    pub git_version: String, // "v1.29.5"
    pub platform: String,    // "linux/amd64"
}

type BoxError = Box<dyn std::error::Error + Send + Sync>; // spelled out in the real signatures

#[derive(Debug, thiserror::Error)]
pub enum ClusterError {
    #[error(transparent)] Kubeconfig(#[from] KubeconfigError),
    #[error("cannot build a client for context '{context}'")]
    InvalidConfig { context: String, #[source] source: BoxError },
    #[error("cannot reach the API server of context '{context}' while {action}")]
    Unreachable { context: String, action: &'static str, #[source] source: BoxError },
    #[error("the API server of context '{context}' did not answer in time while {action}")]
    TimedOut { context: String, action: &'static str },
    #[error("credentials for context '{context}' are unavailable while {action}")]
    CredentialsUnavailable { context: String, action: &'static str }, // detail dropped on purpose
    #[error("the API server rejected the credentials of context '{context}' while {action}: {message}")]
    Unauthorized { context: String, action: &'static str, message: String },   // 401
    #[error("context '{context}' is not allowed to perform {action}: {message}")]
    Forbidden { context: String, action: &'static str, message: String },      // 403
    #[error("the API server of context '{context}' failed {action} with HTTP {code}: {message}")]
    Api { context: String, action: &'static str, code: u16, message: String }, // other status
    #[error("unexpected response from the API server of context '{context}' while {action}")]
    UnexpectedResponse { context: String, action: &'static str, #[source] source: BoxError },
}
```

- `action` is a gerund phrase, for example `"listing pods"` or `"reading the server version"`.
- Message wording may be adjusted for grammar, as long as the context, action, and message stay in it.
- Each `Display` describes its own layer only. Callers walk `source()` for the rest.

## Private plumbing

```rust
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_PAGE_SIZE: u32 = 500;

impl ClusterConnection {
    /// One request under REQUEST_TIMEOUT; errors go through classify_error.
    pub(crate) async fn run<T>(&self, action: &'static str,
        request: impl Future<Output = Result<T, kube::Error>>) -> Result<T, ClusterError>;
    /// collect_pages with a closure: self.run(action, api.list(&params)) -> Page.
    pub(crate) async fn list_all<K>(&self, api: Api<K>, action: &'static str) -> Result<Vec<K>, ClusterError>
    where K: Clone + serde::de::DeserializeOwned + std::fmt::Debug; // no kube::Resource bound needed
    pub(crate) fn client(&self) -> &kube::Client;
}

struct Page<K> { items: Vec<K>, continue_token: Option<String> } // None or "" = last page

async fn collect_pages<K, F, Fut>(fetch_page: F) -> Result<Vec<K>, ClusterError>
where F: FnMut(ListParams) -> Fut, Fut: Future<Output = Result<Page<K>, ClusterError>>;

fn classify_error(context: &str, action: &'static str, error: kube::Error) -> ClusterError;
```

## Paging policy (`collect_pages`)

- Each request uses `ListParams::default().limit(LIST_PAGE_SIZE)`. Later pages also set `continue_token`. Items are appended in order.
- **410 on a page after the first** means the continue token expired. Discard the collected items and restart from page one **once**, like client-go's pager.
- A second 410, a 410 on the first page, and any other error are returned as-is. There are no partial results.

## Error mapping (`classify_error` and `open`)

| Source | `ClusterError` |
|---|---|
| `from_custom_kubeconfig` / `Client::try_from` error | `InvalidConfig` (boxed source) |
| `Error::Api(status)` with 401 / 403 / other | `Unauthorized` / `Forbidden` / `Api { code }`, with `message = status.message` |
| `Error::Auth(_)` | `CredentialsUnavailable`. No detail and nothing logged: exec output can hold secrets. |
| `HyperError`, `Service`, `RustlsTls`, `HttpError`, `ProxyProtocol*`, `TlsRequired` | `Unreachable` (boxed source; may name host:port, which is not secret) |
| everything else (catch-all arm) | `UnexpectedResponse` |
| `REQUEST_TIMEOUT` elapsed | `TimedOut` |

## Hygiene

- No `unwrap`, `expect`, or `panic!`.
- Never log `Config`, `AuthInfo`, the kubeconfig document, request headers, or `Error::Auth`.
- `tracing::debug!` with `context` + `action` only.
- Crypto provider: only `ring` is in `Cargo.lock` today. If `aws-lc-rs` ever appears and the probe panics with "no process-level CryptoProvider", report it. The fix belongs in the app.
