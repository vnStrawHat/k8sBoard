//! The edit model of Edit YAML (spec 0031): the object as the editor opens it, and an edit
//! checked against it. Everything here is local and pure after the one GET; nothing is sent.
//!
//! The trees can hold secrets, so this module never logs and `Debug` shows counts and names only.

use std::fmt;

use serde_json::Value;

use crate::config_values::is_helm_managed;
use crate::connection::{ClusterConnection, ClusterError};
use crate::edit_placeholders::{self, Unrestorable};
use crate::edit_preview::{FieldPath, copy_path, field_paths, has_last_applied};
use crate::object_yaml::{EnvValues, ObjectKind, ObjectRef, mask_object, to_yaml_text};

const ACTION: &str = "reading the object to edit";
const EDIT_HEADER: &str =
    "# Values shown as <hidden> keep their value on the server. Replace one to set a new value.";
const SECRET_HEADER: &str = "# Secret data and stringData cannot be edited here.";
const MISSING_VERSION: &str = "the object has no resourceVersion or uid";
/// The editor text is parsed on the main thread, so a larger text is refused before it is read.
const MAX_EDIT_BYTES: usize = 2 * 1024 * 1024;
const HELM_RELEASE: &str = "the object is a Helm release record and cannot be edited here";
/// The `type` of the Secret that stores a Helm release.
pub(crate) const HELM_RELEASE_TYPE: &str = "helm.sh/release.v1";
/// Owned by the server (0031 decision 12): never shown in the editor, stripped when typed back.
pub(crate) const SERVER_METADATA: [&str; 8] = [
    "managedFields",
    "resourceVersion",
    "uid",
    "generation",
    "creationTimestamp",
    "deletionTimestamp",
    "deletionGracePeriodSeconds",
    "selfLink",
];
/// The object as the editor opened it, stripped and masked.
// Debug is manual: kind, name, and resourceVersion.
pub struct EditBase {
    target: ObjectRef,
    resource_version: String,
    uid: String,
    env: EnvValues,
    masked: Value,
    text: String,
}

impl fmt::Debug for EditBase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EditBase")
            .field("kind", &format_args!("{}", self.target.kind_name()))
            .field("name", &self.target.name())
            .field("resource_version", &self.resource_version)
            .finish()
    }
}

impl EditBase {
    /// Strips, masks, and serializes a raw object. The error is a fixed message.
    fn from_object(
        target: ObjectRef,
        mut object: Value,
        env: EnvValues,
    ) -> Result<Self, &'static str> {
        if is_helm_release(&object) {
            return Err(HELM_RELEASE);
        }
        let metadata_text = |object: &Value, field: &str| {
            object
                .pointer("/metadata")
                .and_then(|metadata| metadata.get(field))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let resource_version = metadata_text(&object, "resourceVersion").ok_or(MISSING_VERSION)?;
        let uid = metadata_text(&object, "uid").ok_or(MISSING_VERSION)?;
        strip_server_fields(&mut object);
        set_target_kind(&mut object, &target);
        mask_object(&mut object, env, |_| 0);
        let mut base = Self {
            target,
            resource_version,
            uid,
            env,
            masked: object,
            text: String::new(),
        };
        base.text = to_yaml_text(&base.masked, Some(&edit_header(base.is_secret())))?;
        Ok(base)
    }

    pub fn target(&self) -> &ObjectRef {
        &self.target
    }

    pub fn resource_version(&self) -> &str {
        &self.resource_version
    }

    pub fn env(&self) -> EnvValues {
        self.env
    }

    /// The editor text: the masked object under the edit header.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_secret(&self) -> bool {
        self.target.builtin_kind() == Some(ObjectKind::Secret)
    }

    /// Whether the object's `managed-by` label says Helm renders it.
    pub fn is_helm_managed(&self) -> bool {
        let labels = self
            .masked
            .pointer("/metadata/labels")
            .and_then(Value::as_object);
        is_helm_managed(
            labels
                .into_iter()
                .flatten()
                .filter_map(|(key, value)| Some((key.as_str(), value.as_str()?))),
        )
    }

    /// Whether the object carries `kubectl.kubernetes.io/last-applied-configuration`.
    pub fn has_last_applied(&self) -> bool {
        has_last_applied(&self.masked)
    }
}

impl ClusterConnection {
    /// One GET of `object`, stripped (decision 12), masked, and serialized under the edit header.
    pub async fn edit_base(
        &self,
        object: &ObjectRef,
        env: EnvValues,
    ) -> Result<EditBase, ClusterError> {
        let value = self.get_object(object, ACTION).await?;
        EditBase::from_object(object.clone(), value, env)
            .map_err(|message| self.unexpected_response(ACTION, message))
    }
}

/// The comment lines above the editor text; a Secret adds that its data is locked.
fn edit_header(is_secret: bool) -> String {
    if is_secret {
        format!("{EDIT_HEADER}\n{SECRET_HEADER}")
    } else {
        EDIT_HEADER.to_owned()
    }
}

/// Whether `object` is the Secret that stores a Helm release (0038 owns those; Edit YAML never
/// writes one).
pub(crate) fn is_helm_release(object: &Value) -> bool {
    object.get("type").and_then(Value::as_str) == Some(HELM_RELEASE_TYPE)
}

/// Sets `kind` to the target's. The mask rules (a Secret's `data`) key on the object's own `kind`,
/// and an answer can omit it, so the target decides what is masked, not the answer.
pub(crate) fn set_target_kind(object: &mut Value, target: &ObjectRef) {
    if let Some(root) = object.as_object_mut() {
        root.insert("kind".to_owned(), Value::from(target.kind_name()));
    }
}

/// Removes what the server owns: `status` and the server metadata of decision 12.
pub(crate) fn strip_server_fields(object: &mut Value) {
    let Some(root) = object.as_object_mut() else {
        return;
    };
    root.remove("status");
    let Some(metadata) = root.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    for field in SERVER_METADATA {
        metadata.remove(field);
    }
}

/// A parsed edit checked against its base. Always valid: the identity is unchanged, every
/// placeholder has a counterpart, a Secret keeps its data, and something changed.
// Debug is manual: kind, name, and the change count.
#[derive(Clone, PartialEq, Eq)]
pub struct ObjectEdit {
    target: ObjectRef,
    base_resource_version: String,
    base_uid: String,
    env: EnvValues,
    edited: Value,
    changed: Vec<FieldPath>,
    leading_zero_lines: Vec<usize>,
}

impl fmt::Debug for ObjectEdit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectEdit")
            .field("kind", &format_args!("{}", self.target.kind_name()))
            .field("name", &self.target.name())
            .field("changes", &self.changed.len())
            .finish()
    }
}

/// Why a text is not an applicable edit.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("YAML error at line {line}, column {column}: {message}")]
    Syntax {
        line: u64,
        column: u64,
        message: String,
    },
    #[error("the document must be a single YAML mapping")]
    NotAnObject,
    #[error("{field} cannot change in an edit")]
    IdentityChanged { field: &'static str },
    #[error("Secret values cannot be edited here")]
    SecretValuesChanged,
    #[error("{path} is <hidden> but has no value on the server; replace it or remove it")]
    UnmatchedPlaceholder { path: String },
    /// Text that starts with `<hidden` but is not the placeholder: the diff view's markers.
    #[error("{path} holds diff marker text; write a value or <hidden>")]
    MarkerText { path: String },
    /// A rebase onto an object that was deleted and created again: the text was for the old one.
    #[error("the object was deleted and created again")]
    Recreated,
    /// A number with a leading zero, which a re-serialization would silently turn into decimal.
    #[error(
        "line {line}: a number with a leading zero is read as decimal (YAML 1.2); quote it, or write the decimal or 0o value"
    )]
    LeadingZero { line: usize },
    #[error("the text is larger than 2 MiB")]
    TooLarge,
    #[error("nothing changed")]
    NoChanges,
}

impl ObjectEdit {
    /// Parses `text` and checks it against `base`, in this order: syntax, a mapping, the stripped
    /// server fields, the identity, the Secret lock, the placeholders, and at least one change.
    pub fn new(base: &EditBase, text: &str) -> Result<Self, EditError> {
        let mut edited = parse_mapping(text)?;
        strip_server_fields(&mut edited);
        check_identity(&base.masked, &edited)?;
        if base.is_secret() && secret_values_differ(&base.masked, &edited) {
            return Err(EditError::SecretValuesChanged);
        }
        edit_placeholders::check(&edited, &base.masked).map_err(|error| {
            let path = error.path().to_string();
            match error {
                Unrestorable::Unmatched(_) => EditError::UnmatchedPlaceholder { path },
                Unrestorable::MarkerText(_) => EditError::MarkerText { path },
            }
        })?;
        let changed = field_paths(&base.masked, &edited);
        if changed.is_empty() {
            return Err(EditError::NoChanges);
        }
        Ok(Self {
            target: base.target.clone(),
            base_resource_version: base.resource_version.clone(),
            base_uid: base.uid.clone(),
            env: base.env,
            edited,
            changed,
            leading_zero_lines: leading_zero_lines(text),
        })
    }

    pub fn target(&self) -> &ObjectRef {
        &self.target
    }

    /// The fields the user changed, for the dialog and the audit; never their values.
    pub fn changed_paths(&self) -> &[FieldPath] {
        &self.changed
    }

    /// The 1-based lines with an unquoted number that starts with `0` (decision 18).
    pub fn leading_zero_lines(&self) -> &[usize] {
        &self.leading_zero_lines
    }

    pub(crate) fn env(&self) -> EnvValues {
        self.env
    }

    pub(crate) fn base_resource_version(&self) -> &str {
        &self.base_resource_version
    }

    pub(crate) fn base_uid(&self) -> &str {
        &self.base_uid
    }

    pub(crate) fn edited(&self) -> &Value {
        &self.edited
    }
}

/// Re-serializes the editor text the way the editor text is built: sorted keys, the same
/// serializer, the leading comment lines kept. Quoting and comments inside the body are not kept.
pub fn format_yaml(text: &str) -> Result<String, EditError> {
    refuse_leading_zero(text)?;
    let mut value = parse_mapping(text)?;
    value.sort_all_objects();
    let header: Vec<&str> = text
        .lines()
        .take_while(|line| line.starts_with('#'))
        .collect();
    let header = (!header.is_empty()).then(|| header.join("\n"));
    to_yaml_text(&value, header.as_deref()).map_err(serialization_error)
}

/// The text of the failure is a fixed message of the serializer.
fn serialization_error(message: &'static str) -> EditError {
    EditError::Syntax {
        line: 0,
        column: 0,
        message: message.to_owned(),
    }
}

/// The user's edit moved onto a newer object (spec 0031 decision 21).
// Debug is manual: counts only.
pub struct Rebased {
    /// The new base's object with the user's changed paths applied, under the edit header.
    pub text: String,
    /// Paths of the user's changes that have no place on the new object.
    pub unreachable: Vec<FieldPath>,
    /// What changed on the server between the two bases.
    pub server_changed: Vec<FieldPath>,
    /// Paths of the user's changes that equal, contain, or lie inside a path the server changed:
    /// the user's value replaces the server's change. A list that became one path is here too.
    pub overwritten: Vec<FieldPath>,
}

impl fmt::Debug for Rebased {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Rebased")
            .field("unreachable", &self.unreachable.len())
            .field("server_changed", &self.server_changed.len())
            .field("overwritten", &self.overwritten.len())
            .finish()
    }
}

/// Re-applies the paths `text` changed against `old` onto `new`: each path is set (or removed) in
/// the new object, so a concurrent change to another path, or to another list item, stays. Where
/// both sides changed one path, the user's value wins. A `<hidden>` copied from the text keeps
/// meaning "the server's value", now the new one. The error is a syntax error or a non-mapping
/// only; the identity and the placeholders are checked when the rebased text is applied.
pub fn rebase(old: &EditBase, text: &str, new: &EditBase) -> Result<Rebased, EditError> {
    // A different uid is another object with the same name: the text was written for the old one.
    if old.uid != new.uid {
        return Err(EditError::Recreated);
    }
    refuse_leading_zero(text)?;
    let mut edited = parse_mapping(text)?;
    strip_server_fields(&mut edited);
    let server_changed = field_paths(&old.masked, &new.masked);
    let mut result = new.masked.clone();
    let mut unreachable = Vec::new();
    let mut overwritten = Vec::new();
    for path in field_paths(&old.masked, &edited) {
        if server_changed.iter().any(|changed| changed.overlaps(&path)) {
            overwritten.push(path.clone());
        }
        if !copy_path(&mut result, &edited, &path) {
            unreachable.push(path);
        }
    }
    let text =
        to_yaml_text(&result, Some(&edit_header(new.is_secret()))).map_err(serialization_error)?;
    Ok(Rebased {
        text,
        unreachable,
        server_changed,
        overwritten,
    })
}

/// Refuses a text that has a number with a leading zero: the parser reads `0444` as decimal 444,
/// so writing the text again would erase what the warning of `leading_zero_lines` says.
pub(crate) fn refuse_leading_zero(text: &str) -> Result<(), EditError> {
    match leading_zero_lines(text).first() {
        Some(line) => Err(EditError::LeadingZero { line: *line }),
        None => Ok(()),
    }
}

pub(crate) fn parse_mapping(text: &str) -> Result<Value, EditError> {
    if text.len() > MAX_EDIT_BYTES {
        return Err(EditError::TooLarge);
    }
    let mut value = serde_saphyr::from_str::<Value>(text).map_err(|error| {
        let (line, column) = error
            .location()
            .map_or((0, 0), |location| (location.line(), location.column()));
        EditError::Syntax {
            line,
            column,
            message: error.without_snippet().to_string(),
        }
    })?;
    if !value.is_object() {
        return Err(EditError::NotAnObject);
    }
    whole_floats_to_integers(&mut value);
    Ok(value)
}

/// The parser reads some integers as floats (`0755` is `755.0`), and the API server rejects
/// `755.0` for an integer field. kubectl's YAML-to-JSON step prints a float with no fraction as an
/// integer, so this does too.
fn whole_floats_to_integers(value: &mut Value) {
    // Below 2^53 every whole f64 is an exact i64.
    const EXACT_LIMIT: f64 = 9_007_199_254_740_992.0;
    match value {
        Value::Number(number) => {
            if let Some(float) = number.as_f64()
                && number.is_f64()
                && float.fract() == 0.0
                && float.abs() < EXACT_LIMIT
            {
                *value = Value::from(float as i64);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(whole_floats_to_integers),
        Value::Object(map) => map.values_mut().for_each(whole_floats_to_integers),
        _ => {}
    }
}

/// `apiVersion`, `kind`, `metadata.name`, and `metadata.namespace` equal the base's.
fn check_identity(base: &Value, edited: &Value) -> Result<(), EditError> {
    let fields = [
        ("apiVersion", "/apiVersion"),
        ("kind", "/kind"),
        ("metadata.name", "/metadata/name"),
        ("metadata.namespace", "/metadata/namespace"),
    ];
    for (field, pointer) in fields {
        if base.pointer(pointer) != edited.pointer(pointer) {
            return Err(EditError::IdentityChanged { field });
        }
    }
    Ok(())
}

/// Whether `data` or `stringData` differs from the base's (both absent is equal).
fn secret_values_differ(base: &Value, edited: &Value) -> bool {
    ["data", "stringData"]
        .iter()
        .any(|field| base.get(field) != edited.get(field))
}

/// The 1-based lines whose value, after `- ` or the first `: `, is an unquoted `0[0-9]+`.
fn leading_zero_lines(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| has_leading_zero_value(line))
        .map(|(index, _)| index + 1)
        .collect()
}

fn has_leading_zero_value(line: &str) -> bool {
    let mut rest = line.trim_start();
    let mut has_marker = false;
    while let Some(after) = rest.strip_prefix("- ") {
        rest = after.trim_start();
        has_marker = true;
    }
    if let Some((_, value)) = rest.split_once(": ") {
        rest = value;
        has_marker = true;
    }
    if !has_marker {
        return false;
    }
    let value = rest.split(" #").next().unwrap_or(rest).trim();
    value.len() > 1 && value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
#[path = "object_edit_tests.rs"]
mod object_edit_tests;
