# 0043 · Proxy per cluster

[Back to index](README.md) · Step 4 · Modules: `crates/cluster/src/connection.rs`, `kubeconfig.rs` (`ConnectionInfo`), root `Cargo.toml`; app `cluster_registry.rs`, `cluster_form.rs`, `clusters_page.rs`, `cluster_session.rs`, `cluster_health.rs`. Wireframe: W2 Connection · `Proxy  None ▾`, note 5 ("k8sBoard does not store tokens").

**No new Kubernetes call.** The same requests travel through a proxy. This step changes `crates/cluster` (transport only, not the 0030 write path) and turns on two `kube` features.

## Today

`ClusterConnection::open` keeps a kubeconfig `proxy-url` and drops `HTTPS_PROXY` (kube never reads `NO_PROXY`). But `kube` is built without `http-proxy`/`socks5`, so a kubeconfig with `proxy-url` fails with "kubeconfig proxy-url is set but this build has no proxy support".

## Build

- Root `Cargo.toml` `kube` features + `"http-proxy"`, `"socks5"` (both only enable `hyper-util/client-proxy`; its `base64`, `ipnet`, `percent-encoding` are already in `Cargo.lock`). AC: no new `[[package]]`.
- `crates/cluster/Cargo.toml`: `http = "1"` becomes a normal dependency (already locked; today optional for `test-support`).
- Effect: a kubeconfig `proxy-url` with `http://`, `https://`, or `socks5://` starts working (the kubeconfig is the user's file; Settings itself offers `http://` and `socks5://` only, below). `redact_config_error` keeps hiding the URL; its text becomes `the proxy URL uses a scheme k8sBoard cannot use`.

## Cluster crate API

```rust
/// How the client reaches the API server. Holds no credential: a `ProxyUrl` has no userinfo,
/// and `Kubeconfig` leaves the kubeconfig's own proxy-url inside the document.
#[derive(Clone, PartialEq, Eq)]          // Debug manual: variant + scheme://host:port
pub enum ProxyChoice { Kubeconfig, Direct, Url(ProxyUrl) }
#[derive(Clone, PartialEq, Eq)]          // Debug manual: display()
pub struct ProxyUrl(http::Uri);          // built only by `parse`: valid by construction
impl ProxyUrl {
    pub fn parse(text: &str) -> Result<Self, ProxyUrlError>;
    pub fn display(&self) -> String;     // scheme://host[:port], scheme lowercase
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProxyUrlError { Scheme, Credentials, Host, Port, Path, TooLong, Malformed }
pub enum ClusterError { /* … */ InvalidProxy { context: String, #[source] source: ProxyUrlError } }
// Display: "context '{context}': the proxy URL in Settings is not valid"
impl ClusterConnection {
    pub async fn open(kubeconfig: &Kubeconfig, context: &str, proxy: &ProxyChoice) -> Result<Self, ClusterError>;
}
pub struct ConnectionInfo { /* server, auth */ pub proxy: Option<String> } // kubeconfig proxy-url, scheme://host[:port]
```

`open` after `from_custom_kubeconfig`:

| `proxy` | `config.proxy_url` |
|---|---|
| `Kubeconfig` | today's rule: the kubeconfig `proxy-url` (an `http(s)://user:pass@` userinfo there becomes Basic `Proxy-Authorization`; kube sends **no** credentials for `socks5://user:pass@`), else none; `HTTPS_PROXY` dropped |
| `Direct` | `None`, even when the kubeconfig sets one |
| `Url(url)` | `Some(url.0.clone())` |

- `Direct` cannot rescue a broken environment: `from_custom_kubeconfig` parses the kubeconfig `proxy-url` and `HTTPS_PROXY` before `open` can override them, so a malformed value in either still fails `open` with `InvalidConfig` (redacted).
- Trace `proxy_host` only (as today). The API server TLS runs end to end inside the tunnel; its certificate checks are unchanged.

## Validation (`ProxyUrl::parse`, hand-written, no regex)

| Check (in order) | `ProxyUrlError` | Form message |
|---|---|---|
| trimmed, ≤ 2,048 chars, no whitespace or control char | `TooLong` / `Malformed` | `Enter a URL such as http://proxy.example:3128.` |
| starts with `http://` or `socks5://`, case-insensitive; the stored `Uri` gets the **lowercase** scheme (kube matches `"socks5"` case-sensitively) | `Scheme` | `Use http:// or socks5://.` |
| authority (up to `/`, `?`, `#`) has no `@` | `Credentials` | `Leave out the user name and password: k8sBoard does not store proxy credentials. Put them in the kubeconfig proxy-url instead.` |
| host present (IPv6 in brackets allowed) | `Host` | `Add the proxy host.` |
| port absent or 1–65535 | `Port` | `Use a port from 1 to 65535.` |
| nothing after the authority but an optional `/` | `Path` | `Remove the path; a proxy URL is scheme://host:port.` |
| `http::Uri` parses | `Malformed` | as the first row |

**Why no `https://` proxy in Settings.** kube builds the TLS to an `https://` proxy from the cluster's rustls config: only the kubeconfig CA is trusted, the cluster's `tls-server-name` is pinned, the cluster's client certificate is offered to the proxy, and `insecure-skip-tls-verify` also disables the proxy's verification. A typical corporate proxy fails that, and offering the client certificate to a third party is wrong. Settings accept `http://` and `socks5://`; the tunnelled API-server TLS stays end to end either way.

## App side

```rust
// cluster_registry.rs
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")]
pub(crate) enum ClusterProxy { Direct, Url(String) }  // JSON: "direct" | {"url": "http://proxy:3128"}
impl fmt::Debug for ClusterProxy { … }                 // Direct · Url(scheme://host:port) · Url(<invalid>)
ClusterEntry.proxy: Option<ClusterProxy>;             // None = from kubeconfig
ClusterProfile.proxy: Result<ProxyChoice, ProxyUrlError>; // parsed in `profile()`; Url(t) → ProxyUrl::parse(t)
/// The one way the app opens a client: an unparsable stored URL fails closed.
pub(crate) async fn open_cluster(kubeconfig: &Kubeconfig, context: &str,
    proxy: &Result<ProxyChoice, ProxyUrlError>) -> Result<ClusterConnection, ClusterError>;
// Err(e) → ClusterError::InvalidProxy { context, source: e }; never Direct
```

- The 3 callers use `open_cluster` with `profile.proxy` read on the main thread when the work starts: `connect_cluster` (session start and switch), `ProbeTarget.proxy` (`cluster_health.rs`), `test_connection(.., proxy, timeout)`. Exec, logs, port forwards, and watches reuse the session client, so they follow it.
- A change applies to the next connection; a running session keeps its client (like 0025 decision 24).

## Form (Connection section, after Authentication)

| Control | Behaviour |
|---|---|
| Dropdown | shows the **stored** choice: `From kubeconfig ({info.proxy or "none"})` (default) · `None (direct)` · `Custom URL`. Picking From kubeconfig or None stores at once |
| Input (when Custom is picked or stored) | prefilled with `ProxyUrl::display()` of the stored URL, or empty when the stored text does not parse (the raw text is never shown); placeholder `http://proxy.example:3128`. **Commits on Enter or blur**, never per keystroke (unlike 0025 decision 7): valid → stores `Some(Url(display()))`; invalid → the message under the field, nothing stored |
| Inline status | `Not applied` (muted) next to the dropdown while Custom is picked but no valid URL is committed; the stored choice still applies |
| Hint | `Applies the next time k8sBoard connects. HTTPS_PROXY and NO_PROXY are not read, but exec credential plugins (aws, gcloud, …) inherit them from the environment.` |

Test connection uses the stored choice, so it checks the proxy right after a commit.

## Proxy authentication

- Never in `settings.json` (validation refuses userinfo, AC 12).
- An HTTP proxy that needs a password: put it in the kubeconfig `proxy-url` (`http://user:pass@host:3128`); kube sends it as Basic `Proxy-Authorization`. The credential stays in the user's own file; the form shows host and port only. SOCKS5 credentials are not supported by kube.
- OS keychain: the project has no keychain code; adding one is a new dependency plus a C1 review → **deferred** (open item 1).

## Known ceilings

- One proxy per cluster; no global proxy and no `NO_PROXY` list (one cluster at a time, 0046).
- Exec plugins reach their own endpoints (STS, OAuth) through whatever their environment says.
