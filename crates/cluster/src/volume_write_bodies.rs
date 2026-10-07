//! The pure bodies of the volume writes (spec 0032b, UX round 3): a PersistentVolumeClaim created
//! again with another class, and a PersistentVolume's reclaim policy. `object_write.rs` reads the
//! object and sends; nothing here does I/O, and a refusal carries no object text.

use std::fmt;

use serde_json::{Map, Value, json};

/// What happens to a PersistentVolume's storage asset when its claim is deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReclaimPolicy {
    /// The volume and its data stay, `Released`, until an administrator reclaims them.
    Retain,
    /// The volume and the storage asset behind it are deleted with the claim.
    Delete,
}

impl ReclaimPolicy {
    /// The API text, also the audit value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retain => "Retain",
            Self::Delete => "Delete",
        }
    }

    /// `None` for `Recycle` (deprecated) and anything else the app does not set.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "Retain" => Some(Self::Retain),
            "Delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

impl fmt::Display for ReclaimPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The merge patch of a reclaim policy.
pub(crate) fn reclaim_policy_patch(policy: ReclaimPolicy) -> Value {
    json!({ "spec": { "persistentVolumeReclaimPolicy": policy.as_str() } })
}

/// Why a claim is not recreated, before anything is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecreateRefusal {
    /// The claim is not the one with the pinned uid: it was replaced since it was read.
    Replaced,
    /// A volume is bound, or being bound: deleting the claim would touch data.
    Bound,
    /// `metadata.deletionTimestamp` is set.
    Terminating,
    /// The object lacks the fields a claim has.
    Unreadable,
}

/// The claim to create after deleting `claim` (the fresh object): the same name, namespace,
/// labels, size, access modes, volume mode, and selector, with `class` instead of the old class.
/// Annotations are left out: the provisioner and the scheduler write their own, and a copy would
/// claim a node or a provisioner the new class does not have.
pub(crate) fn recreated_claim_body(
    claim: &Value,
    uid: &str,
    class: &str,
) -> Result<Value, RecreateRefusal> {
    let text = |pointer: &str| claim.pointer(pointer).and_then(Value::as_str);
    if text("/metadata/uid") != Some(uid) {
        return Err(RecreateRefusal::Replaced);
    }
    if claim.pointer("/metadata/deletionTimestamp").is_some() {
        return Err(RecreateRefusal::Terminating);
    }
    let is_bound = text("/status/phase") == Some("Bound")
        || text("/spec/volumeName").is_some_and(|volume| !volume.is_empty());
    if is_bound {
        return Err(RecreateRefusal::Bound);
    }
    let (Some(name), Some(namespace)) = (text("/metadata/name"), text("/metadata/namespace"))
    else {
        return Err(RecreateRefusal::Unreadable);
    };
    let old = claim.get("spec").ok_or(RecreateRefusal::Unreadable)?;
    let mut spec = Map::new();
    for key in ["accessModes", "resources", "volumeMode", "selector"] {
        if let Some(value) = old.get(key) {
            spec.insert(key.to_owned(), value.clone());
        }
    }
    spec.insert("storageClassName".to_owned(), Value::from(class));
    let mut metadata = json!({ "name": name, "namespace": namespace });
    if let Some(labels) = claim.pointer("/metadata/labels") {
        metadata["labels"] = labels.clone();
    }
    Ok(json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": metadata,
        "spec": spec,
    }))
}

#[cfg(test)]
#[path = "volume_write_bodies_tests.rs"]
mod volume_write_bodies_tests;
