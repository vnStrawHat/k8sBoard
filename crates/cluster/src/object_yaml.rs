//! One-shot, masked YAML of a single object, serialized like `kubectl get -o yaml`.
//!
//! The text can embed secrets, so nothing in this module logs, prints, or keeps it: raw
//! objects are masked before they are serialized, and only the masked text leaves the crate.

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod, ResourceQuota,
    Secret, Service, ServiceAccount,
};
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use k8s_openapi::api::storage::v1::StorageClass;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Api;
use kube::api::{ApiResource, DynamicObject};
use serde_json::Value;
use serde_saphyr::SerializerOptions;

use crate::connection::{ClusterConnection, ClusterError};
use crate::custom_resource_definition::{CustomResourceType, ResourceScope, custom_api_resource};
use crate::edit_placeholders::HIDDEN;
use crate::storage_class::mask_mount_option;

const ACTION: &str = "reading the object YAML";
/// Fixed on purpose: the library error could quote the object's content.
const CONVERSION_FAILURE: &str = "the object could not be converted to YAML";

/// Annotations that embed a whole applied manifest, so they can carry Secret data and env literals.
const MASKED_ANNOTATIONS: [&str; 3] = [
    "kubectl.kubernetes.io/last-applied-configuration",
    "kapp.k14s.io/original",
    "kapp.k14s.io/original-diff",
];
const CONTAINER_LISTS: [&str; 3] = ["containers", "initContainers", "ephemeralContainers"];

/// A built-in kind whose objects can be read one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Pod,
    Node,
    Namespace,
    Event,
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
    Job,
    CronJob,
    Service,
    Ingress,
    ConfigMap,
    NetworkPolicy,
    HorizontalPodAutoscaler,
    ResourceQuota,
    PodDisruptionBudget,
    PersistentVolumeClaim,
    PersistentVolume,
    StorageClass,
    ServiceAccount,
    Secret,
    Role,
    ClusterRole,
    RoleBinding,
    ClusterRoleBinding,
    CustomResourceDefinition,
}

impl ObjectKind {
    /// Every kind, in declaration order.
    pub const ALL: [ObjectKind; 27] = [
        Self::Pod,
        Self::Node,
        Self::Namespace,
        Self::Event,
        Self::Deployment,
        Self::StatefulSet,
        Self::DaemonSet,
        Self::ReplicaSet,
        Self::Job,
        Self::CronJob,
        Self::Service,
        Self::Ingress,
        Self::ConfigMap,
        Self::NetworkPolicy,
        Self::HorizontalPodAutoscaler,
        Self::ResourceQuota,
        Self::PodDisruptionBudget,
        Self::PersistentVolumeClaim,
        Self::PersistentVolume,
        Self::StorageClass,
        Self::ServiceAccount,
        Self::Secret,
        Self::Role,
        Self::ClusterRole,
        Self::RoleBinding,
        Self::ClusterRoleBinding,
        Self::CustomResourceDefinition,
    ];

    /// The API group and the plural resource name, for example `("apps", "deployments")`; the
    /// core group is empty. A static table, so a permission check can name the resource without
    /// building an `ApiResource`.
    pub(crate) fn resource(self) -> (&'static str, &'static str) {
        const RBAC: &str = "rbac.authorization.k8s.io";
        match self {
            Self::Pod => ("", "pods"),
            Self::Node => ("", "nodes"),
            Self::Namespace => ("", "namespaces"),
            Self::Event => ("", "events"),
            Self::Deployment => ("apps", "deployments"),
            Self::StatefulSet => ("apps", "statefulsets"),
            Self::DaemonSet => ("apps", "daemonsets"),
            Self::ReplicaSet => ("apps", "replicasets"),
            Self::Job => ("batch", "jobs"),
            Self::CronJob => ("batch", "cronjobs"),
            Self::Service => ("", "services"),
            Self::Ingress => ("networking.k8s.io", "ingresses"),
            Self::ConfigMap => ("", "configmaps"),
            Self::NetworkPolicy => ("networking.k8s.io", "networkpolicies"),
            Self::HorizontalPodAutoscaler => ("autoscaling", "horizontalpodautoscalers"),
            Self::ResourceQuota => ("", "resourcequotas"),
            Self::PodDisruptionBudget => ("policy", "poddisruptionbudgets"),
            Self::PersistentVolumeClaim => ("", "persistentvolumeclaims"),
            Self::PersistentVolume => ("", "persistentvolumes"),
            Self::StorageClass => ("storage.k8s.io", "storageclasses"),
            Self::ServiceAccount => ("", "serviceaccounts"),
            Self::Secret => ("", "secrets"),
            Self::Role => (RBAC, "roles"),
            Self::ClusterRole => (RBAC, "clusterroles"),
            Self::RoleBinding => (RBAC, "rolebindings"),
            Self::ClusterRoleBinding => (RBAC, "clusterrolebindings"),
            Self::CustomResourceDefinition => ("apiextensions.k8s.io", "customresourcedefinitions"),
        }
    }

    /// Whether Edit YAML is offered for the kind (0031 decision 25): the kinds the wireframes give
    /// an edit item. Custom resources have no `ObjectKind`, and Helm releases read as `Secret`
    /// objects but are never edited through this path.
    pub fn is_editable(self) -> bool {
        matches!(
            self,
            Self::Pod
                | Self::Deployment
                | Self::StatefulSet
                | Self::DaemonSet
                | Self::CronJob
                | Self::Service
                | Self::Ingress
                | Self::NetworkPolicy
                | Self::ConfigMap
                | Self::HorizontalPodAutoscaler
                | Self::ResourceQuota
                | Self::PodDisruptionBudget
                | Self::Secret
                | Self::Role
                | Self::ClusterRole
                | Self::RoleBinding
                | Self::ClusterRoleBinding
        )
    }

    /// The Kubernetes `kind`, for example `Deployment`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Pod => "Pod",
            Self::Node => "Node",
            Self::Namespace => "Namespace",
            Self::Event => "Event",
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
            Self::ReplicaSet => "ReplicaSet",
            Self::Job => "Job",
            Self::CronJob => "CronJob",
            Self::Service => "Service",
            Self::Ingress => "Ingress",
            Self::ConfigMap => "ConfigMap",
            Self::NetworkPolicy => "NetworkPolicy",
            Self::HorizontalPodAutoscaler => "HorizontalPodAutoscaler",
            Self::ResourceQuota => "ResourceQuota",
            Self::PodDisruptionBudget => "PodDisruptionBudget",
            Self::PersistentVolumeClaim => "PersistentVolumeClaim",
            Self::PersistentVolume => "PersistentVolume",
            Self::StorageClass => "StorageClass",
            Self::ServiceAccount => "ServiceAccount",
            Self::Secret => "Secret",
            Self::Role => "Role",
            Self::ClusterRole => "ClusterRole",
            Self::RoleBinding => "RoleBinding",
            Self::ClusterRoleBinding => "ClusterRoleBinding",
            Self::CustomResourceDefinition => "CustomResourceDefinition",
        }
    }

    pub fn is_namespaced(self) -> bool {
        !matches!(
            self,
            Self::Node
                | Self::Namespace
                | Self::PersistentVolume
                | Self::StorageClass
                | Self::ClusterRole
                | Self::ClusterRoleBinding
                | Self::CustomResourceDefinition
        )
    }
}

/// What an `ObjectRef` points at: a built-in kind, or one served version of a custom resource.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ObjectTarget {
    Builtin(ObjectKind),
    Custom(CustomResourceType),
}

impl ObjectTarget {
    fn is_namespaced(&self) -> bool {
        match self {
            Self::Builtin(kind) => kind.is_namespaced(),
            Self::Custom(resource) => resource.scope == ResourceScope::Namespaced,
        }
    }
}

/// One object. Always valid: it has a namespace exactly when its kind is namespaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    target: ObjectTarget,
    namespace: Option<String>,
    name: String,
}

impl ObjectRef {
    /// `None` when the namespace does not fit the kind's scope.
    pub fn new(kind: ObjectKind, namespace: Option<String>, name: String) -> Option<Self> {
        Self::of_target(ObjectTarget::Builtin(kind), namespace, name)
    }

    /// An object of a custom resource. `None` when the namespace does not fit the scope.
    pub fn custom(
        resource: CustomResourceType,
        namespace: Option<String>,
        name: String,
    ) -> Option<Self> {
        Self::of_target(ObjectTarget::Custom(resource), namespace, name)
    }

    /// The built-in kind, `None` for a custom resource.
    pub(crate) fn builtin_kind(&self) -> Option<ObjectKind> {
        match &self.target {
            ObjectTarget::Builtin(kind) => Some(*kind),
            ObjectTarget::Custom(_) => None,
        }
    }

    /// The Kubernetes `kind` of the object, for example `Node`.
    pub fn kind_name(&self) -> &str {
        match &self.target {
            ObjectTarget::Builtin(kind) => kind.name(),
            ObjectTarget::Custom(resource) => &resource.kind,
        }
    }

    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The resource, namespace, and name when this points at a custom object.
    pub(crate) fn as_custom(&self) -> Option<(&CustomResourceType, Option<&str>, &str)> {
        let ObjectTarget::Custom(resource) = &self.target else {
            return None;
        };
        Some((resource, self.namespace.as_deref(), &self.name))
    }

    fn of_target(target: ObjectTarget, namespace: Option<String>, name: String) -> Option<Self> {
        (namespace.is_some() == target.is_namespaced()).then_some(Self {
            target,
            namespace,
            name,
        })
    }
}

/// Whether env literals are masked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnvValues {
    #[default]
    Hidden,
    Shown,
}

/// Masked YAML of one object. No `Debug`: the text must never reach a log.
pub struct ObjectYaml {
    pub text: String,
    /// Env literals replaced by `<hidden>` (0 when `EnvValues::Shown`).
    pub hidden_env_values: usize,
}

impl ClusterConnection {
    /// The API handle that addresses `object`: its own namespace, or the whole cluster.
    pub(crate) fn object_api(&self, object: &ObjectRef) -> Api<DynamicObject> {
        let resource = match &object.target {
            ObjectTarget::Builtin(kind) => api_resource(*kind),
            ObjectTarget::Custom(resource) => custom_api_resource(resource),
        };
        let client = self.client().clone();
        match &object.namespace {
            Some(namespace) => Api::namespaced_with(client, namespace, &resource),
            None => Api::all_with(client, &resource),
        }
    }

    /// One GET of `object`, masked and serialized on the calling task.
    pub async fn object_yaml(
        &self,
        object: &ObjectRef,
        env: EnvValues,
    ) -> Result<ObjectYaml, ClusterError> {
        let value = self.get_object(object, ACTION).await?;
        let masked = match object.target {
            ObjectTarget::Builtin(_) => to_masked_yaml(value, env),
            ObjectTarget::Custom(_) => to_masked_custom_yaml(value, env),
        };
        masked.map_err(|message| self.unexpected_response(ACTION, message))
    }

    /// One GET of `object` as raw JSON. The value can hold secrets: callers mask it before it
    /// leaves the crate and never log it.
    pub(crate) async fn get_object(
        &self,
        object: &ObjectRef,
        action: &'static str,
    ) -> Result<Value, ClusterError> {
        let api = self.object_api(object);
        let found = self.run(action, api.get(&object.name)).await?;
        serde_json::to_value(&found)
            .map_err(|_| self.unexpected_response(action, CONVERSION_FAILURE))
    }

    /// An answer that could not be used. `message` is fixed text: a library error could quote the
    /// object's content.
    pub(crate) fn unexpected_response(
        &self,
        action: &'static str,
        message: &'static str,
    ) -> ClusterError {
        ClusterError::UnexpectedResponse {
            context: self.context().to_owned(),
            action,
            source: message.into(),
        }
    }
}

fn api_resource(kind: ObjectKind) -> ApiResource {
    match kind {
        ObjectKind::Pod => ApiResource::erase::<Pod>(&()),
        ObjectKind::Node => ApiResource::erase::<Node>(&()),
        ObjectKind::Namespace => ApiResource::erase::<Namespace>(&()),
        ObjectKind::Event => ApiResource::erase::<Event>(&()),
        ObjectKind::Deployment => ApiResource::erase::<Deployment>(&()),
        ObjectKind::StatefulSet => ApiResource::erase::<StatefulSet>(&()),
        ObjectKind::DaemonSet => ApiResource::erase::<DaemonSet>(&()),
        ObjectKind::ReplicaSet => ApiResource::erase::<ReplicaSet>(&()),
        ObjectKind::Job => ApiResource::erase::<Job>(&()),
        ObjectKind::CronJob => ApiResource::erase::<CronJob>(&()),
        ObjectKind::Service => ApiResource::erase::<Service>(&()),
        ObjectKind::Ingress => ApiResource::erase::<Ingress>(&()),
        ObjectKind::ConfigMap => ApiResource::erase::<ConfigMap>(&()),
        ObjectKind::NetworkPolicy => ApiResource::erase::<NetworkPolicy>(&()),
        ObjectKind::HorizontalPodAutoscaler => ApiResource::erase::<HorizontalPodAutoscaler>(&()),
        ObjectKind::ResourceQuota => ApiResource::erase::<ResourceQuota>(&()),
        ObjectKind::PodDisruptionBudget => ApiResource::erase::<PodDisruptionBudget>(&()),
        ObjectKind::PersistentVolumeClaim => ApiResource::erase::<PersistentVolumeClaim>(&()),
        ObjectKind::PersistentVolume => ApiResource::erase::<PersistentVolume>(&()),
        ObjectKind::StorageClass => ApiResource::erase::<StorageClass>(&()),
        ObjectKind::ServiceAccount => ApiResource::erase::<ServiceAccount>(&()),
        ObjectKind::Secret => ApiResource::erase::<Secret>(&()),
        ObjectKind::Role => ApiResource::erase::<Role>(&()),
        ObjectKind::ClusterRole => ApiResource::erase::<ClusterRole>(&()),
        ObjectKind::RoleBinding => ApiResource::erase::<RoleBinding>(&()),
        ObjectKind::ClusterRoleBinding => ApiResource::erase::<ClusterRoleBinding>(&()),
        ObjectKind::CustomResourceDefinition => ApiResource::erase::<CustomResourceDefinition>(&()),
    }
}

/// Masks, sorts keys like kubectl, and serializes. The error is a fixed message.
fn to_masked_yaml(object: Value, env: EnvValues) -> Result<ObjectYaml, &'static str> {
    mask_to_yaml(object, env, |_| 0)
}

/// Like `to_masked_yaml`, plus the custom object rules S2-S4 for objects of unknown kinds.
fn to_masked_custom_yaml(object: Value, env: EnvValues) -> Result<ObjectYaml, &'static str> {
    mask_to_yaml(object, env, |object| {
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        mask_custom_object(object, &kind)
    })
}

/// What `mask_object` hid.
pub(crate) struct MaskCount {
    pub(crate) hidden: usize,
    /// Env literals among `hidden` (0 when `EnvValues::Shown`).
    pub(crate) hidden_env_values: usize,
}

/// Strips `managedFields`, applies every mask rule, and sorts the keys like kubectl. `extra` runs
/// after the built-in rules and before the keys are sorted; it returns its count (custom objects).
pub(crate) fn mask_object(
    object: &mut Value,
    env: EnvValues,
    extra: impl FnOnce(&mut Value) -> usize,
) -> MaskCount {
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove("managedFields");
    }
    let mut hidden = mask_manifest_annotations(object, false)
        + mask_secret_data(object)
        + mask_storage_class_parameters(object)
        + mask_credential_mount_options(object);
    let mut hidden_env_values = 0;
    if env == EnvValues::Hidden
        && let Some(spec) = object.get_mut("spec")
    {
        hidden_env_values = mask_env_values(spec);
    }
    hidden += hidden_env_values;
    hidden += extra(object);
    object.sort_all_objects();
    MaskCount {
        hidden,
        hidden_env_values,
    }
}

/// The 0007 path: `mask_object`, serialized, with the hidden-count header.
fn mask_to_yaml(
    mut object: Value,
    env: EnvValues,
    extra: impl FnOnce(&mut Value) -> usize,
) -> Result<ObjectYaml, &'static str> {
    let count = mask_object(&mut object, env, extra);
    let body = to_yaml_text(&object, None)?;
    Ok(ObjectYaml {
        text: with_hidden_header(body, count.hidden),
        hidden_env_values: count.hidden_env_values,
    })
}

/// `yaml_text` plus an optional comment header (the edit header); `None` is no header. The
/// error is a fixed message.
pub(crate) fn to_yaml_text(object: &Value, header: Option<&str>) -> Result<String, &'static str> {
    let body = yaml_text(object)?;
    Ok(match header {
        Some(header) => format!("{header}\n{body}"),
        None => body,
    })
}

/// Serializes like kubectl. The error is a fixed message.
pub(crate) fn yaml_text(value: &Value) -> Result<String, &'static str> {
    // The options type is non-exhaustive, so it cannot be built with struct update syntax.
    let mut options = SerializerOptions::default();
    // Long single-line strings stay on one line, like kubectl.
    options.folded_wrap_chars = usize::MAX;
    serde_saphyr::to_string_with_options(value, options).map_err(|_| CONVERSION_FAILURE)
}

/// Prefixes the comment that says how many values were hidden; no prefix for zero.
pub(crate) fn with_hidden_header(body: String, hidden: usize) -> String {
    match hidden {
        0 => body,
        1 => format!("# k8sBoard hid 1 value as {HIDDEN}.\n{body}"),
        count => format!("# k8sBoard hid {count} values as {HIDDEN}.\n{body}"),
    }
}

/// Hides `MASKED_ANNOTATIONS` in every `metadata.annotations` map at any depth.
pub(crate) fn mask_manifest_annotations(value: &mut Value, is_metadata: bool) -> usize {
    match value {
        Value::Object(map) => {
            let mut hidden = 0;
            for (key, child) in map.iter_mut() {
                if is_metadata && key == "annotations" {
                    hidden += hide_annotations(child);
                }
                hidden += mask_manifest_annotations(child, key == "metadata");
            }
            hidden
        }
        Value::Array(items) => items
            .iter_mut()
            .map(|item| mask_manifest_annotations(item, false))
            .sum(),
        _ => 0,
    }
}

fn hide_annotations(annotations: &mut Value) -> usize {
    let Some(map) = annotations.as_object_mut() else {
        return 0;
    };
    let mut hidden = 0;
    for key in MASKED_ANNOTATIONS {
        if let Some(annotation) = map.get_mut(key) {
            *annotation = Value::from(HIDDEN);
            hidden += 1;
        }
    }
    hidden
}

/// Hides the values of a StorageClass's secret-like `parameters`, keyed on the response's `kind`.
fn mask_storage_class_parameters(object: &mut Value) -> usize {
    if object.get("kind").and_then(Value::as_str) != Some("StorageClass") {
        return 0;
    }
    let Some(parameters) = object.get_mut("parameters").and_then(Value::as_object_mut) else {
        return 0;
    };
    let mut hidden = 0;
    for (key, value) in parameters {
        if is_secret_key(key) {
            *value = Value::from(HIDDEN);
            hidden += 1;
        }
    }
    hidden
}

/// Hides credential values of mount options (CIFS `password=`): `mountOptions` of a StorageClass,
/// `spec.mountOptions` of a PersistentVolume, keyed on the response's `kind`.
fn mask_credential_mount_options(object: &mut Value) -> usize {
    let options = match object.get("kind").and_then(Value::as_str) {
        Some("StorageClass") => object.get_mut("mountOptions"),
        Some("PersistentVolume") => object
            .get_mut("spec")
            .and_then(|spec| spec.get_mut("mountOptions")),
        _ => None,
    };
    let Some(options) = options.and_then(Value::as_array_mut) else {
        return 0;
    };
    let mut hidden = 0;
    for option in options {
        let Some(text) = option.as_str() else {
            continue;
        };
        let masked = mask_mount_option(text);
        if masked != text {
            *option = Value::from(masked);
            hidden += 1;
        }
    }
    hidden
}

/// Hides every `data` and `stringData` value of a Secret, keyed on the response's `kind`.
pub(crate) fn mask_secret_data(object: &mut Value) -> usize {
    if object.get("kind").and_then(Value::as_str) != Some("Secret") {
        return 0;
    }
    let mut hidden = 0;
    for field in ["data", "stringData"] {
        let Some(map) = object.get_mut(field).and_then(Value::as_object_mut) else {
            continue;
        };
        for secret in map.values_mut() {
            *secret = Value::from(HIDDEN);
            hidden += 1;
        }
    }
    hidden
}

/// Hides `env[].value` literals of every container list under `spec`, at any depth, so pod,
/// workload, and job templates are all covered. `valueFrom` references stay.
pub(crate) fn mask_env_values(value: &mut Value) -> usize {
    match value {
        Value::Object(map) => {
            let mut hidden = 0;
            for (key, child) in map.iter_mut() {
                if CONTAINER_LISTS.contains(&key.as_str())
                    && let Some(containers) = child.as_array_mut()
                {
                    hidden += containers.iter_mut().map(hide_container_env).sum::<usize>();
                }
                hidden += mask_env_values(child);
            }
            hidden
        }
        Value::Array(items) => items.iter_mut().map(mask_env_values).sum(),
        _ => 0,
    }
}

fn hide_container_env(container: &mut Value) -> usize {
    let Some(env) = container.get_mut("env").and_then(Value::as_array_mut) else {
        return 0;
    };
    let mut hidden = 0;
    for entry in env {
        let Some(literal) = entry.get_mut("value").filter(|literal| literal.is_string()) else {
            continue;
        };
        *literal = Value::from(HIDDEN);
        hidden += 1;
    }
    hidden
}

/// Reference suffixes: a key that only names another object is never a credential
/// (`secretRef`, `passwordSecretRef`, `*-secret-name`).
const REFERENCE_SUFFIXES: [&str; 4] = ["secretname", "secretnamespace", "ref", "refs"];
/// Key fragments that mark a plaintext credential, after normalizing the key.
const SECRET_FRAGMENTS: [&str; 14] = [
    "secret",
    "password",
    "passwd",
    "token",
    "credential",
    "accesskey",
    "userkey",
    "privatekey",
    "apikey",
    "passphrase",
    "bearer",
    "clientkey",
    "kubeconfig",
    "connectionstring",
];

/// Whether a key looks like it holds a credential. A name heuristic: the key is lowercased and
/// stripped of `-`, `_`, `.`, and `/`, references are never secret, and any credential fragment
/// makes it secret.
pub(crate) fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|ch| !matches!(ch, '-' | '_' | '.' | '/'))
        .flat_map(char::to_lowercase)
        .collect();
    if REFERENCE_SUFFIXES
        .iter()
        .any(|suffix| normalized.ends_with(suffix))
    {
        return false;
    }
    SECRET_FRAGMENTS
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

/// A kind whose objects keep plain values under `data` or `spec`, like `ClusterSecret`.
pub(crate) fn is_secret_kind(kind: &str) -> bool {
    kind.to_ascii_lowercase().contains("secret") || is_secret_key(kind)
}

/// The custom object rules S2-S4 on one object (or on a `{spec, status}` wrapper): hides what
/// they select and returns how many values were hidden. `metadata`, `apiVersion`, and `kind`
/// are never touched. A scalar inside an array counts under the array's key.
pub(crate) fn mask_custom_object(object: &mut Value, kind: &str) -> usize {
    let Some(root) = object.as_object_mut() else {
        return 0;
    };
    let is_secret_kind = is_secret_kind(kind);
    let mut hidden = 0;
    for (key, value) in root.iter_mut() {
        if matches!(key.as_str(), "metadata" | "apiVersion" | "kind") {
            continue;
        }
        // The status of a secret-like kind stays readable; S3 and S4 still apply inside it.
        let hides_all = is_secret_kind && key != "status";
        hidden += mask_custom_value(key, value, hides_all);
    }
    hidden
}

/// Env-style `{name, value}` pairs outside container lists (`env`, `extraEnv`, `params`): the
/// value of a pair whose `name` is secret-like is hidden, whatever the pair is called.
fn hide_named_value(map: &mut serde_json::Map<String, Value>) -> usize {
    let has_secret_name = map
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(is_secret_key);
    if !has_secret_name {
        return 0;
    }
    match map.get_mut("value") {
        Some(value @ (Value::String(_) | Value::Number(_))) if *value != HIDDEN => {
            *value = Value::from(HIDDEN);
            1
        }
        _ => 0,
    }
}

fn mask_custom_value(key: &str, value: &mut Value, hides_all: bool) -> usize {
    match value {
        Value::Object(map) => {
            let hidden_pair = hide_named_value(map);
            hidden_pair
                + map
                    .iter_mut()
                    .map(|(key, child)| mask_custom_value(key, child, hides_all))
                    .sum::<usize>()
        }
        Value::Array(items) => items
            .iter_mut()
            .map(|item| mask_custom_value(key, item, hides_all))
            .sum(),
        Value::String(text) if text == HIDDEN => 0,
        Value::String(text) => {
            if hides_all || is_secret_key(key) {
                *value = Value::from(HIDDEN);
                return 1;
            }
            match mask_url_userinfo(text) {
                Some(masked) => {
                    *value = Value::from(masked);
                    1
                }
                None => 0,
            }
        }
        Value::Number(_) if hides_all || is_secret_key(key) => {
            *value = Value::from(HIDDEN);
            1
        }
        _ => 0,
    }
}

/// `scheme://userinfo@host` with `userinfo` replaced by `<hidden>`, for every URL in `text`.
/// The authority ends at the first `/`, `?`, `#`, or whitespace after `://`, and only an `@`
/// inside it counts (the last one). `None` when nothing changed.
pub(crate) fn mask_url_userinfo(text: &str) -> Option<String> {
    if !text.contains("://") {
        return None;
    }
    let mut masked = String::with_capacity(text.len());
    let mut is_changed = false;
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        let (head, tail) = rest.split_at(index + "://".len());
        masked.push_str(head);
        let end = tail
            .find(|ch: char| matches!(ch, '/' | '?' | '#') || ch.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) if &authority[..at] != HIDDEN => {
                masked.push_str(HIDDEN);
                masked.push_str(&authority[at..]);
                is_changed = true;
            }
            _ => masked.push_str(authority),
        }
        rest = &tail[end..];
    }
    masked.push_str(rest);
    is_changed.then_some(masked)
}

#[cfg(test)]
#[path = "object_yaml_tests.rs"]
mod object_yaml_tests;
