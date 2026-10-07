//! Pieces shared by the workload, network, and config summaries.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{Container, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use crate::container_spec::{ContainerResource, container_resources};
use crate::edit_placeholders::HIDDEN;
use crate::event::optional_message;
use crate::object_yaml::{MASKED_ANNOTATIONS, is_secret_key};
use crate::pod_status::non_negative;
use crate::selector::Selector;

const REVISION_ANNOTATION: &str = "deployment.kubernetes.io/revision";
/// The annotation `kubectl rollout history` prints as the CHANGE-CAUSE of a revision; the
/// Deployment's copy is what its next ReplicaSet inherits.
pub(crate) const CHANGE_CAUSE_ANNOTATION: &str = "kubernetes.io/change-cause";

/// The owner reference with `controller == true`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerRef {
    pub kind: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkloadCondition {
    /// The condition type, for example `Available`.
    pub name: String,
    /// `status == "True"`; `False` and `Unknown` are both `false`.
    pub is_true: bool,
    /// An empty reason is `None`.
    pub reason: Option<String>,
    /// `conditions[].message`, cut to 1 KiB; empty is `None`.
    pub message: Option<String>,
    /// `conditions[].lastTransitionTime`; `None` when the controller did not set it.
    pub last_transition: Option<jiff::Timestamp>,
}

/// A main container of a pod template. Only the name, image, ports, and resources are kept:
/// `env`, `envFrom`, `command`, `args`, and `volumeMounts` can hold plaintext secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateContainer {
    pub name: String,
    /// The image as written in the spec.
    pub image: String,
    pub ports: Vec<ContainerPort>,
    /// `cpu`, `memory`, `ephemeral-storage`, then the rest by name; the quota check of a scale reads it.
    pub resources: Vec<ContainerResource>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerPort {
    pub name: Option<String>,
    pub port: u16,
    /// Defaults to `TCP`.
    pub protocol: String,
    /// `hostPort`; a value outside `u16` is `None`.
    pub host_port: Option<u16>,
}

pub(crate) fn controller_ref(metadata: &ObjectMeta) -> Option<ControllerRef> {
    metadata
        .owner_references
        .iter()
        .flatten()
        .find(|owner| owner.controller == Some(true))
        .map(|owner| ControllerRef {
            kind: owner.kind.clone(),
            name: owner.name.clone(),
        })
}

/// `key=value` terms in key order.
pub(crate) fn label_terms(metadata: &ObjectMeta) -> Vec<String> {
    key_value_terms(metadata.labels.as_ref())
}

/// The annotations of an object as `key=value` terms, for display. `Debug` prints the count only: a
/// value can hold anything a tool wrote, and a summary is printed whole by `{:?}`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AnnotationTerms(Vec<String>);

impl AnnotationTerms {
    pub fn new(terms: Vec<String>) -> Self {
        Self(terms)
    }

    pub fn terms(&self) -> &[String] {
        &self.0
    }
}

impl std::fmt::Debug for AnnotationTerms {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "AnnotationTerms({})", self.0.len())
    }
}

/// How many annotations a summary keeps, and how long a value may be before it is cut: a summary is
/// kept for every object of a list, and a few tools write whole documents into annotations.
const MAX_ANNOTATIONS: usize = 50;
const MAX_ANNOTATION_CHARS: usize = 200;

/// The annotations as `key=value` terms in key order, for display. The annotations that embed an
/// applied manifest are left out, the value of a key that looks like a credential is hidden, and a
/// long value is cut at one line.
pub(crate) fn annotation_terms(metadata: &ObjectMeta) -> AnnotationTerms {
    let terms = metadata
        .annotations
        .iter()
        .flatten()
        .filter(|(key, _)| !MASKED_ANNOTATIONS.contains(&key.as_str()))
        .take(MAX_ANNOTATIONS)
        .map(|(key, value)| {
            if is_secret_key(key) {
                return format!("{key}={HIDDEN}");
            }
            let one_line = value.replace(['\n', '\r'], " ");
            match one_line.char_indices().nth(MAX_ANNOTATION_CHARS) {
                Some((end, _)) => format!("{key}={}…", &one_line[..end]),
                None => format!("{key}={one_line}"),
            }
        })
        .collect();
    AnnotationTerms(terms)
}

pub(crate) fn key_value_terms(pairs: Option<&BTreeMap<String, String>>) -> Vec<String> {
    pairs
        .into_iter()
        .flatten()
        .map(|(key, value)| format!("{key}={value}"))
        .collect()
}

/// kubectl's selector syntax: `matchLabels` first, then the expressions in order.
pub(crate) fn selector_terms(selector: &LabelSelector) -> Vec<String> {
    Selector::of(selector).terms()
}

/// Main containers in spec order; init containers are excluded.
pub(crate) fn template_containers(template: &PodTemplateSpec) -> Vec<TemplateContainer> {
    let Some(spec) = &template.spec else {
        return Vec::new();
    };
    spec.containers
        .iter()
        .map(|container| TemplateContainer {
            name: container.name.clone(),
            image: container.image.clone().unwrap_or_default(),
            ports: container_ports(container),
            resources: container_resources(container),
        })
        .collect()
}

/// The container's declared ports in spec order.
pub(crate) fn container_ports(container: &Container) -> Vec<ContainerPort> {
    container
        .ports
        .iter()
        .flatten()
        .filter_map(|port| {
            Some(ContainerPort {
                name: port.name.clone(),
                // The API rejects ports outside u16, so such a port is dropped.
                port: u16::try_from(port.container_port).ok()?,
                protocol: port.protocol.clone().unwrap_or_else(|| "TCP".to_owned()),
                host_port: port.host_port.and_then(|port| u16::try_from(port).ok()),
            })
        })
        .collect()
}

pub(crate) fn condition(
    name: &str,
    status: &str,
    reason: Option<&str>,
    message: Option<&str>,
    last_transition: Option<jiff::Timestamp>,
) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true: status == "True",
        reason: non_empty(reason),
        message: optional_message(message),
        last_transition,
    }
}

/// `25%` or `1`.
pub(crate) fn int_or_string_text(value: &IntOrString) -> String {
    match value {
        IntOrString::Int(number) => number.to_string(),
        IntOrString::String(text) => text.clone(),
    }
}

/// An absent count reads as zero.
pub(crate) fn optional_count(count: Option<i32>) -> u32 {
    non_negative(count.unwrap_or(0))
}

/// The Deployment revision, shared by Deployments and their ReplicaSets. Read here
/// once so no other annotation is ever kept.
pub(crate) fn revision(metadata: &ObjectMeta) -> Option<String> {
    metadata
        .annotations
        .as_ref()?
        .get(REVISION_ANNOTATION)
        .cloned()
}

/// The change cause of a Deployment or a ReplicaSet, when it has a non-empty one. Read here once, so
/// no other annotation is ever kept.
pub(crate) fn change_cause(metadata: &ObjectMeta) -> Option<String> {
    non_empty(
        metadata
            .annotations
            .as_ref()?
            .get(CHANGE_CAUSE_ANNOTATION)
            .map(String::as_str),
    )
}

pub(crate) fn non_empty(text: Option<&str>) -> Option<String> {
    text.filter(|text| !text.is_empty()).map(str::to_owned)
}

#[cfg(test)]
#[path = "workload_tests.rs"]
mod workload_tests;
