//! The Secrets that are built from fields (UX round 3, N11 and N16): a docker-registry pull secret,
//! a TLS pair, and the replacement of a TLS pair. Pure builders of a `WriteIntent`: nothing here
//! opens a dialog or sends a request, and no error text quotes what was typed.
//!
//! The text of a field can be a password or a private key, so `Debug` is never derived for a type
//! that holds one, and the JSON that carries them lives only as long as one call.

use cluster::{
    KeyChange, KeyCheck, NewValue, ObjectDraft, ObjectKind, TlsPairInfo, TlsPairIssue, ValuesBase,
    WriteOperation, WriteRequest, check_tls_pair, docker_config_json,
};
use gpui_kit::SharedString;
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use crate::app_shell::write_flow::WriteIntent;
use crate::certificate_expiry::{date_time_text, expiry_label};
use crate::node_edits::NodeScope;
use crate::object_create_view::create_intent;
use crate::resource_actions::{ResourceAction, action_risk};

/// What `docker login` writes for Docker Hub.
pub(crate) const DEFAULT_REGISTRY_SERVER: &str = "https://index.docker.io/v1/";
const REGISTRY_TYPE: &str = "kubernetes.io/dockerconfigjson";
const TLS_TYPE: &str = "kubernetes.io/tls";
const DOCKER_CONFIG_KEY: &str = ".dockerconfigjson";
const CERTIFICATE_KEY: &str = "tls.crt";
const PRIVATE_KEY_KEY: &str = "tls.key";
const KEY_DIFFERS: &str = "The key does not match the certificate";
const KEY_NOT_CHECKED: &str =
    "The key could not be compared with the certificate; the server accepts any pair";

/// The fields of a docker-registry pull secret, as typed.
pub(crate) struct RegistryFields<'a> {
    pub(crate) name: &'a str,
    pub(crate) namespace: &'a str,
    pub(crate) server: &'a str,
    pub(crate) username: &'a str,
    pub(crate) password: &'a str,
    pub(crate) email: &'a str,
}

/// The text of a new Secret: `stringData` is folded into `data` by the draft.
fn secret_text(
    name: &str,
    namespace: &str,
    secret_type: &str,
    string_data: Map<String, Value>,
) -> Zeroizing<String> {
    let secret = json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": name, "namespace": namespace},
        "type": secret_type,
        "stringData": string_data,
    });
    Zeroizing::new(secret.to_string())
}

/// The create of the Secret the text describes, as the write flow's intent.
fn create_secret_intent(
    scope: &NodeScope<'_>,
    text: &str,
    change_lines: Vec<SharedString>,
) -> Result<WriteIntent, SharedString> {
    let draft = ObjectDraft::new(ObjectKind::Secret, text)
        .map_err(|error| SharedString::from(error.to_string()))?;
    let request = WriteRequest::new(
        draft.target().clone(),
        WriteOperation::CreateObject(Box::new(draft)),
    )
    .ok_or_else(|| SharedString::from("This Secret cannot be created here"))?;
    let cluster_name = SharedString::from(scope.cluster_name.to_owned());
    let mut intent = create_intent(
        scope.cluster,
        &cluster_name,
        ObjectKind::Secret,
        request,
        Vec::new(),
    );
    intent.change_lines = change_lines;
    Ok(intent)
}

/// New docker-registry Secret: the four fields of `kubectl create secret docker-registry` become
/// `.dockerconfigjson`. `Err` is the line the dialog shows under the fields.
pub(crate) fn registry_intent(
    scope: &NodeScope<'_>,
    fields: &RegistryFields<'_>,
) -> Result<WriteIntent, SharedString> {
    let fields = RegistryFields {
        name: fields.name.trim(),
        namespace: fields.namespace.trim(),
        server: fields.server.trim(),
        username: fields.username.trim(),
        // A password is kept as typed.
        password: fields.password,
        email: fields.email.trim(),
    };
    for (value, problem) in [
        (fields.name, "Enter a name"),
        (fields.namespace, "Enter a namespace"),
        (fields.server, "Enter the registry server"),
        (fields.username, "Enter the user name"),
        (fields.password, "Enter the password"),
    ] {
        if value.is_empty() {
            return Err(problem.into());
        }
    }
    let config = docker_config_json(
        fields.server,
        fields.username,
        fields.password,
        fields.email,
    );
    let mut string_data = Map::new();
    string_data.insert(DOCKER_CONFIG_KEY.to_owned(), json!(config.as_str()));
    let text = secret_text(fields.name, fields.namespace, REGISTRY_TYPE, string_data);
    let lines = vec![
        format!("type: {REGISTRY_TYPE}").into(),
        format!("registry: {}, user {}", fields.server, fields.username).into(),
        format!("{DOCKER_CONFIG_KEY}: built from these fields").into(),
    ];
    create_secret_intent(scope, &text, lines)
}

/// What the TLS form shows under its two fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TlsReport {
    /// Neither field has text yet.
    Empty,
    /// Why the pair cannot be used.
    Problem(SharedString),
    Ready(TlsPairInfo),
}

pub(crate) fn tls_report(certificate: &str, key: &str) -> TlsReport {
    if certificate.trim().is_empty() && key.trim().is_empty() {
        return TlsReport::Empty;
    }
    match check_tls_pair(certificate.as_bytes(), key.as_bytes()) {
        Ok(info) => TlsReport::Ready(info),
        Err(TlsPairIssue::Certificate) => {
            TlsReport::Problem("The certificate has no CERTIFICATE block that parses".into())
        }
        Err(TlsPairIssue::NoKey) => TlsReport::Problem("The key has no PRIVATE KEY block".into()),
    }
}

/// The lines under the fields: Subject, Not after with the days left, and the key check.
pub(crate) fn tls_report_lines(info: &TlsPairInfo, now: jiff::Timestamp) -> Vec<String> {
    let key = match info.key {
        KeyCheck::Matches => "Key matches the certificate",
        KeyCheck::Differs => KEY_DIFFERS,
        KeyCheck::NotChecked => "Key not checked: it cannot be compared with the certificate",
    };
    vec![
        format!("Subject: {}", info.subject),
        format!(
            "Not after: {} ({})",
            date_time_text(info.not_after),
            expiry_label(info.not_after, now).text
        ),
        key.to_owned(),
    ]
}

/// The pair, once the form can send it: a report that is ready, with a key that does not differ.
fn usable_pair(certificate: &str, key: &str) -> Result<TlsPairInfo, SharedString> {
    match tls_report(certificate, key) {
        TlsReport::Empty => Err("Paste the certificate and the key".into()),
        TlsReport::Problem(text) => Err(text),
        TlsReport::Ready(info) if info.key == KeyCheck::Differs => Err(KEY_DIFFERS.into()),
        TlsReport::Ready(info) => Ok(info),
    }
}

fn pair_lines(info: &TlsPairInfo, old_not_after: Option<jiff::Timestamp>) -> Vec<SharedString> {
    let key = match info.key {
        KeyCheck::Matches => "set (matches the certificate)",
        _ => "set (not compared with the certificate)",
    };
    let not_after = match old_not_after {
        Some(old) => format!(
            "not after: {} → {}",
            date_time_text(old),
            date_time_text(info.not_after)
        ),
        None => format!("not after: {}", date_time_text(info.not_after)),
    };
    vec![
        format!("{CERTIFICATE_KEY}: {}", info.subject).into(),
        not_after.into(),
        format!("{PRIVATE_KEY_KEY}: {key}").into(),
    ]
}

/// New TLS Secret from a certificate and its key.
pub(crate) fn tls_create_intent(
    scope: &NodeScope<'_>,
    name: &str,
    namespace: &str,
    certificate: &str,
    key: &str,
) -> Result<WriteIntent, SharedString> {
    let (name, namespace) = (name.trim(), namespace.trim());
    if name.is_empty() {
        return Err("Enter a name".into());
    }
    if namespace.is_empty() {
        return Err("Enter a namespace".into());
    }
    let info = usable_pair(certificate, key)?;
    let mut string_data = Map::new();
    string_data.insert(CERTIFICATE_KEY.to_owned(), json!(certificate));
    string_data.insert(PRIVATE_KEY_KEY.to_owned(), json!(key));
    let text = secret_text(name, namespace, TLS_TYPE, string_data);
    let mut lines: Vec<SharedString> = vec![format!("type: {TLS_TYPE}").into()];
    lines.extend(pair_lines(&info, None));
    let mut intent = create_secret_intent(scope, &text, lines)?;
    if info.key != KeyCheck::Matches {
        intent.warnings.push(KEY_NOT_CHECKED.into());
    }
    Ok(intent)
}

/// Replace certificate of a TLS Secret: `tls.crt` and `tls.key` change together, guarded by the
/// `resourceVersion` of `base`. `old_not_after` is the expiry the Secret has now, when it is known.
pub(crate) fn tls_replace_intent(
    scope: &NodeScope<'_>,
    base: &ValuesBase,
    certificate: &str,
    key: &str,
    old_not_after: Option<jiff::Timestamp>,
) -> Result<WriteIntent, SharedString> {
    let info = usable_pair(certificate, key)?;
    let change = |name: &str, text: &str| {
        let value = NewValue::new(Zeroizing::new(text.to_owned()));
        if base.keys().iter().any(|existing| existing.name == name) {
            KeyChange::Set {
                key: name.to_owned(),
                value,
            }
        } else {
            KeyChange::Add {
                key: name.to_owned(),
                value,
            }
        }
    };
    let edit = base
        .edit(vec![
            change(CERTIFICATE_KEY, certificate),
            change(PRIVATE_KEY_KEY, key),
        ])
        .map_err(|error| SharedString::from(error.to_string()))?;
    let target = base.target().clone();
    let request = WriteRequest::new(
        target.clone(),
        WriteOperation::SetDataValues(Box::new(edit)),
    )
    .ok_or_else(|| SharedString::from("These values cannot be replaced here"))?;
    let change_lines = pair_lines(&info, old_not_after);
    let action = ResourceAction::ReplaceCertificate;
    let warnings = if info.key == KeyCheck::Matches {
        Vec::new()
    } else {
        vec![KEY_NOT_CHECKED.into()]
    };
    Ok(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!("Replace certificate of secret {}", target.name()).into(),
        button: "Replace certificate".into(),
        request,
        risk: action_risk(action),
        warnings,
        change_lines,
        audit_fields: Vec::new(),
    })
}

#[cfg(test)]
#[path = "secret_forms_tests.rs"]
mod secret_forms_tests;
