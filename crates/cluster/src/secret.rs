//! Secrets: summaries for the explorer and the one-shot read of a Secret's values.
//!
//! Values are the most sensitive data k8sBoard touches, so nothing in this module logs or
//! traces, summaries never hold a value, and plaintext lives only in `SecretValue`.

use std::collections::BTreeMap;

use futures::Stream;
use k8s_openapi::api::core::v1::Secret;
use kube::Resource;
use kube::api::GetParams;
use kube::core::Request;
use kube::runtime::watcher::{self, ListSemantic};
use serde::Deserialize;
use serde::de::IgnoredAny;
use zeroize::{Zeroize, Zeroizing};

use crate::certificate::{CertificateInfo, CertificateIssue, parse_certificates};
use crate::connection::{ClusterConnection, ClusterError};
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, selected_summary_watch};
use crate::workload::label_terms;

const READ_ACTION: &str = "reading secret values";
const TLS_TYPE: &str = "kubernetes.io/tls";
const TLS_CERTIFICATE_KEY: &str = "tls.crt";
const DOCKER_CONFIG_JSON_TYPE: &str = "kubernetes.io/dockerconfigjson";
const DOCKER_CONFIG_TYPE: &str = "kubernetes.io/dockercfg";
pub(crate) const SERVICE_ACCOUNT_TOKEN_TYPE: &str = "kubernetes.io/service-account-token";
/// The only annotation read: it holds an account name, never a value.
const SERVICE_ACCOUNT_NAME_ANNOTATION: &str = "kubernetes.io/service-account.name";
/// Pages cap the plaintext in flight at 50 Secrets per list response.
const WATCH_PAGE_SIZE: u32 = 50;

/// Names, sizes, type, and type details. No value is ever copied, so `Debug` is safe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// The `type` as written; `Opaque` when unset.
    pub secret_type: String,
    /// `data` keys, sorted by name.
    pub keys: Vec<SecretKey>,
    pub details: SecretDetails,
    pub is_immutable: bool,
    /// `ownerReferences` is not empty.
    pub is_owned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretKey {
    pub name: String,
    pub size_bytes: usize,
    /// The value is not UTF-8.
    pub is_binary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretDetails {
    None,
    /// `kubernetes.io/tls` with a parsed `tls.crt`; leaf first, never empty.
    Certificate {
        chain: Vec<CertificateInfo>,
    },
    /// `kubernetes.io/tls` without a usable certificate.
    NoCertificate(CertificateIssue),
    /// Registry hosts of a docker config secret, sorted. Credentials are never copied.
    Registries(Vec<String>),
    ServiceAccountToken {
        account: Option<String>,
    },
}

/// Plaintext of one key, wiped on drop. No `Debug`, `Display`, `Clone`, or `PartialEq`, so a
/// value cannot reach a log or a comparison by accident.
pub struct SecretValue {
    key: String,
    bytes: Zeroizing<Vec<u8>>,
}

impl SecretValue {
    pub fn new(key: String, bytes: Vec<u8>) -> Self {
        Self {
            key,
            bytes: Zeroizing::new(bytes),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn size_bytes(&self) -> usize {
        self.bytes.len()
    }

    /// `None` when the value is not UTF-8.
    pub fn as_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }
}

impl ClusterConnection {
    /// Watches secrets in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_secrets(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<SecretSummary>> + Send + 'static {
        selected_summary_watch(
            self,
            self.scoped_apis(&scope),
            secret_watch_config(),
            "watching secrets",
            secret_summary,
        )
    }

    /// Like `watch_secrets`, for `kubernetes.io/tls` secrets only.
    pub fn watch_tls_secrets(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<SecretSummary>> + Send + 'static {
        selected_summary_watch(
            self,
            self.scoped_apis(&scope),
            tls_secret_watch_config(),
            "watching TLS secrets",
            secret_summary,
        )
    }

    /// One GET of a Secret, decoded here and never by kube: kube-client logs the whole body
    /// when a decode fails. Returns every value sorted by key; the caller keeps what it needs.
    pub async fn secret_values(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Vec<SecretValue>, ClusterError> {
        let body = self.secret_text(namespace, name, READ_ACTION).await?;
        Ok(values_of(decode_secret(self.context(), &body)?))
    }

    /// The JSON body of one Secret GET, wiped on drop. Callers decode it themselves because
    /// kube-client logs the whole body when a decode fails; `action` names the caller's work
    /// in errors. The body holds every value of the Secret.
    // A read-only GET decoded in this crate (0016): the secret.rs row of the 0030 table.
    #[allow(clippy::disallowed_methods)]
    pub(crate) async fn secret_text(
        &self,
        namespace: &str,
        name: &str,
        action: &'static str,
    ) -> Result<Zeroizing<String>, ClusterError> {
        let context = self.context();
        let request = secret_collection(namespace)
            .get(name, &GetParams::default())
            .map_err(|_| unexpected(context, action, "the secret request could not be built"))?;
        let body = self
            .run(action, self.client().request_text(request))
            .await
            .map_err(|error| without_body_bytes(context, error))?;
        Ok(Zeroizing::new(body))
    }
}

/// Paged `MostRecent` lists: the watch-cache list ignores `limit` and would return every
/// Secret in scope in one body.
fn secret_watch_config() -> watcher::Config {
    watcher::Config::default()
        .list_semantic(ListSemantic::MostRecent)
        .page_size(WATCH_PAGE_SIZE)
}

fn tls_secret_watch_config() -> watcher::Config {
    secret_watch_config().fields(&format!("type={TLS_TYPE}"))
}

fn secret_collection(namespace: &str) -> Request {
    Request::new(Secret::url_path(&(), Some(namespace)))
}

/// Fixed text only: serde and UTF-8 errors can quote the body.
fn unexpected(context: &str, action: &'static str, source: &'static str) -> ClusterError {
    ClusterError::UnexpectedResponse {
        context: context.to_owned(),
        action,
        source: source.into(),
    }
}

/// An unexpected-response error from the request carries a kube error whose `Debug` can hold the
/// body (the UTF-8 error keeps its bytes), so it is replaced with fixed text. Other kinds of
/// error (403, 404, transport) hold no body and pass through for the caller to word.
fn without_body_bytes(context: &str, error: ClusterError) -> ClusterError {
    let ClusterError::UnexpectedResponse { action, source, .. } = &error else {
        return error;
    };
    let text = match source.downcast_ref::<kube::Error>() {
        Some(kube::Error::FromUtf8(_)) => "the secret response was not valid UTF-8",
        _ => "the secret response could not be read",
    };
    unexpected(context, action, text)
}

pub(crate) fn decode_secret(context: &str, body: &str) -> Result<Secret, ClusterError> {
    serde_json::from_str(body)
        .map_err(|_| unexpected(context, READ_ACTION, "the secret could not be decoded"))
}

/// Moves each value out of the decoded Secret (no copy) and wipes the annotations, which
/// can embed the data through `last-applied-configuration`.
fn values_of(mut secret: Secret) -> Vec<SecretValue> {
    for value in secret
        .metadata
        .annotations
        .iter_mut()
        .flat_map(|map| map.values_mut())
    {
        value.zeroize();
    }
    secret
        .data
        .take()
        .into_iter()
        .flatten()
        .map(|(key, value)| SecretValue::new(key, value.0))
        .collect()
}

pub(crate) fn secret_summary(secret: &Secret) -> SecretSummary {
    let secret_type = secret
        .type_
        .as_deref()
        .filter(|text| !text.is_empty())
        .unwrap_or("Opaque")
        .to_owned();
    // `data` is a BTreeMap, so the keys are already sorted.
    let keys = secret
        .data
        .iter()
        .flatten()
        .map(|(name, value)| SecretKey {
            name: name.clone(),
            size_bytes: value.0.len(),
            is_binary: std::str::from_utf8(&value.0).is_err(),
        })
        .collect();
    SecretSummary {
        namespace: secret.metadata.namespace.clone().unwrap_or_default(),
        name: secret.metadata.name.clone().unwrap_or_default(),
        created_at: secret
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&secret.metadata),
        details: secret_details(secret, &secret_type),
        secret_type,
        keys,
        is_immutable: secret.immutable == Some(true),
        is_owned: secret
            .metadata
            .owner_references
            .as_ref()
            .is_some_and(|owners| !owners.is_empty()),
    }
}

fn secret_details(secret: &Secret, secret_type: &str) -> SecretDetails {
    match secret_type {
        TLS_TYPE => tls_details(secret),
        DOCKER_CONFIG_JSON_TYPE => {
            SecretDetails::Registries(registry_hosts(secret, ".dockerconfigjson", json_hosts))
        }
        DOCKER_CONFIG_TYPE => {
            SecretDetails::Registries(registry_hosts(secret, ".dockercfg", legacy_hosts))
        }
        SERVICE_ACCOUNT_TOKEN_TYPE => SecretDetails::ServiceAccountToken {
            account: secret
                .metadata
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get(SERVICE_ACCOUNT_NAME_ANNOTATION))
                .filter(|account| !account.is_empty())
                .cloned(),
        },
        _ => SecretDetails::None,
    }
}

/// Reads `tls.crt` only; `tls.key` is never touched.
fn tls_details(secret: &Secret) -> SecretDetails {
    let certificate = secret
        .data
        .as_ref()
        .and_then(|data| data.get(TLS_CERTIFICATE_KEY))
        .filter(|value| !value.0.is_empty());
    let Some(certificate) = certificate else {
        return SecretDetails::NoCertificate(CertificateIssue::Missing);
    };
    match parse_certificates(&certificate.0) {
        Ok(chain) => SecretDetails::Certificate { chain },
        Err(issue) => SecretDetails::NoCertificate(issue),
    }
}

/// `{"auths": {host: credentials}}`. `IgnoredAny` drops every credential unread.
#[derive(Deserialize)]
struct DockerConfigJson {
    #[serde(default)]
    auths: BTreeMap<String, IgnoredAny>,
}

type Hosts = BTreeMap<String, IgnoredAny>;

fn json_hosts(bytes: &[u8]) -> Option<Hosts> {
    let config: DockerConfigJson = serde_json::from_slice(bytes).ok()?;
    Some(config.auths)
}

/// The legacy `.dockercfg` is `{host: credentials}` at the top level.
fn legacy_hosts(bytes: &[u8]) -> Option<Hosts> {
    serde_json::from_slice(bytes).ok()
}

/// Missing or unparsable config gives no hosts.
fn registry_hosts(secret: &Secret, key: &str, read: fn(&[u8]) -> Option<Hosts>) -> Vec<String> {
    secret
        .data
        .as_ref()
        .and_then(|data| data.get(key))
        .and_then(|value| read(&value.0))
        .map(|hosts| hosts.into_keys().collect())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "secret_tests.rs"]
mod secret_tests;
