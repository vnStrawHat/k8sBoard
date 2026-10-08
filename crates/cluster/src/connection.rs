use std::fmt;
use std::future::Future;
use std::time::Duration;

use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::serde::de::DeserializeOwned;
use kube::Api;
use kube::api::{ApiResource, DynamicObject, ListParams};
use kube::config::KubeConfigOptions;
use tokio::time::error::Elapsed;

use crate::kubeconfig::{Kubeconfig, KubeconfigError};
use crate::namespace::NamespaceScope;
use crate::object_write::WritePolicy;
use crate::proxy::{ProxyChoice, ProxyUrlError};
use crate::traffic::TrafficCounter;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const LIST_PAGE_SIZE: u32 = 500;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A connection to one cluster context. Clones share the HTTP client. Reads are open; writes go
/// only through `write` in `object_write.rs`.
// Debug is manual: it prints the context only.
#[derive(Clone)]
pub struct ClusterConnection {
    client: kube::Client,
    context: String,
    default_namespace: String,
    write_policy: WritePolicy,
    traffic: TrafficCounter,
}

/// Server version as reported by `GET /version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerVersion {
    /// For example `v1.29.5`.
    pub git_version: String,
    /// For example `linux/amd64`.
    pub platform: String,
}

/// Each `Display` describes its own layer only; callers walk `source()` for the rest.
#[derive(Debug, thiserror::Error)]
pub enum ClusterError {
    #[error(transparent)]
    Kubeconfig(#[from] KubeconfigError),
    #[error("cannot build a client for context '{context}'")]
    InvalidConfig {
        context: String,
        #[source]
        source: BoxError,
    },
    /// The proxy saved in Settings does not parse. The client is not built, and the connection is
    /// never made direct instead: traffic must not bypass a proxy the user asked for.
    #[error("context '{context}': the proxy URL in Settings is not valid")]
    InvalidProxy {
        context: String,
        #[source]
        source: ProxyUrlError,
    },
    #[error("cannot reach the API server of context '{context}' while {action}")]
    Unreachable {
        context: String,
        action: &'static str,
        #[source]
        source: BoxError,
    },
    #[error("the API server of context '{context}' did not answer in time while {action}")]
    TimedOut {
        context: String,
        action: &'static str,
    },
    // The detail is dropped on purpose: exec plugin output can hold secrets.
    #[error("credentials for context '{context}' are unavailable while {action}")]
    CredentialsUnavailable {
        context: String,
        action: &'static str,
    },
    #[error(
        "the API server rejected the credentials of context '{context}' while {action}: {message}"
    )]
    Unauthorized {
        context: String,
        action: &'static str,
        message: String,
    },
    #[error("context '{context}' is not allowed to perform {action}: {message}")]
    Forbidden {
        context: String,
        action: &'static str,
        message: String,
    },
    #[error("the API server of context '{context}' failed {action} with HTTP {code}: {message}")]
    Api {
        context: String,
        action: &'static str,
        code: u16,
        message: String,
    },
    #[error("unexpected response from the API server of context '{context}' while {action}")]
    UnexpectedResponse {
        context: String,
        action: &'static str,
        #[source]
        source: BoxError,
    },
    /// One namespace of a merged multi-namespace watch failed; the others may be fine.
    #[error("namespace '{namespace}'")]
    Namespace {
        namespace: String,
        #[source]
        source: Box<ClusterError>,
    },
    /// A failure kept as the text it was first announced with, so a merged watch can repeat
    /// it (`ClusterError` is not `Clone`). Holds no more than the original `Display`.
    #[error("{message}")]
    Rendered { message: String },
}

/// An API handle with the namespace it is scoped to; `None` is every namespace.
pub(crate) type ScopedApi<K> = (Option<String>, Api<K>);

/// One page of a paged list. `None` or an empty token marks the last page.
struct Page<K> {
    items: Vec<K>,
    continue_token: Option<String>,
}

impl ClusterConnection {
    /// Builds a client for `context` that reaches the server as `proxy` says. Does no network I/O,
    /// but spawns kube's client worker, so it must be polled on the application's tokio runtime.
    pub async fn open(
        kubeconfig: &Kubeconfig,
        context: &str,
        proxy: &ProxyChoice,
    ) -> Result<Self, ClusterError> {
        let summary = kubeconfig.resolve_context(Some(context))?;
        let name = summary.name.clone();
        let has_proxy_url = kubeconfig.has_proxy_url(&summary.cluster);
        let options = KubeConfigOptions {
            context: Some(name.clone()),
            cluster: None,
            user: None,
        };
        let mut config =
            kube::Config::from_custom_kubeconfig(kubeconfig.document().clone(), &options)
                .await
                .map_err(|error| invalid_config(&name, error.into()))?;
        config.connect_timeout = Some(CONNECT_TIMEOUT);
        // kube falls back to HTTPS_PROXY/https_proxy and never reads NO_PROXY, so honouring the
        // variable could send a local cluster through a proxy. Only a proxy-url set in the
        // kubeconfig, or the one picked in Settings, is used.
        if matches!(proxy, ProxyChoice::Kubeconfig)
            && !has_proxy_url
            && let Some(ambient) = &config.proxy_url
        {
            // Host only: the URL may carry userinfo.
            tracing::info!(
                context = %name,
                proxy_host = ambient.host().unwrap_or("unknown"),
                "ignoring the proxy from the environment"
            );
        }
        config.proxy_url = proxy.config_uri(has_proxy_url, config.proxy_url.take());
        if let Some(url) = &config.proxy_url {
            // Host only, as above.
            tracing::debug!(context = %name, proxy_host = url.host().unwrap_or("unknown"), "connecting through a proxy");
        }
        // kube's default retry layer retries HTTP 429 for ~30 s, which hides the budget refusal of
        // an eviction (a 429 `TooManyRequests`) behind a timeout. The app reconnects by itself.
        config.default_retry = false;
        // No read timeout: watches (0002) need long reads.
        let default_namespace = config.default_namespace.clone();
        let traffic = TrafficCounter::default();
        let builder = kube::client::ClientBuilder::try_from(config)
            .map_err(|error| invalid_config(&name, error))?;
        let client = traffic.client(builder);
        let write_policy = WritePolicy::of_build(&name);
        Ok(Self {
            client,
            context: name,
            default_namespace,
            write_policy,
            traffic,
        })
    }

    /// A connection over a fake transport, so tests never reach a cluster.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_client(client: kube::Client, context: &str, write_policy: WritePolicy) -> Self {
        Self {
            client,
            context: context.to_owned(),
            default_namespace: "default".to_owned(),
            write_policy,
            traffic: TrafficCounter::default(),
        }
    }

    /// The bytes this connection has sent and received so far; clones of the counter share the totals.
    pub fn traffic(&self) -> TrafficCounter {
        self.traffic.clone()
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    /// The context namespace, or `default`.
    pub fn default_namespace(&self) -> &str {
        &self.default_namespace
    }

    /// `GET /version`. Also serves as the "test connection" call.
    pub async fn server_version(&self) -> Result<ServerVersion, ClusterError> {
        let action = "reading the server version";
        let info = self.run(action, self.client.apiserver_version()).await?;
        Ok(ServerVersion {
            git_version: info.git_version,
            platform: info.platform,
        })
    }

    pub(crate) fn write_policy(&self) -> WritePolicy {
        self.write_policy
    }

    pub(crate) fn client(&self) -> &kube::Client {
        &self.client
    }

    /// API handles for a namespaced kind: one unnamed handle for every namespace, or one
    /// named handle per picked namespace, in scope order.
    pub(crate) fn scoped_apis<K>(&self, scope: &NamespaceScope) -> Vec<ScopedApi<K>>
    where
        K: kube::Resource<Scope = NamespaceResourceScope>,
        K::DynamicType: Default,
    {
        let client = self.client();
        if matches!(scope, NamespaceScope::All) {
            return vec![(None, Api::all(client.clone()))];
        }
        scope
            .namespaces()
            .iter()
            .map(|namespace| {
                let api = Api::namespaced(client.clone(), namespace);
                (Some(namespace.clone()), api)
            })
            .collect()
    }

    /// Like `scoped_apis`, for a kind known only by its `ApiResource`.
    pub(crate) fn scoped_dynamic_apis(
        &self,
        scope: &NamespaceScope,
        resource: &ApiResource,
    ) -> Vec<ScopedApi<DynamicObject>> {
        let client = self.client();
        if matches!(scope, NamespaceScope::All) {
            return vec![(None, Api::all_with(client.clone(), resource))];
        }
        scope
            .namespaces()
            .iter()
            .map(|namespace| {
                let api = Api::namespaced_with(client.clone(), namespace, resource);
                (Some(namespace.clone()), api)
            })
            .collect()
    }

    /// Runs one request under `REQUEST_TIMEOUT`; errors go through `classify_error`.
    pub(crate) async fn run<T>(
        &self,
        action: &'static str,
        request: impl Future<Output = Result<T, kube::Error>>,
    ) -> Result<T, ClusterError> {
        tracing::debug!(context = %self.context, action, "sending request");
        match tokio::time::timeout(REQUEST_TIMEOUT, request).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(classify_error(&self.context, action, error)),
            Err(_elapsed) => Err(ClusterError::TimedOut {
                context: self.context.clone(),
                action,
            }),
        }
    }

    /// Lists every object, following continue tokens.
    pub(crate) async fn list_all<K>(
        &self,
        api: Api<K>,
        action: &'static str,
    ) -> Result<Vec<K>, ClusterError>
    where
        K: Clone + DeserializeOwned + fmt::Debug,
    {
        self.list_all_where(api, action, None).await
    }

    /// Like `list_all`, keeping only the objects the `field_selector` matches (server side).
    pub(crate) async fn list_all_where<K>(
        &self,
        api: Api<K>,
        action: &'static str,
        field_selector: Option<&str>,
    ) -> Result<Vec<K>, ClusterError>
    where
        K: Clone + DeserializeOwned + fmt::Debug,
    {
        let api = &api;
        collect_pages(|params| async move {
            let params = match field_selector {
                Some(selector) => params.fields(selector),
                None => params,
            };
            let list = self.run(action, api.list(&params)).await?;
            Ok(Page {
                items: list.items,
                continue_token: list.metadata.continue_,
            })
        })
        .await
    }
}

/// Runs one request under `REQUEST_TIMEOUT`, keeping the timeout apart from the `kube::Error`
/// because `ClusterConnection::run` classifies errors before the `Status` or the upgrade refusal
/// can be read. Shared by `object_write.rs`, `pod_shell.rs`, and `port_forward.rs`.
pub(crate) async fn run_raw<T>(
    request: impl Future<Output = Result<T, kube::Error>>,
) -> Result<Result<T, kube::Error>, Elapsed> {
    tokio::time::timeout(REQUEST_TIMEOUT, request).await
}

impl fmt::Debug for ClusterConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClusterConnection")
            .field("context", &self.context)
            .finish()
    }
}

fn invalid_config(context: &str, error: kube::Error) -> ClusterError {
    ClusterError::InvalidConfig {
        context: context.to_owned(),
        source: redact_config_error(error),
    }
}

/// Replaces errors whose text can embed secrets with fixed messages and drops the
/// original: a proxy URL may carry `user:pass@` userinfo, and auth errors may carry
/// exec plugin output.
fn redact_config_error(error: kube::Error) -> BoxError {
    match error {
        kube::Error::ProxyProtocolDisabled { .. }
        | kube::Error::ProxyProtocolUnsupported { .. } => {
            "the proxy URL uses a scheme k8sBoard cannot use".into()
        }
        kube::Error::Auth(_) => "credentials could not be prepared".into(),
        other => Box::new(other),
    }
}

/// Collects every page. A 410 on a page after the first means the continue token
/// expired: the collected items are dropped and the listing restarts once, like
/// client-go's pager. Any other error, and a second 410, is returned as-is.
async fn collect_pages<K, F, Fut>(mut fetch_page: F) -> Result<Vec<K>, ClusterError>
where
    F: FnMut(ListParams) -> Fut,
    Fut: Future<Output = Result<Page<K>, ClusterError>>,
{
    let mut has_restarted = false;
    'restart: loop {
        let mut items = Vec::new();
        let mut continue_token: Option<String> = None;
        loop {
            let mut params = ListParams::default().limit(LIST_PAGE_SIZE);
            let is_first_page = continue_token.is_none();
            if let Some(token) = &continue_token {
                params = params.continue_token(token);
            }
            match fetch_page(params).await {
                Ok(page) => {
                    items.extend(page.items);
                    match page.continue_token.filter(|token| !token.is_empty()) {
                        Some(token) => continue_token = Some(token),
                        None => return Ok(items),
                    }
                }
                Err(ClusterError::Api { code: 410, .. }) if !is_first_page && !has_restarted => {
                    has_restarted = true;
                    continue 'restart;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

pub(crate) fn classify_error(
    context: &str,
    action: &'static str,
    error: kube::Error,
) -> ClusterError {
    let context = context.to_owned();
    match error {
        kube::Error::Api(status) => match status.code {
            401 => ClusterError::Unauthorized {
                context,
                action,
                message: status.message,
            },
            403 => ClusterError::Forbidden {
                context,
                action,
                message: status.message,
            },
            code => ClusterError::Api {
                context,
                action,
                code,
                message: status.message,
            },
        },
        // Nothing is logged or kept: exec plugin output can hold secrets.
        kube::Error::Auth(_) => ClusterError::CredentialsUnavailable { context, action },
        // Serde messages can quote fragments of the response body.
        kube::Error::SerdeError(_) => ClusterError::UnexpectedResponse {
            context,
            action,
            source: "response body could not be decoded".into(),
        },
        kube::Error::HyperError(_)
        | kube::Error::Service(_)
        | kube::Error::RustlsTls(_)
        | kube::Error::HttpError(_)
        | kube::Error::ProxyProtocolUnsupported { .. }
        | kube::Error::ProxyProtocolDisabled { .. }
        | kube::Error::TlsRequired => ClusterError::Unreachable {
            context,
            action,
            source: Box::new(error),
        },
        _ => ClusterError::UnexpectedResponse {
            context,
            action,
            source: Box::new(error),
        },
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod connection_tests;
