# 0043 · Proxy per cluster

[Back to index](README.md) · Step 4 · Modules: `crates/cluster/src/connection.rs`, `kubeconfig.rs` (`ConnectionInfo`), root `Cargo.toml`; app `cluster_registry.rs`, `cluster_form.rs`, `clusters_page.rs`, `cluster_session.rs`, `cluster_health.rs`. Wireframe: W2 Connection · `Proxy  None ▾`, note 5 ("k8sBoard does not store tokens").

**No new Kubernetes call.** The same requests travel through a proxy. This step changes `crates/cluster` (transport only, not the 0030 write path) and turns on two `kube` features.

## Today

`ClusterConnection::open` keeps a kubeconfig `proxy-url` and drops `HTTPS_PROXY` (kube never reads `NO_PROXY`). But `kube` is built without `http-proxy`/`socks5`, so a kubeconfig with `proxy-url` fails with "kubeconfig proxy-url is set but this build has no proxy support".

## Build

- Root `Cargo.toml` `kube` features + `"http-proxy"`, `"socks5"` (both only enable `hyper-util/client-proxy`; its `base64`, `ipnet`, `percent-encoding` are already in `Cargo.lock`). AC: no new `[[package]]` in `Cargo.lock`.
- `crates/cluster/Cargo.toml`: `http = "1"` becomes a normal dependency (already locked; today optional for `test-support`).
- Effect: kubeconfig `proxy-url` with `http://`, `https://`, or `socks5://` starts working. `redact_config_error` keeps hiding the URL; its text becomes `the proxy URL uses a scheme k8sBoard cannot use`.

## Cluster crate API

```rust
/// How the client reaches the API server. Holds no credential: a Settings URL has no userinfo,
/// and `Kubeconfig` leaves the kubeconfig's own proxy-url inside the document.
#[derive(Clone, PartialEq, Eq)]          // Debug manual: variant + host only
pub enum ProxyChoice { Kubeconfig, Direct, Url(String) }
pub struct ProxyUrl(http::Uri);          // built only by `parse`
impl ProxyUrl {
    pub fn parse(text: &str) -> Result<Self, ProxyUrlError>;
    pub fn display(&self) -> String;     // scheme://host[:port]
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProxyUrlError { Scheme, Credentials, Host, Port, Path, TooLong, Malformed }
impl ClusterConnection {
    pub async fn open(kubeconfig: &Kubeconfig, context: &str, proxy: &ProxyChoice) -> Result<Self, ClusterError>;
}
pub struct ConnectionInfo { /* server, auth */ pub proxy: Option<String> } // kubeconfig proxy-url, scheme://host[:port]
```

`open` after `from_custom_kubeconfig`:

| `proxy` | `config.proxy_url` |
|---|---|
| `Kubeconfig` | today's rule: the kubeconfig `proxy-url` (with any userinfo it holds, used by kube for Basic auth), else none; `HTTPS_PROXY` dropped |
| `Direct` | `None`, even when the kubeconfig sets one |
| `Url(text)` | `ProxyUrl::parse(text)?` → `Some(uri)`; a parse error → `ClusterError::InvalidConfig { context, source: ProxyUrlError }` (a hand-edited bad value fails loudly, never falls back to direct) |

Trace `proxy_host` only (as today). The API server TLS still runs end to end inside the tunnel; certificate checks are unchanged.

## Validation (`ProxyUrl::parse`, hand-written, no regex)

| Check (in order) | `ProxyUrlError` | Form message |
|---|---|---|
| trimmed, ≤ 2,048 chars, no whitespace or control char | `TooLong` / `Malformed` | `Enter a URL such as http://proxy.example:3128.` |
| starts with `http://`, `https://`, or `socks5://` (case-insensitive) | `Scheme` | `Use http://, https://, or socks5://.` |
| authority (up to `/`, `?`, `#`) has no `@` | `Credentials` | `Leave out the user name and password: k8sBoard does not store proxy credentials. Put them in the kubeconfig proxy-url instead.` |
| host present (IPv6 in brackets allowed) | `Host` | `Add the proxy host.` |
| port absent or 1–65535 | `Port` | `Use a port from 1 to 65535.` |
| nothing after the authority but an optional `/` | `Path` | `Remove the path; a proxy URL is scheme://host:port.` |
| `http::Uri` parses | `Malformed` | as the first row |

## App side

```rust
// cluster_registry.rs
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")]
pub(crate) enum ClusterProxy { Direct, Url(String) }  // JSON: "direct" | {"url": "http://proxy:3128"}
ClusterEntry.proxy: Option<ClusterProxy>;             // None = from kubeconfig
ClusterProfile.proxy: ProxyChoice;                    // None → Kubeconfig, Direct → Direct, Url(t) → Url(t)
```

- Callers pass `&profile.proxy`, read on the main thread when the work starts: `connect_cluster(.., proxy)` (session start and switch), `ProbeTarget.proxy` (switcher health probe, `cluster_health.rs`), `test_connection(.., proxy, timeout)`. Exec, logs, port forwards, and watches reuse the session client, so they follow it.
- A change applies to the next connection; a running session keeps its client (like 0025 decision 24).

## Form (Connection section, after Authentication)

| Control | Behaviour |
|---|---|
| Dropdown | `From kubeconfig ({info.proxy or "none"})` (default) · `None (direct)` · `Custom URL` |
| Input (Custom only) | placeholder `http://proxy.example:3128`; each valid edit stores `Some(Url(text))`; an invalid one shows the message and keeps the last stored value (0025 decision 7); empty keeps the previous value |
| Hint | `Applies the next time k8sBoard connects. HTTPS_PROXY and NO_PROXY are not read.` |

Test connection uses the stored choice, so it checks the proxy right after an edit.

## Proxy authentication

- Never in `settings.json` (validation refuses userinfo, AC).
- A proxy that needs a password: put it in the kubeconfig `proxy-url` (`http://user:pass@host:3128`), which kube already turns into a Basic `Proxy-Authorization` header. The credential stays in the user's own file; the form shows host and port only.
- OS keychain: the project has no keychain code and adding one is a new dependency plus a C1 review → **deferred** (open item 2).

## Known ceilings

- An `https://` proxy is verified with the cluster's TLS settings (kube builds the proxy TLS from the same config): a proxy with a private CA needs that CA in the kubeconfig `certificate-authority-data`, or an `http://` proxy.
- One proxy per cluster; no global proxy and no `NO_PROXY` list (one cluster at a time, 0046).
