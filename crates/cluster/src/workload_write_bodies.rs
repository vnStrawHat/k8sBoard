//! The pure request bodies of the 0032 operations that read an object before they send: Roll back,
//! Trigger now, and Re-run. `object_write.rs` reads the objects and sends; nothing here does I/O.
//!
//! The objects can hold env literals, so nothing here logs, and a refusal carries no object text.

use serde_json::{Value, json};

use crate::workload::CHANGE_CAUSE_ANNOTATION;

const REVISION_ANNOTATION: &str = "deployment.kubernetes.io/revision";
const POD_TEMPLATE_HASH: &str = "pod-template-hash";
/// The labels and selectors of a Job that its controller owns; a copy must not carry them.
pub(crate) const JOB_CONTROLLER_LABELS: [&str; 4] = [
    "controller-uid",
    "job-name",
    "batch.kubernetes.io/controller-uid",
    "batch.kubernetes.io/job-name",
];
/// The Job name limit is 63: a base, the suffix, and the 5 characters the server adds.
pub(crate) const TRIGGER_BASE_CHARS: usize = 50;
pub(crate) const TRIGGER_SUFFIX: &str = "-manual-";
pub(crate) const RERUN_BASE_CHARS: usize = 51;
pub(crate) const RERUN_SUFFIX: &str = "-rerun-";

/// The `generateName` of a created Job: the first `base_chars` of `name` plus `suffix`.
pub(crate) fn generate_name(name: &str, base_chars: usize, suffix: &str) -> String {
    let base: String = name.chars().take(base_chars).collect();
    format!("{base}{suffix}")
}

/// Why a Roll back is refused before anything is sent.
pub(crate) enum RollBackRefusal {
    /// The Deployment or the ReplicaSet lacks a field the check needs.
    Unreadable,
    /// The ReplicaSet is not this Deployment's, or is not that revision.
    ForeignReplicaSet,
    SameTemplate,
}

/// The JSON Patch of a Roll back: a `test` of the Deployment's uid, then `replace /spec/template`
/// with the ReplicaSet's template (the `pod-template-hash` label removed, as kubectl does).
pub(crate) fn rollback_operations(
    deployment: &Value,
    replica_set: &Value,
    revision: u64,
) -> Result<Value, RollBackRefusal> {
    let uid = deployment
        .pointer("/metadata/uid")
        .and_then(Value::as_str)
        .ok_or(RollBackRefusal::Unreadable)?;
    let is_owned = replica_set
        .pointer("/metadata/ownerReferences")
        .and_then(Value::as_array)
        .is_some_and(|owners| {
            owners.iter().any(|owner| {
                owner.get("controller") == Some(&Value::Bool(true))
                    && owner.get("uid").and_then(Value::as_str) == Some(uid)
            })
        });
    let is_revision = replica_set
        .pointer("/metadata/annotations")
        .and_then(|annotations| annotations.get(REVISION_ANNOTATION))
        .and_then(Value::as_str)
        == Some(revision.to_string().as_str());
    if !is_owned || !is_revision {
        return Err(RollBackRefusal::ForeignReplicaSet);
    }
    let template = template_without_hash(replica_set).ok_or(RollBackRefusal::Unreadable)?;
    let current = template_without_hash(deployment).ok_or(RollBackRefusal::Unreadable)?;
    if template == current {
        return Err(RollBackRefusal::SameTemplate);
    }
    Ok(json!([
        { "op": "test", "path": "/metadata/uid", "value": uid },
        { "op": "replace", "path": "/spec/template", "value": template },
    ]))
}

/// The longest change cause Set image accepts: a cause is one line of a history list.
pub(crate) const MAX_CHANGE_CAUSE_CHARS: usize = 256;

/// The strategic merge patch of a Set image: the container merges on its `name`, and the change
/// cause is written (or, without one, removed: a cause left from an earlier change would be copied
/// to the new ReplicaSet and name the wrong change).
pub(crate) fn set_image_patch(container: &str, image: &str, change_cause: Option<&str>) -> Value {
    json!({
        "metadata": { "annotations": { CHANGE_CAUSE_ANNOTATION: change_cause } },
        "spec": { "template": { "spec": { "containers": [{ "name": container, "image": image }] } } },
    })
}

/// Whether `cause` is a one-line text of a length the history list can show.
pub(crate) fn is_valid_change_cause(cause: &str) -> bool {
    !cause.is_empty()
        && cause.chars().count() <= MAX_CHANGE_CAUSE_CHARS
        && !cause.chars().any(char::is_control)
}

/// The pod template of a Deployment or ReplicaSet without the controller's hash label.
fn template_without_hash(object: &Value) -> Option<Value> {
    let mut template = object.pointer("/spec/template")?.clone();
    if let Some(labels) = template
        .pointer_mut("/metadata/labels")
        .and_then(Value::as_object_mut)
    {
        labels.remove(POD_TEMPLATE_HASH);
    }
    Some(template)
}

/// A Job from a CronJob's template, like `kubectl create job --from=cronjob/NAME`. The owner
/// reference has `controller: true` and no `blockOwnerDeletion`: setting that needs `update` on
/// `cronjobs/finalizers`, which `create jobs` alone does not grant.
pub(crate) fn trigger_job_body(
    cron_job: &Value,
    namespace: Option<&str>,
    name: &str,
) -> Option<Value> {
    let uid = cron_job.pointer("/metadata/uid")?.as_str()?;
    let template = cron_job.pointer("/spec/jobTemplate")?;
    let spec = template.get("spec")?.clone();
    let mut annotations = template
        .pointer("/metadata/annotations")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    annotations.insert(
        "cronjob.kubernetes.io/instantiate".to_owned(),
        "manual".into(),
    );
    let mut metadata = json!({
        "generateName": generate_name(name, TRIGGER_BASE_CHARS, TRIGGER_SUFFIX),
        "namespace": namespace?,
        "annotations": annotations,
        "ownerReferences": [{
            "apiVersion": "batch/v1",
            "kind": "CronJob",
            "name": name,
            "uid": uid,
            "controller": true,
        }],
    });
    if let Some(labels) = template.pointer("/metadata/labels") {
        metadata["labels"] = labels.clone();
    }
    Some(json!({ "apiVersion": "batch/v1", "kind": "Job", "metadata": metadata, "spec": spec }))
}

/// A standalone copy of a Job: no owner, no annotations, no selector, no controller labels, and
/// not suspended.
pub(crate) fn rerun_job_body(job: &Value, namespace: Option<&str>, name: &str) -> Option<Value> {
    let mut spec = job.get("spec")?.clone();
    let fields = spec.as_object_mut()?;
    fields.remove("selector");
    fields.remove("manualSelector");
    fields.insert("suspend".to_owned(), Value::Bool(false));
    if let Some(labels) = spec
        .pointer_mut("/template/metadata/labels")
        .and_then(Value::as_object_mut)
    {
        remove_controller_labels(labels);
    }
    let mut metadata = json!({
        "generateName": generate_name(name, RERUN_BASE_CHARS, RERUN_SUFFIX),
        "namespace": namespace?,
    });
    if let Some(labels) = job.pointer("/metadata/labels").and_then(Value::as_object) {
        let mut labels = labels.clone();
        remove_controller_labels(&mut labels);
        metadata["labels"] = Value::Object(labels);
    }
    Some(json!({ "apiVersion": "batch/v1", "kind": "Job", "metadata": metadata, "spec": spec }))
}

fn remove_controller_labels(labels: &mut serde_json::Map<String, Value>) {
    for label in JOB_CONTROLLER_LABELS {
        labels.remove(label);
    }
}

#[cfg(test)]
#[path = "workload_write_bodies_tests.rs"]
mod workload_write_bodies_tests;
