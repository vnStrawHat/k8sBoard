//! One-shot, masked YAML of a single object, serialized like `kubectl get -o yaml`.
//!
//! The text can embed secrets, so nothing in this module logs, prints, or keeps it: raw
//! objects are masked before they are serialized, and only the masked text leaves the crate.

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{ConfigMap, Event, Namespace, Node, Pod, Service};
use k8s_openapi::api::networking::v1::Ingress;
use kube::Api;
use kube::api::{ApiResource, DynamicObject};
use serde_json::Value;
use serde_saphyr::SerializerOptions;

use crate::connection::{ClusterConnection, ClusterError};

const ACTION: &str = "reading the object YAML";
/// Fixed on purpose: the library error could quote the object's content.
const CONVERSION_FAILURE: &str = "the object could not be converted to YAML";
const HIDDEN: &str = "<hidden>";

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
}

impl ObjectKind {
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
        }
    }

    pub fn is_namespaced(self) -> bool {
        !matches!(self, Self::Node | Self::Namespace)
    }
}

/// One object. Always valid: it has a namespace exactly when its kind is namespaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    kind: ObjectKind,
    namespace: Option<String>,
    name: String,
}

impl ObjectRef {
    /// `None` when the namespace does not fit the kind's scope.
    pub fn new(kind: ObjectKind, namespace: Option<String>, name: String) -> Option<Self> {
        (namespace.is_some() == kind.is_namespaced()).then_some(Self {
            kind,
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
    /// One GET of `object`, masked and serialized on the calling task.
    pub async fn object_yaml(
        &self,
        object: &ObjectRef,
        env: EnvValues,
    ) -> Result<ObjectYaml, ClusterError> {
        let resource = api_resource(object.kind);
        let client = self.client().clone();
        let api: Api<DynamicObject> = match &object.namespace {
            Some(namespace) => Api::namespaced_with(client, namespace, &resource),
            None => Api::all_with(client, &resource),
        };
        let found = self.run(ACTION, api.get(&object.name)).await?;
        let unexpected = |message: &'static str| ClusterError::UnexpectedResponse {
            context: self.context().to_owned(),
            action: ACTION,
            source: message.into(),
        };
        let value = serde_json::to_value(&found).map_err(|_| unexpected(CONVERSION_FAILURE))?;
        to_masked_yaml(value, env).map_err(unexpected)
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
    }
}

/// Masks, sorts keys like kubectl, and serializes. The error is a fixed message.
fn to_masked_yaml(mut object: Value, env: EnvValues) -> Result<ObjectYaml, &'static str> {
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove("managedFields");
    }
    let mut hidden = mask_manifest_annotations(&mut object, false) + mask_secret_data(&mut object);
    let mut hidden_env_values = 0;
    if env == EnvValues::Hidden
        && let Some(spec) = object.get_mut("spec")
    {
        hidden_env_values = mask_env_values(spec);
    }
    hidden += hidden_env_values;
    object.sort_all_objects();

    // The options type is non-exhaustive, so it cannot be built with struct update syntax.
    let mut options = SerializerOptions::default();
    // Long single-line strings stay on one line, like kubectl.
    options.folded_wrap_chars = usize::MAX;
    let body =
        serde_saphyr::to_string_with_options(&object, options).map_err(|_| CONVERSION_FAILURE)?;
    let text = match hidden {
        0 => body,
        1 => format!("# k8sBoard hid 1 value as {HIDDEN}.\n{body}"),
        count => format!("# k8sBoard hid {count} values as {HIDDEN}.\n{body}"),
    };
    Ok(ObjectYaml {
        text,
        hidden_env_values,
    })
}

/// Hides `MASKED_ANNOTATIONS` in every `metadata.annotations` map at any depth.
fn mask_manifest_annotations(value: &mut Value, is_metadata: bool) -> usize {
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

/// Hides every `data` and `stringData` value of a Secret, keyed on the response's `kind`.
fn mask_secret_data(object: &mut Value) -> usize {
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
fn mask_env_values(value: &mut Value) -> usize {
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

#[cfg(test)]
#[path = "object_yaml_tests.rs"]
mod object_yaml_tests;
