//! The pure body of a cert-manager Certificate "Renew now" (0018 step 6): `cmctl renew` adds the
//! condition `Issuing=True` to the Certificate's status, and cert-manager's trigger controller
//! then issues a new certificate. `object_write.rs` reads the object and sends; nothing here does
//! I/O, and a refusal carries no object text.

use serde_json::{Map, Value, json};

const ISSUING: &str = "Issuing";
const REASON: &str = "ManuallyTriggered";
const MESSAGE: &str = "Certificate re-issuance manually triggered";

/// Why a renewal is refused before anything is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenewRefusal {
    /// An `Issuing=True` condition exists: an issuance is already under way.
    AlreadyIssuing,
    /// `metadata.deletionTimestamp` is set.
    Deleting,
    /// The object lacks a `resourceVersion`, or its metadata, status, or conditions are not the
    /// shapes a Certificate has.
    Unreadable,
}

/// The `PUT .../status` body of a renewal: the fresh object without `managedFields` (the server
/// keeps them when absent), with any `Issuing` condition replaced by `Issuing=True`. Every other
/// field, condition, and the `resourceVersion` stay as read, so a change made meanwhile is a 409.
pub(crate) fn renewal_status_body(
    mut fresh: Value,
    requested_at: jiff::Timestamp,
) -> Result<Value, RenewRefusal> {
    let root = fresh.as_object_mut().ok_or(RenewRefusal::Unreadable)?;
    let metadata = root
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
        .ok_or(RenewRefusal::Unreadable)?;
    if metadata
        .get("deletionTimestamp")
        .is_some_and(|stamp| !stamp.is_null())
    {
        return Err(RenewRefusal::Deleting);
    }
    let has_version = metadata
        .get("resourceVersion")
        .and_then(Value::as_str)
        .is_some_and(|version| !version.is_empty());
    if !has_version {
        return Err(RenewRefusal::Unreadable);
    }
    metadata.remove("managedFields");
    let generation = metadata.get("generation").filter(|value| value.is_number());
    let generation = generation.cloned();

    let conditions = conditions_of(root)?;
    if conditions.iter().any(is_issuing_true) {
        return Err(RenewRefusal::AlreadyIssuing);
    }
    conditions.retain(|condition| condition.get("type").and_then(Value::as_str) != Some(ISSUING));
    let mut issuing = json!({
        "type": ISSUING,
        "status": "True",
        "reason": REASON,
        "message": MESSAGE,
        "lastTransitionTime": condition_time(&requested_at),
    });
    if let Some(generation) = generation {
        issuing["observedGeneration"] = generation;
    }
    conditions.push(issuing);
    Ok(fresh)
}

/// `status.conditions`, created (with `status`) when missing or null.
fn conditions_of(root: &mut Map<String, Value>) -> Result<&mut Vec<Value>, RenewRefusal> {
    let status = root.entry("status").or_insert(Value::Null);
    if status.is_null() {
        *status = json!({});
    }
    let conditions = status
        .as_object_mut()
        .ok_or(RenewRefusal::Unreadable)?
        .entry("conditions")
        .or_insert(Value::Null);
    if conditions.is_null() {
        *conditions = json!([]);
    }
    conditions.as_array_mut().ok_or(RenewRefusal::Unreadable)
}

fn is_issuing_true(condition: &Value) -> bool {
    condition.get("type").and_then(Value::as_str) == Some(ISSUING)
        && condition.get("status").and_then(Value::as_str) == Some("True")
}

/// RFC 3339 in UTC, whole seconds: a dry-run and its commit send the same text.
fn condition_time(timestamp: &jiff::Timestamp) -> String {
    timestamp.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
#[path = "certificate_renewal_tests.rs"]
mod certificate_renewal_tests;
