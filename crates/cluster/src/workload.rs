//! Pieces shared by the workload, network, and config summaries.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::PodTemplateSpec;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use crate::pod_status::non_negative;

const REVISION_ANNOTATION: &str = "deployment.kubernetes.io/revision";

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
}

/// A main container of a pod template. Only the name, image, and ports are kept:
/// `env`, `envFrom`, `command`, `args`, and `volumeMounts` can hold plaintext secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateContainer {
    pub name: String,
    /// The image as written in the spec.
    pub image: String,
    pub ports: Vec<ContainerPort>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerPort {
    pub name: Option<String>,
    pub port: u16,
    /// Defaults to `TCP`.
    pub protocol: String,
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

pub(crate) fn key_value_terms(pairs: Option<&BTreeMap<String, String>>) -> Vec<String> {
    pairs
        .into_iter()
        .flatten()
        .map(|(key, value)| format!("{key}={value}"))
        .collect()
}

/// kubectl's selector syntax: `matchLabels` first, then the expressions in order.
pub(crate) fn selector_terms(selector: &LabelSelector) -> Vec<String> {
    let mut terms = key_value_terms(selector.match_labels.as_ref());
    for expression in selector.match_expressions.iter().flatten() {
        let key = &expression.key;
        let values = expression
            .values
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let values = values.join(",");
        match expression.operator.as_str() {
            "In" => terms.push(format!("{key} in ({values})")),
            "NotIn" => terms.push(format!("{key} notin ({values})")),
            "Exists" => terms.push(key.clone()),
            "DoesNotExist" => terms.push(format!("!{key}")),
            _ => {}
        }
    }
    terms
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
            ports: container
                .ports
                .iter()
                .flatten()
                .filter_map(|port| {
                    Some(ContainerPort {
                        name: port.name.clone(),
                        // The API rejects ports outside u16, so such a port is dropped.
                        port: u16::try_from(port.container_port).ok()?,
                        protocol: port.protocol.clone().unwrap_or_else(|| "TCP".to_owned()),
                    })
                })
                .collect(),
        })
        .collect()
}

pub(crate) fn condition(name: &str, status: &str, reason: Option<&str>) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true: status == "True",
        reason: non_empty(reason),
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

pub(crate) fn non_empty(text: Option<&str>) -> Option<String> {
    text.filter(|text| !text.is_empty()).map(str::to_owned)
}

#[cfg(test)]
#[path = "workload_tests.rs"]
mod workload_tests;
