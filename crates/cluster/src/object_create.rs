//! A new object parsed from editor text and checked (spec 0042): the body of `CreateObject`.
//!
//! The text is user input and can hold credentials, so nothing here logs, and `Debug` shows the
//! kind, namespace, and name only.

use std::borrow::Cow;
use std::fmt;

use kube::api::DynamicObject;
use serde_json::Value;

use crate::dns_name::{is_dns_label, is_dns_subdomain, is_path_segment_name};
use crate::edit_placeholders::MARKER_PREFIX;
use crate::edit_preview::{FieldPath, PathSegment};
use crate::object_edit::{
    EditError, SERVER_METADATA, parse_mapping, refuse_leading_zero, serialization_error,
};
use crate::object_write::ChangedField;
use crate::object_yaml::{ObjectKind, ObjectRef, api_resource, to_yaml_text};

/// Lists in the confirm and the audit line stop here (decision 16).
const MAX_LISTED: usize = 10;
const POWERFUL_ROLES: [&str; 3] = ["cluster-admin", "admin", "edit"];
const POD_SECURITY_ENFORCE: &str = "pod-security.kubernetes.io/enforce";
const KUBE_SYSTEM: &str = "kube-system";

impl ObjectKind {
    /// The kinds W7 gives a `New` button (decision 1): a closed list, so a new kind is a reviewed
    /// code change.
    pub fn is_creatable(self) -> bool {
        matches!(
            self,
            Self::Namespace
                | Self::ConfigMap
                | Self::ResourceQuota
                | Self::PodDisruptionBudget
                | Self::RoleBinding
        )
    }
}

/// A new object parsed from the editor text and checked. Always valid: a creatable kind, the
/// kind's `apiVersion`, a valid name and namespace, and no server-owned field.
// Debug is manual: kind, namespace, and name; never a value.
#[derive(Clone, PartialEq, Eq)]
pub struct ObjectDraft {
    target: ObjectRef,
    body: Value,
    warnings: Vec<DraftWarning>,
}

impl fmt::Debug for ObjectDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectDraft")
            .field("kind", &format_args!("{}", self.target.kind_name()))
            .field("namespace", &self.target.namespace())
            .field("name", &self.target.name())
            .finish()
    }
}

/// Something the user should read before sending the draft (decision 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftWarning {
    /// The `roleRef` is `cluster-admin`, `admin`, or `edit`.
    PowerfulRole { role: String },
    /// A Group or User subject whose name starts with `system:`.
    BroadSubject { kind: String, name: String },
    /// The Namespace enforces the `privileged` pod security level.
    PrivilegedPodSecurity,
    /// A ServiceAccount subject in `kube-system`, where the control plane's own accounts live.
    SystemNamespaceAccount { name: String },
    /// The `roleRef` is a ClusterRole whose name starts with `system:`.
    SystemRole { role: String },
}

impl DraftWarning {
    /// Whether the confirm asks for the object's name on every environment (decision 15).
    pub fn needs_typed_name(&self) -> bool {
        matches!(self, Self::PowerfulRole { .. } | Self::BroadSubject { .. })
    }
}

/// Why a text is not a creatable object. Every message names the field.
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    /// Syntax, not a single mapping, a leading-zero number, or too large.
    #[error(transparent)]
    Text(#[from] EditError),
    #[error("{0} cannot be created here")]
    NotCreatable(&'static str),
    #[error("kind must be {expected}")]
    WrongKind { expected: &'static str },
    #[error("apiVersion must be {expected}")]
    WrongApiVersion { expected: String },
    #[error("metadata.name is required")]
    MissingName,
    #[error("metadata.generateName is not supported; write a name")]
    GenerateName,
    #[error("metadata.name is not a valid {kind} name")]
    InvalidName { kind: &'static str },
    #[error("metadata.namespace is required")]
    MissingNamespace,
    #[error("a {kind} has no namespace")]
    UnexpectedNamespace { kind: &'static str },
    /// Every server-owned field of the text at once, as `kubectl get -o yaml` leaves them.
    #[error("{}", server_fields_message(fields))]
    ServerFields { fields: Vec<String> },
    /// Several YAML documents: the editor holds one object.
    #[error("Found {count} documents; paste one")]
    SeveralDocuments { count: usize },
    #[error("{path} holds <hidden>; write a value")]
    Placeholder { path: String },
    /// A `metadata` field of the wrong type (for example `labels: 5`).
    #[error("metadata is not valid: check labels, annotations, and finalizers")]
    InvalidMetadata,
}

impl ObjectDraft {
    /// Parses `text` as a new object of `kind`, in this order: creatable kind, syntax and a single
    /// mapping, `kind` and `apiVersion`, server fields, name, namespace, placeholders.
    pub fn new(kind: ObjectKind, text: &str) -> Result<Self, DraftError> {
        if !kind.is_creatable() {
            return Err(DraftError::NotCreatable(kind.name()));
        }
        let count = documents(text).len();
        if count > 1 {
            return Err(DraftError::SeveralDocuments { count });
        }
        let body = parse_mapping(text)?;
        refuse_leading_zero(text)?;
        if body.get("kind").and_then(Value::as_str) != Some(kind.name()) {
            return Err(DraftError::WrongKind {
                expected: kind.name(),
            });
        }
        let api_version = api_resource(kind).api_version;
        if body.get("apiVersion").and_then(Value::as_str) != Some(api_version.as_str()) {
            return Err(DraftError::WrongApiVersion {
                expected: api_version,
            });
        }
        let fields = server_fields(&body);
        if !fields.is_empty() {
            return Err(DraftError::ServerFields { fields });
        }
        let metadata = body.get("metadata");
        if metadata.is_some_and(|value| value.get("generateName").is_some()) {
            return Err(DraftError::GenerateName);
        }
        let name = text_at(&body, "/metadata/name")
            .filter(|name| !name.is_empty())
            .ok_or(DraftError::MissingName)?;
        if !is_valid_name(kind, name) {
            return Err(DraftError::InvalidName { kind: kind.name() });
        }
        let namespace = namespace_of(kind, &body)?;
        if serde_json::from_value::<DynamicObject>(body.clone()).is_err() {
            return Err(DraftError::InvalidMetadata);
        }
        if let Some(path) = find_placeholder(&body, &mut Vec::new()) {
            return Err(DraftError::Placeholder {
                path: path.to_string(),
            });
        }
        let target = ObjectRef::new(kind, namespace, name.to_owned())
            .ok_or(DraftError::NotCreatable(kind.name()))?;
        let warnings = warnings_of(kind, &body);
        Ok(Self {
            target,
            body,
            warnings,
        })
    }

    pub fn target(&self) -> &ObjectRef {
        &self.target
    }

    pub fn warnings(&self) -> &[DraftWarning] {
        &self.warnings
    }

    pub(crate) fn body(&self) -> &Value {
        &self.body
    }

    /// Whether the body still agrees with the target and holds no server-owned field:
    /// `checked_operation` asks again, so a draft built in another way cannot reach the wire.
    pub(crate) fn is_consistent(&self) -> bool {
        let Some(kind) = self.target.builtin_kind() else {
            return false;
        };
        kind.is_creatable()
            && text_at(&self.body, "/kind") == Some(kind.name())
            && text_at(&self.body, "/apiVersion") == Some(api_resource(kind).api_version.as_str())
            && text_at(&self.body, "/metadata/name") == Some(self.target.name())
            && text_at(&self.body, "/metadata/namespace") == self.target.namespace()
            && server_fields(&self.body).is_empty()
            && self.body.pointer("/metadata/generateName").is_none()
    }

    /// The fields for the confirm and the audit line: names and paths, never a ConfigMap value
    /// (decision 16). Lists stop at ten items, then one `… and {n} more` field.
    pub(crate) fn changed_fields(&self) -> Vec<ChangedField> {
        let mut fields = vec![field(
            Cow::Borrowed("metadata.name"),
            Some(self.target.name().to_owned()),
        )];
        if let Some(namespace) = self.target.namespace() {
            fields.push(field(
                Cow::Borrowed("metadata.namespace"),
                Some(namespace.to_owned()),
            ));
        }
        fields.extend(self.finalizer_fields());
        let kind = self.target.builtin_kind();
        match kind {
            Some(ObjectKind::RoleBinding) => fields.extend(self.role_binding_fields()),
            Some(ObjectKind::ConfigMap) => fields.extend(self.config_map_fields()),
            Some(ObjectKind::ResourceQuota) => fields.extend(self.quota_fields()),
            Some(ObjectKind::PodDisruptionBudget) => fields.extend(self.budget_fields()),
            _ => {}
        }
        fields
    }

    /// A finalizer the draft sets is listed: a name nothing removes keeps the object (a Namespace
    /// especially) from ever being deleted.
    fn finalizer_fields(&self) -> Vec<ChangedField> {
        let finalizers = self
            .body
            .pointer("/metadata/finalizers")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let listed = finalizers
            .iter()
            .enumerate()
            .map(|(index, name)| {
                field(
                    Cow::Owned(format!("metadata.finalizers[{index}]")),
                    Some(scalar_text(name)),
                )
            })
            .collect();
        capped(listed)
    }

    fn role_binding_fields(&self) -> Vec<ChangedField> {
        let mut fields = Vec::new();
        if let Some(role) = self.body.get("roleRef") {
            let text = |key: &str| role.get(key).and_then(Value::as_str).unwrap_or_default();
            fields.push(field(
                Cow::Borrowed("roleRef"),
                Some(format!("{}/{}", text("kind"), text("name"))),
            ));
        }
        let subjects = self
            .body
            .get("subjects")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let listed = subjects
            .iter()
            .enumerate()
            .map(|(index, subject)| {
                field(
                    Cow::Owned(format!("subjects[{index}]")),
                    Some(subject_text(subject)),
                )
            })
            .collect();
        fields.extend(capped(listed));
        fields
    }

    fn config_map_fields(&self) -> Vec<ChangedField> {
        let keys = ["data", "binaryData"].into_iter().flat_map(|section| {
            self.body
                .get(section)
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(move |map| {
                    map.keys()
                        .map(move |key| field(Cow::Owned(format!("{section}[{key}]")), None))
                })
        });
        capped(keys.collect())
    }

    fn quota_fields(&self) -> Vec<ChangedField> {
        let hard = self.body.pointer("/spec/hard").and_then(Value::as_object);
        let listed = hard.into_iter().flatten().map(|(resource, quantity)| {
            field(
                Cow::Owned(format!("spec.hard[{resource}]")),
                Some(scalar_text(quantity)),
            )
        });
        capped(listed.collect())
    }

    fn budget_fields(&self) -> Vec<ChangedField> {
        [
            ("spec.minAvailable", "/spec/minAvailable"),
            ("spec.maxUnavailable", "/spec/maxUnavailable"),
        ]
        .into_iter()
        .filter_map(|(path, pointer)| {
            let value = self.body.pointer(pointer)?;
            Some(field(Cow::Borrowed(path), Some(scalar_text(value))))
        })
        .collect()
    }
}

/// The name rule of the API server per kind (decision 9): the RBAC kinds accept `:`.
pub(crate) fn is_valid_name(kind: ObjectKind, name: &str) -> bool {
    match kind {
        ObjectKind::Namespace => is_dns_label(name),
        ObjectKind::RoleBinding => is_path_segment_name(name),
        _ => is_dns_subdomain(name),
    }
}

/// The leaf paths of `draft` that `answer` lacks (decision 14): a field the server dropped.
/// Lists are compared by index; an empty map, an empty list, and `null` are skipped because the
/// server omits them; a field the server added is not looked at. Paths only, never values.
pub(crate) fn missing_paths(draft: &Value, answer: &Value) -> Vec<FieldPath> {
    let mut paths = Vec::new();
    collect_missing(draft, Some(answer), &mut Vec::new(), &mut paths);
    paths
}

fn collect_missing(
    draft: &Value,
    answer: Option<&Value>,
    at: &mut Vec<PathSegment>,
    paths: &mut Vec<FieldPath>,
) {
    let answer = answer.filter(|value| !value.is_null());
    match draft {
        Value::Null => {}
        Value::Object(map) if map.is_empty() => {}
        // The server omits an empty string like a `null`: a ServiceAccount subject with
        // `apiGroup: ""` comes back without it.
        Value::String(text) if text.is_empty() => {}
        Value::Array(items) if items.is_empty() => {}
        Value::Object(map) => {
            for (key, child) in map {
                at.push(PathSegment::Key(key.clone()));
                collect_missing(child, answer.and_then(|value| value.get(key)), at, paths);
                at.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                at.push(PathSegment::Index(index));
                collect_missing(child, answer.and_then(|value| value.get(index)), at, paths);
                at.pop();
            }
        }
        _ if answer.is_none() => paths.push(FieldPath::new(at.clone())),
        _ => {}
    }
}

fn field(path: Cow<'static, str>, value: Option<String>) -> ChangedField {
    ChangedField { path, value }
}

/// At most `MAX_LISTED` fields, then one `… and {n} more`.
fn capped(mut fields: Vec<ChangedField>) -> Vec<ChangedField> {
    let more = fields.len().saturating_sub(MAX_LISTED);
    if more > 0 {
        fields.truncate(MAX_LISTED);
        fields.push(field(Cow::Owned(format!("\u{2026} and {more} more")), None));
    }
    fields
}

/// `{Kind} {ns/}{name}` of a RoleBinding subject.
fn subject_text(subject: &Value) -> String {
    let text = |key: &str| subject.get(key).and_then(Value::as_str).unwrap_or_default();
    match subject.get("namespace").and_then(Value::as_str) {
        Some(namespace) if !namespace.is_empty() => {
            format!("{} {namespace}/{}", text("kind"), text("name"))
        }
        _ => format!("{} {}", text("kind"), text("name")),
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn text_at<'a>(body: &'a Value, pointer: &str) -> Option<&'a str> {
    body.pointer(pointer).and_then(Value::as_str)
}

/// Every server-owned field of `body` as a path, `status` first.
fn server_fields(body: &Value) -> Vec<String> {
    let status = body.get("status").map(|_| "status".to_owned());
    let metadata = body.get("metadata");
    let owned = SERVER_METADATA
        .into_iter()
        .chain(["ownerReferences"])
        .filter(|field| metadata.is_some_and(|metadata| metadata.get(field).is_some()))
        .map(|field| format!("metadata.{field}"));
    status.into_iter().chain(owned).collect()
}

/// The error text: the fields are listed once, so the user sees all of them before fixing.
fn server_fields_message(fields: &[String]) -> String {
    match fields {
        [field] => format!("{field} is set by the server; remove it"),
        fields => format!(
            "{} fields are set by the server: {}; remove them",
            fields.len(),
            fields.join(", ")
        ),
    }
}

/// What the editor can do about a failed draft in one click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftFix {
    /// Take every server-owned field out of the text.
    RemoveServerFields,
    /// Keep the first YAML document only.
    KeepFirstDocument,
}

impl DraftError {
    /// The one-click fix of this error, if it has one.
    pub fn fix(&self) -> Option<DraftFix> {
        match self {
            Self::ServerFields { .. } => Some(DraftFix::RemoveServerFields),
            Self::SeveralDocuments { .. } => Some(DraftFix::KeepFirstDocument),
            _ => None,
        }
    }
}

impl DraftFix {
    /// The text with the fix applied. Removing server fields re-serializes the object, like Format.
    pub fn apply(self, text: &str) -> Result<String, EditError> {
        match self {
            Self::KeepFirstDocument => Ok(documents(text).into_iter().next().unwrap_or_default()),
            Self::RemoveServerFields => {
                let mut body = parse_mapping(text)?;
                if let Some(root) = body.as_object_mut() {
                    root.remove("status");
                    if let Some(metadata) = root.get_mut("metadata").and_then(Value::as_object_mut)
                    {
                        for field in SERVER_METADATA.into_iter().chain(["ownerReferences"]) {
                            metadata.remove(field);
                        }
                    }
                }
                body.sort_all_objects();
                to_yaml_text(&body, None).map_err(serialization_error)
            }
        }
    }
}

/// A namespaced kind needs a DNS-label namespace; a Namespace has none.
fn namespace_of(kind: ObjectKind, body: &Value) -> Result<Option<String>, DraftError> {
    let namespace = body.pointer("/metadata/namespace");
    if !kind.is_namespaced() {
        return match namespace {
            Some(_) => Err(DraftError::UnexpectedNamespace { kind: kind.name() }),
            None => Ok(None),
        };
    }
    namespace
        .and_then(Value::as_str)
        .filter(|namespace| is_dns_label(namespace))
        .map(|namespace| Some(namespace.to_owned()))
        .ok_or(DraftError::MissingNamespace)
}

/// The first string that is `<hidden>` or starts like a diff marker, in key order.
fn find_placeholder(value: &Value, at: &mut Vec<PathSegment>) -> Option<FieldPath> {
    match value {
        Value::String(text) if text.starts_with(MARKER_PREFIX) => Some(FieldPath::new(at.clone())),
        Value::Object(map) => map.iter().find_map(|(key, child)| {
            at.push(PathSegment::Key(key.clone()));
            let found = find_placeholder(child, at);
            at.pop();
            found
        }),
        Value::Array(items) => items.iter().enumerate().find_map(|(index, child)| {
            at.push(PathSegment::Index(index));
            let found = find_placeholder(child, at);
            at.pop();
            found
        }),
        _ => None,
    }
}

fn warnings_of(kind: ObjectKind, body: &Value) -> Vec<DraftWarning> {
    match kind {
        ObjectKind::RoleBinding => role_binding_warnings(body),
        ObjectKind::Namespace => {
            let enforce = body
                .pointer("/metadata/labels")
                .and_then(|labels| labels.get(POD_SECURITY_ENFORCE))
                .and_then(Value::as_str);
            if enforce == Some("privileged") {
                vec![DraftWarning::PrivilegedPodSecurity]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn role_binding_warnings(body: &Value) -> Vec<DraftWarning> {
    let mut warnings = Vec::new();
    let role = body.get("roleRef");
    let role_name = role
        .and_then(|role| role.get("name"))
        .and_then(Value::as_str);
    let role_kind = role
        .and_then(|role| role.get("kind"))
        .and_then(Value::as_str);
    if let (Some("ClusterRole"), Some(name)) = (role_kind, role_name)
        && POWERFUL_ROLES.contains(&name)
    {
        warnings.push(DraftWarning::PowerfulRole {
            role: name.to_owned(),
        });
    }
    if let (Some("ClusterRole"), Some(name)) = (role_kind, role_name)
        && name.starts_with("system:")
    {
        warnings.push(DraftWarning::SystemRole {
            role: name.to_owned(),
        });
    }
    let subjects = body.get("subjects").and_then(Value::as_array);
    for subject in subjects.into_iter().flatten() {
        let kind = subject.get("kind").and_then(Value::as_str);
        let name = subject.get("name").and_then(Value::as_str);
        if let (Some(kind @ ("Group" | "User")), Some(name)) = (kind, name)
            && name.starts_with("system:")
        {
            warnings.push(DraftWarning::BroadSubject {
                kind: kind.to_owned(),
                name: name.to_owned(),
            });
        }
        let namespace = subject.get("namespace").and_then(Value::as_str);
        if let (Some("ServiceAccount"), Some(name), Some(KUBE_SYSTEM)) = (kind, name, namespace) {
            warnings.push(DraftWarning::SystemNamespaceAccount {
                name: name.to_owned(),
            });
        }
    }
    warnings
}

/// The documents of a multi-document text that have content, each as its own text. A leading `---`
/// marker is the usual start of one document, and a document of comments only is not one.
fn documents(text: &str) -> Vec<String> {
    let mut documents = Vec::new();
    let mut current = String::new();
    let mut has_content = false;
    let mut finish = |current: &mut String, has_content: &mut bool| {
        if *has_content {
            documents.push(std::mem::take(current));
        }
        current.clear();
        *has_content = false;
    };
    for line in text.lines() {
        let marker = line.trim_end();
        if marker == "---" || marker.starts_with("--- ") {
            finish(&mut current, &mut has_content);
            continue;
        }
        has_content |= !marker.is_empty() && !marker.trim_start().starts_with('#');
        current.push_str(line);
        current.push('\n');
    }
    finish(&mut current, &mut has_content);
    documents
}

#[cfg(test)]
#[path = "object_create_tests.rs"]
mod object_create_tests;
