//! The bodies and validity rules of the node maintenance writes (spec 0034): the eviction, the
//! taint patch, and the label patch. Pure: nothing here sends a request.
//!
//! A body holds the values of the edit, which are not secret and are audited (0034 decision 14).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::debug_pod_bodies::is_label_value;
use crate::dns_name::is_dns_subdomain;
use crate::node::NodeTaint;

const EFFECTS: [&str; 3] = ["NoSchedule", "PreferNoSchedule", "NoExecute"];
/// A uid is a UUID; the bound only keeps a garbage value out of the eviction body.
const MAX_UID_LENGTH: usize = 64;
/// The longest `name` part of a qualified name (the part after the optional prefix).
const MAX_NAME_LENGTH: usize = 63;

/// How long a pod gets to stop before the kubelet kills it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GracePeriod {
    /// The pod's own `terminationGracePeriodSeconds`.
    PodDefault,
    Seconds(u32),
}

/// One label key of a label edit: set to `value`, or removed when `value` is `None`.
// Debug is manual: the key only, never a value.
#[derive(Clone, PartialEq, Eq)]
pub struct LabelChange {
    pub key: String,
    pub value: Option<String>,
}

impl std::fmt::Debug for LabelChange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LabelChange")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// The `policy/v1` Eviction. The uid precondition is what makes a retry safe: a pod that was
/// recreated under the same name (a StatefulSet) is never evicted by mistake. The key is
/// `deleteOptions` (camel case), which kube's own `Api::evict` does not send.
pub(crate) fn eviction_body(namespace: &str, name: &str, uid: &str, grace: GracePeriod) -> Value {
    let mut options = json!({"preconditions": {"uid": uid}});
    if let GracePeriod::Seconds(seconds) = grace {
        options["gracePeriodSeconds"] = json!(seconds);
    }
    json!({
        "apiVersion": "policy/v1",
        "kind": "Eviction",
        "metadata": {"name": name, "namespace": namespace},
        "deleteOptions": options,
    })
}

/// `resourceVersion` makes the patch a 409 when the node changed since it was read: `spec.taints`
/// has no merge key, so the whole list is replaced and a concurrent change must not be lost.
/// `timeAdded` is sent back unchanged (whole seconds), because the node lifecycle controller
/// times its `NoExecute` evictions from it.
pub(crate) fn taints_patch(taints: &[NodeTaint], resource_version: &str) -> Value {
    let taints: Vec<Value> = taints
        .iter()
        .map(|taint| {
            let mut entry = Map::new();
            entry.insert("key".to_owned(), json!(taint.key));
            if let Some(value) = taint.value.as_deref().filter(|value| !value.is_empty()) {
                entry.insert("value".to_owned(), json!(value));
            }
            entry.insert("effect".to_owned(), json!(taint.effect));
            if let Some(added) = &taint.time_added {
                let stamp = added.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
                entry.insert("timeAdded".to_owned(), json!(stamp));
            }
            Value::Object(entry)
        })
        .collect();
    json!({"metadata": {"resourceVersion": resource_version}, "spec": {"taints": taints}})
}

/// What a taint edit changes, one line per taint (a taint is its key and effect): `conflict 4 → 3`
/// for a new value, `− workload=data:NoSchedule` for a removed taint, `+ maintenance:NoSchedule` for
/// an added one. Changed taints come first, in the new order, then removed, then added.
pub(crate) fn taint_changes(previous: &[NodeTaint], taints: &[NodeTaint]) -> Vec<String> {
    let find = |list: &'_ [NodeTaint], taint: &NodeTaint| {
        list.iter()
            .find(|other| other.key == taint.key && other.effect == taint.effect)
            .cloned()
    };
    let value_text = |taint: &NodeTaint| match taint.value.as_deref() {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => "(no value)".to_owned(),
    };
    let mut changed = Vec::new();
    let mut added = Vec::new();
    for taint in taints {
        match find(previous, taint) {
            Some(old) if old.value != taint.value => changed.push(format!(
                "{} {} → {}",
                taint.key,
                value_text(&old),
                value_text(taint)
            )),
            Some(_) => {}
            None => added.push(format!("+ {taint}")),
        }
    }
    let removed = previous
        .iter()
        .filter(|taint| find(taints, taint).is_none())
        .map(|taint| format!("− {taint}"));
    changed.into_iter().chain(removed).chain(added).collect()
}

/// A per-key merge patch: keys are independent, so no `resourceVersion`; `null` removes a key.
pub(crate) fn labels_patch(changes: &[LabelChange]) -> Value {
    let labels: Map<String, Value> = changes
        .iter()
        .map(|change| (change.key.clone(), json!(change.value)))
        .collect();
    json!({"metadata": {"labels": labels}})
}

/// A uid the eviction can carry: 1 to 64 characters of `[A-Za-z0-9-]`.
pub(crate) fn is_valid_uid(uid: &str) -> bool {
    (1..=MAX_UID_LENGTH).contains(&uid.len())
        && uid
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

/// Kubernetes' qualified name: an optional DNS-subdomain prefix and `/`, then 1 to 63 characters
/// of `[A-Za-z0-9-_.]` starting and ending alphanumeric.
pub(crate) fn is_qualified_name(key: &str) -> bool {
    let (prefix, name) = match key.split_once('/') {
        Some((prefix, name)) => (Some(prefix), name),
        None => (None, key),
    };
    prefix.is_none_or(is_dns_subdomain)
        && (1..=MAX_NAME_LENGTH).contains(&name.len())
        && is_label_value(name)
}

/// Whether every taint has a qualified key, a label-value value, one of the three effects, and
/// no other taint repeats its key and effect.
pub(crate) fn are_valid_taints(taints: &[NodeTaint]) -> bool {
    let mut seen = HashSet::new();
    taints.iter().all(|taint| {
        is_qualified_name(&taint.key)
            && taint.value.as_deref().is_none_or(is_label_value)
            && EFFECTS.contains(&taint.effect.as_str())
            && seen.insert((taint.key.as_str(), taint.effect.as_str()))
    })
}

/// Whether the edit is non-empty, every key is a qualified name, every set value is a label value,
/// and no key appears twice.
pub(crate) fn are_valid_label_changes(changes: &[LabelChange]) -> bool {
    let mut seen = HashSet::new();
    !changes.is_empty()
        && changes.iter().all(|change| {
            is_qualified_name(&change.key)
                && change.value.as_deref().is_none_or(is_label_value)
                && seen.insert(change.key.as_str())
        })
}

#[cfg(test)]
#[path = "node_maintenance_bodies_tests.rs"]
mod node_maintenance_bodies_tests;
