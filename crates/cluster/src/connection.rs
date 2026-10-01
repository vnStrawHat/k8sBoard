use std::fmt;
use std::future::Future;
use std::time::Duration;

use k8s_openapi::serde::de::DeserializeOwned;
use kube::Api;
use kube::api::ListParams;
use kube::config::KubeConfigOptions;

use crate::kubeconfig::{Kubeconfig, KubeconfigError};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_PAGE_SIZE: u32 = 500;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A read-only connection to one cluster context. Clones share the HTTP client.
// Debug is manual: it prints the context only.
#[derive(Clone)]
pub struct ClusterConnection {
    client: kube::Client,
    context: String,
    default_namespace: String,
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
}

/// One page of a paged list. `None` or an empty token marks the last page.
struct Page<K> {
    items: Vec<K>,
    continue_token: Option<String>,
}

impl ClusterConnection {
    /// Builds a client for `context`. Does no network I/O, but spawns kube's client
    /// worker, so it must be polled on the application's tokio runtime.
    pub async fn open(kubeconfig: &Kubeconfig, context: &str) -> Result<Self, ClusterError> {
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
        // kube falls back to HTTPS_PROXY/https_proxy, never reads NO_PROXY, and this build has
        // no proxy support, so an ambient proxy would make every open() fail with
        // ProxyProtocolDisabled. Only a proxy-url set in the kubeconfig is kept.
        if !has_proxy_url && let Some(proxy) = &config.proxy_url {
            // Host only: the URL may carry userinfo.
            tracing::info!(
                context = %name,
                proxy_host = proxy.host().unwrap_or("unknown"),
                "ignoring the proxy from the environment"
            );
        }
        config.proxy_url = proxy_for(has_proxy_url, config.proxy_url.take());
        // No read timeout: watches (0002) need long reads.
        let default_namespace = config.default_namespace.clone();
        let client =
            kube::Client::try_from(config).map_err(|error| invalid_config(&name, error))?;
        Ok(Self {
            client,
            context: name,
            default_namespace,
        })
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

    pub(crate) fn client(&self) -> &kube::Client {
        &self.client
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
        let api = &api;
        collect_pages(|params| async move {
            let list = self.run(action, api.list(&params)).await?;
            Ok(Page {
                items: list.items,
                continue_token: list.metadata.continue_,
            })
        })
        .await
    }
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

/// The proxy to use: the loaded one only when the kubeconfig itself set a `proxy-url`.
fn proxy_for<T>(has_proxy_url: bool, loaded: Option<T>) -> Option<T> {
    if has_proxy_url { loaded } else { None }
}

/// Replaces errors whose text can embed secrets with fixed messages and drops the
/// original: a proxy URL may carry `user:pass@` userinfo, and auth errors may carry
/// exec plugin output.
fn redact_config_error(error: kube::Error) -> BoxError {
    match error {
        kube::Error::ProxyProtocolDisabled { .. }
        | kube::Error::ProxyProtocolUnsupported { .. } => {
            "kubeconfig proxy-url is set but this build has no proxy support".into()
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

fn classify_error(context: &str, action: &'static str, error: kube::Error) -> ClusterError {
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
