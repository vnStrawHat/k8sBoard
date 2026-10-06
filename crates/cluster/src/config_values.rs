//! Editing the values of a ConfigMap or Secret (spec 0047): the checked base the editor opens, the
//! checked edit, and the merge-patch body.
//!
//! Secret values are write-only here: the base keeps key names and sizes and wipes the data before
//! it returns. Nothing in this module logs, and `NewValue` and `ValueKey` have no `Debug`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde_json::{Map, Value, json};
use zeroize::{Zeroize, Zeroizing};

use crate::connection::{ClusterConnection, ClusterError};
use crate::object_edit::HELM_RELEASE_TYPE;
use crate::object_yaml::{ObjectKind, ObjectRef};
use crate::secret::{SERVICE_ACCOUNT_TOKEN_TYPE, decode_secret, secret_summary};

const ACTION: &str = "reading the object before editing its values";
const DECODE_FAILURE: &str = "the object could not be decoded";
const MISSING_VERSION: &str = "the object has no resourceVersion";
/// A ConfigMap text longer than this is not put in an input (main-thread layout).
pub const MAX_INLINE_VALUE: usize = 128 * 1024;
/// One value, and the estimated whole object (the API server's limit decides in the end).
const MAX_VALUE_BYTES: usize = 1024 * 1024;
const MAX_OBJECT_BYTES: usize = 1024 * 1024;
const MAX_KEY_BYTES: usize = 253;
const OWNER_LABEL: &str = "owner";
const HELM_OWNER: &str = "helm";
const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
const HELM_MANAGER: &str = "Helm";

/// What every write dialog and check says about an object that Helm owns: the next `helm upgrade`
/// renders it again from the chart.
pub const HELM_MANAGED_WARNING: &str = "Managed by Helm: the next upgrade replaces this change";

/// Whether the labels (`key`, `value` pairs) say Helm manages the object.
pub fn is_helm_managed<'a>(labels: impl IntoIterator<Item = (&'a str, &'a str)>) -> bool {
    labels
        .into_iter()
        .any(|(key, value)| key == MANAGED_BY_LABEL && value == HELM_MANAGER)
}

/// `is_helm_managed` for the `key=value` label terms of a workload summary.
pub fn terms_are_helm_managed(terms: &[String]) -> bool {
    is_helm_managed(terms.iter().filter_map(|term| term.split_once('=')))
}

/// What the editor opens: key metadata, flags, and ConfigMap text. Built only by `values_base`.
// Debug is manual: kind, namespace, name, resourceVersion, key count.
pub struct ValuesBase {
    target: ObjectRef,
    resource_version: String,
    keys: Vec<ValueKey>,
    notes: BaseNotes,
}

/// One key of the object. No `Debug`: a ConfigMap text can be sensitive.
#[derive(Clone)]
pub struct ValueKey {
    pub name: String,
    pub field: DataField,
    pub size_bytes: usize,
    pub content: KeyContent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataField {
    Data,
    BinaryData,
}

impl DataField {
    fn as_str(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::BinaryData => "binaryData",
        }
    }
}

#[derive(Clone)]
pub enum KeyContent {
    /// ConfigMap text up to `MAX_INLINE_VALUE`.
    Text(String),
    /// Secret text: never loaded, never shown.
    Hidden,
    /// A non-UTF-8 Secret value, or a ConfigMap `binaryData` key.
    Binary,
    /// ConfigMap text over `MAX_INLINE_VALUE`.
    TooLarge,
}

/// Facts for the editor's warnings.
#[derive(Clone, Debug, Default)]
pub struct BaseNotes {
    pub is_helm_managed: bool,
    /// `Kind/name` of the first owner reference.
    pub owner: Option<String>,
}

impl ValuesBase {
    pub fn target(&self) -> &ObjectRef {
        &self.target
    }

    pub fn resource_version(&self) -> &str {
        &self.resource_version
    }

    /// Sorted by name.
    pub fn keys(&self) -> &[ValueKey] {
        &self.keys
    }

    pub fn notes(&self) -> &BaseNotes {
        &self.notes
    }

    pub fn is_secret(&self) -> bool {
        self.target.builtin_kind() == Some(ObjectKind::Secret)
    }

    /// Checks `changes` locally (AC 18) and pairs each with its field. The error names the first
    /// offending key.
    pub fn edit(&self, changes: Vec<KeyChange>) -> Result<ValuesEdit, ValuesEditError> {
        if changes.is_empty() {
            return Err(ValuesEditError::NoChange);
        }
        let mut seen = BTreeSet::new();
        let mut paired = Vec::with_capacity(changes.len());
        for change in changes {
            let key = change.key();
            if !is_valid_key_name(key) {
                return Err(ValuesEditError::InvalidKey(key.to_owned()));
            }
            if !seen.insert(key.to_owned()) {
                return Err(ValuesEditError::DuplicateKey(key.to_owned()));
            }
            let field = self.field_of(&change)?;
            if change
                .value()
                .is_some_and(|value| value.0.len() > MAX_VALUE_BYTES)
            {
                return Err(ValuesEditError::TooLarge(key.to_owned()));
            }
            paired.push(DataFieldChange { field, change });
        }
        paired.sort_by(|left, right| left.change.key().cmp(right.change.key()));
        if self.estimated_size(&paired) > MAX_OBJECT_BYTES {
            return Err(ValuesEditError::ObjectTooLarge);
        }
        Ok(ValuesEdit {
            target: self.target.clone(),
            resource_version: self.resource_version.clone(),
            changes: paired,
        })
    }

    /// The field a change goes to: an `Add` goes to `data`, the others keep the key's field.
    fn field_of(&self, change: &KeyChange) -> Result<DataField, ValuesEditError> {
        let key = change.key();
        let existing = self.keys.iter().find(|candidate| candidate.name == key);
        match (change, existing) {
            (KeyChange::Add { .. }, None) => Ok(DataField::Data),
            (KeyChange::Add { .. }, Some(_)) => Err(ValuesEditError::DuplicateKey(key.to_owned())),
            (KeyChange::Set { .. } | KeyChange::Remove { .. }, None) => {
                Err(ValuesEditError::UnknownKey(key.to_owned()))
            }
            (KeyChange::Set { .. }, Some(found)) => match found.content {
                KeyContent::Text(_) | KeyContent::Hidden => Ok(found.field),
                KeyContent::Binary | KeyContent::TooLarge => {
                    Err(ValuesEditError::BinaryValue(key.to_owned()))
                }
            },
            (KeyChange::Remove { .. }, Some(found)) => Ok(found.field),
        }
    }

    /// An early hint (decision 13): the keys that stay plus the new values, as stored (base64 for
    /// a Secret and for `binaryData`).
    fn estimated_size(&self, paired: &[DataFieldChange]) -> usize {
        let touched: BTreeSet<&str> = paired.iter().map(|paired| paired.change.key()).collect();
        let kept = self
            .keys
            .iter()
            .filter(|key| !touched.contains(key.name.as_str()))
            .map(|key| key.name.len() + self.stored_len(key.field, key.size_bytes));
        let added = paired.iter().filter_map(|paired| {
            let value = paired.change.value()?;
            Some(paired.change.key().len() + self.stored_len(paired.field, value.0.len()))
        });
        kept.chain(added).sum()
    }

    fn stored_len(&self, field: DataField, len: usize) -> usize {
        if self.is_secret() || field == DataField::BinaryData {
            len.div_ceil(3) * 4
        } else {
            len
        }
    }
}

impl fmt::Debug for ValuesBase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValuesBase")
            .field("kind", &format_args!("{}", self.target.kind_name()))
            .field("namespace", &self.target.namespace())
            .field("name", &self.target.name())
            .field("resource_version", &self.resource_version)
            .field("keys", &self.keys.len())
            .finish()
    }
}

/// Plaintext of one new value, wiped on drop. No `Debug`, `Display`, or `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct NewValue(Zeroizing<String>);

impl NewValue {
    pub fn new(text: Zeroizing<String>) -> Self {
        Self(text)
    }
}

/// One requested change. Manual `Debug`: the variant and the key.
#[derive(Clone, PartialEq, Eq)]
pub enum KeyChange {
    Add { key: String, value: NewValue },
    Set { key: String, value: NewValue },
    Remove { key: String },
}

impl KeyChange {
    pub fn key(&self) -> &str {
        match self {
            Self::Add { key, .. } | Self::Set { key, .. } | Self::Remove { key } => key,
        }
    }

    fn value(&self) -> Option<&NewValue> {
        match self {
            Self::Add { value, .. } | Self::Set { value, .. } => Some(value),
            Self::Remove { .. } => None,
        }
    }

    /// `added`, `value changed`, or `removed`: the marker of the confirm dialog and the audit.
    fn marker(&self) -> &'static str {
        match self {
            Self::Add { .. } => "added",
            Self::Set { .. } => "value changed",
            Self::Remove { .. } => "removed",
        }
    }
}

impl fmt::Debug for KeyChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            Self::Add { .. } => "Add",
            Self::Set { .. } => "Set",
            Self::Remove { .. } => "Remove",
        };
        formatter
            .debug_tuple(variant)
            .field(&format_args!("{}", self.key()))
            .finish()
    }
}

/// A change with the field it goes to. The derived `Debug` prints `field` and `KeyChange`'s
/// manual `Debug`, so it holds no value. (Named apart from `edit_preview::FieldChange`.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataFieldChange {
    pub field: DataField,
    pub change: KeyChange,
}

/// A checked edit, built only by `ValuesBase::edit`. Manual `Debug`: kind, name, resourceVersion,
/// change count.
#[derive(Clone, PartialEq, Eq)]
pub struct ValuesEdit {
    target: ObjectRef,
    resource_version: String,
    /// Sorted by key.
    changes: Vec<DataFieldChange>,
}

impl ValuesEdit {
    pub fn target(&self) -> &ObjectRef {
        &self.target
    }

    pub fn changes(&self) -> &[DataFieldChange] {
        &self.changes
    }

    /// The `changed_fields` paths: `data[KEY] added`, `data[KEY] value changed`, ... Never a value.
    pub(crate) fn change_paths(&self) -> impl Iterator<Item = String> + '_ {
        self.changes.iter().map(|paired| {
            format!(
                "{}[{}] {}",
                paired.field.as_str(),
                paired.change.key(),
                paired.change.marker()
            )
        })
    }

    fn is_secret(&self) -> bool {
        self.target.builtin_kind() == Some(ObjectKind::Secret)
    }
}

impl fmt::Debug for ValuesEdit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValuesEdit")
            .field("kind", &format_args!("{}", self.target.kind_name()))
            .field("name", &self.target.name())
            .field("resource_version", &self.resource_version)
            .field("changes", &self.changes.len())
            .finish()
    }
}

/// Why no editor opens. `Display` is fixed text.
#[derive(Debug, thiserror::Error)]
pub enum ValuesBaseError {
    #[error("Helm release records cannot be edited")]
    HelmRelease,
    #[error("service account tokens are managed by Kubernetes")]
    ServiceAccountToken,
    #[error("the object is immutable")]
    Immutable,
    #[error("only ConfigMap and Secret values can be edited")]
    NotEditable,
    #[error(transparent)]
    Cluster(#[from] ClusterError),
}

/// Why an edit is refused before any request. `Display` is fixed text plus at most a key name.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValuesEditError {
    #[error("there is no change to apply")]
    NoChange,
    #[error("'{0}' is not a valid key name")]
    InvalidKey(String),
    #[error("the key '{0}' is used twice")]
    DuplicateKey(String),
    #[error("the key '{0}' does not exist")]
    UnknownKey(String),
    #[error("the key '{0}' holds binary data and can only be removed")]
    BinaryValue(String),
    #[error("the value of '{0}' is larger than 1 MiB")]
    TooLarge(String),
    #[error("the object would be larger than 1 MiB")]
    ObjectTooLarge,
}

/// A key name of a ConfigMap or Secret: `[-._a-zA-Z0-9]+`, at most 253 bytes, not `.` or `..`.
pub fn is_valid_key_name(name: &str) -> bool {
    (1..=MAX_KEY_BYTES).contains(&name.len())
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

impl ClusterConnection {
    /// One GET. Refuses before anything is kept (decision 4); a Secret's data is wiped before the
    /// base returns.
    pub async fn values_base(&self, target: &ObjectRef) -> Result<ValuesBase, ValuesBaseError> {
        match target.builtin_kind() {
            Some(ObjectKind::Secret) => {
                let namespace = target.namespace().ok_or(ValuesBaseError::NotEditable)?;
                let body = self.secret_text(namespace, target.name(), ACTION).await?;
                let secret = decode_secret(self.context(), &body)?;
                drop(body);
                secret_base(self.context(), secret, target)
            }
            Some(ObjectKind::ConfigMap) => {
                let object = self.get_object(target, ACTION).await?;
                let config_map: ConfigMap = serde_json::from_value(object)
                    .map_err(|_| self.unexpected_response(ACTION, DECODE_FAILURE))?;
                config_map_base(self.context(), config_map, target)
            }
            _ => Err(ValuesBaseError::NotEditable),
        }
    }
}

/// Builds the base of a Secret, then wipes the Secret whatever the result is.
fn secret_base(
    context: &str,
    mut secret: Secret,
    target: &ObjectRef,
) -> Result<ValuesBase, ValuesBaseError> {
    let base = secret_base_of(context, &secret, target);
    wipe_secret(&mut secret);
    base
}

fn secret_base_of(
    context: &str,
    secret: &Secret,
    target: &ObjectRef,
) -> Result<ValuesBase, ValuesBaseError> {
    let secret_type = secret.type_.as_deref();
    if secret_type == Some(HELM_RELEASE_TYPE) {
        return Err(ValuesBaseError::HelmRelease);
    }
    refuse_by_flags(&secret.metadata, secret_type, secret.immutable)?;
    let keys = secret_summary(secret)
        .keys
        .into_iter()
        .map(|key| ValueKey {
            name: key.name,
            field: DataField::Data,
            size_bytes: key.size_bytes,
            content: if key.is_binary {
                KeyContent::Binary
            } else {
                KeyContent::Hidden
            },
        })
        .collect();
    finish_base(context, &secret.metadata, target, keys)
}

fn config_map_base(
    context: &str,
    mut config_map: ConfigMap,
    target: &ObjectRef,
) -> Result<ValuesBase, ValuesBaseError> {
    refuse_by_flags(&config_map.metadata, None, config_map.immutable)?;
    let text = config_map
        .data
        .take()
        .into_iter()
        .flatten()
        .map(|(name, value)| {
            let size_bytes = value.len();
            let content = if size_bytes > MAX_INLINE_VALUE {
                KeyContent::TooLarge
            } else {
                KeyContent::Text(value)
            };
            ValueKey {
                name,
                field: DataField::Data,
                size_bytes,
                content,
            }
        });
    let binary = config_map
        .binary_data
        .iter()
        .flatten()
        .map(|(name, value)| ValueKey {
            name: name.clone(),
            field: DataField::BinaryData,
            size_bytes: value.0.len(),
            content: KeyContent::Binary,
        });
    let mut keys: Vec<ValueKey> = text.chain(binary).collect();
    keys.sort_by(|left, right| left.name.cmp(&right.name));
    finish_base(context, &config_map.metadata, target, keys)
}

/// The refusals both kinds share: the `owner=helm` label (Helm's ConfigMap and Secret storage
/// drivers), a service account token, and an immutable object.
fn refuse_by_flags(
    metadata: &ObjectMeta,
    secret_type: Option<&str>,
    immutable: Option<bool>,
) -> Result<(), ValuesBaseError> {
    let is_helm_record = metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get(OWNER_LABEL))
        .is_some_and(|owner| owner == HELM_OWNER);
    if is_helm_record {
        return Err(ValuesBaseError::HelmRelease);
    }
    if secret_type == Some(SERVICE_ACCOUNT_TOKEN_TYPE) {
        return Err(ValuesBaseError::ServiceAccountToken);
    }
    if immutable == Some(true) {
        return Err(ValuesBaseError::Immutable);
    }
    Ok(())
}

fn finish_base(
    context: &str,
    metadata: &ObjectMeta,
    target: &ObjectRef,
    keys: Vec<ValueKey>,
) -> Result<ValuesBase, ValuesBaseError> {
    let resource_version = metadata
        .resource_version
        .clone()
        .filter(|version| !version.is_empty())
        .ok_or_else(|| {
            ValuesBaseError::Cluster(ClusterError::UnexpectedResponse {
                context: context.to_owned(),
                action: ACTION,
                source: MISSING_VERSION.into(),
            })
        })?;
    let is_helm_managed = is_helm_managed(
        metadata
            .labels
            .iter()
            .flatten()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    let owner = metadata
        .owner_references
        .iter()
        .flatten()
        .next()
        .map(|owner| format!("{}/{}", owner.kind, owner.name));
    Ok(ValuesBase {
        target: target.clone(),
        resource_version,
        keys,
        notes: BaseNotes {
            is_helm_managed,
            owner,
        },
    })
}

/// Wipes every value and annotation (annotations can embed the data through
/// `last-applied-configuration`).
fn wipe_secret(secret: &mut Secret) {
    for value in secret.data.iter_mut().flat_map(BTreeMap::values_mut) {
        value.0.zeroize();
    }
    for value in secret
        .metadata
        .annotations
        .iter_mut()
        .flat_map(BTreeMap::values_mut)
    {
        value.zeroize();
    }
}

/// The merge-patch body: the base `resourceVersion` and the changed keys only. A removal is
/// `null`. A Secret's text goes out base64-encoded in `data`; no other Secret field is sent.
pub(crate) fn values_patch(edit: &ValuesEdit) -> Value {
    let mut data = Map::new();
    let mut binary_data = Map::new();
    for paired in &edit.changes {
        let wire = match paired.change.value() {
            Some(value) if edit.is_secret() => Value::from(STANDARD.encode(value.0.as_bytes())),
            Some(value) => Value::from(value.0.as_str()),
            None => Value::Null,
        };
        let map = match paired.field {
            DataField::Data => &mut data,
            DataField::BinaryData => &mut binary_data,
        };
        map.insert(paired.change.key().to_owned(), wire);
    }
    let mut body = Map::new();
    body.insert(
        "metadata".to_owned(),
        json!({ "resourceVersion": edit.resource_version }),
    );
    if !data.is_empty() {
        body.insert("data".to_owned(), Value::Object(data));
    }
    if !binary_data.is_empty() {
        body.insert("binaryData".to_owned(), Value::Object(binary_data));
    }
    Value::Object(body)
}

#[cfg(test)]
#[path = "config_values_tests.rs"]
mod config_values_tests;
