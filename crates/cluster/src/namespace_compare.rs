//! Read-only comparison of two namespaces (spec 0057): the objects of eleven kinds matched by name,
//! the ones in only one namespace, and the field changes of the ones in both.
//!
//! The text of an object can embed secrets, so nothing here logs it: Secret values are reduced to
//! equality tokens before anything is kept, and env literals are masked like the YAML tab.

use std::collections::BTreeMap;
use std::hash::{BuildHasher as _, RandomState};

use futures::future::join_all;
use kube::Api;
use kube::api::DynamicObject;
use serde_json::Value;

use crate::connection::{ClusterConnection, ClusterError};
use crate::edit_preview::{FieldChange, field_changes, field_paths};
use crate::helm_release::RELEASE_TYPE as HELM_RELEASE_SECRET_TYPE;
use crate::object_edit::clean_value;
use crate::object_yaml::{
    EnvValues, ObjectKind, api_resource, mask_manifest_annotations, mask_object, to_yaml_text,
};

const ACTION: &str = "listing a namespace to compare it";
/// Fixed on purpose: the library error could quote the object's content.
const CONVERSION_FAILURE: &str = "the object could not be converted to YAML";

/// The kinds that are compared, in the order the dialog shows them.
pub const COMPARE_KINDS: [ObjectKind; 11] = [
    ObjectKind::Deployment,
    ObjectKind::StatefulSet,
    ObjectKind::DaemonSet,
    ObjectKind::CronJob,
    ObjectKind::Service,
    ObjectKind::ConfigMap,
    ObjectKind::Secret,
    ObjectKind::Ingress,
    ObjectKind::PersistentVolumeClaim,
    ObjectKind::HorizontalPodAutoscaler,
    ObjectKind::ResourceQuota,
];

/// Annotations the cluster or a controller sets per namespace or per object; they differ without
/// meaning anything for "what is configured differently".
const VOLATILE_ANNOTATIONS: [&str; 7] = [
    "deployment.kubernetes.io/revision",
    "meta.helm.sh/release-namespace",
    "pv.kubernetes.io/bind-completed",
    "pv.kubernetes.io/bound-by-controller",
    "volume.kubernetes.io/selected-node",
    "volume.kubernetes.io/storage-provisioner",
    "volume.beta.kubernetes.io/storage-provisioner",
];

/// What the comparison found for the two namespaces.
pub struct NamespaceComparison {
    pub left: String,
    pub right: String,
    pub kinds: Vec<KindComparison>,
    /// Env literals hidden on either side; they may differ without showing.
    pub hidden_env_values: usize,
}

pub struct KindComparison {
    pub kind: ObjectKind,
    pub outcome: KindOutcome,
}

pub enum KindOutcome {
    /// One of the two lists failed; the text names why.
    Unreadable(String),
    Compared {
        only_left: Vec<String>,
        only_right: Vec<String>,
        differs: Vec<ObjectDifference>,
        /// Objects equal on both sides.
        same: usize,
    },
}

/// One object that exists in both namespaces with different content.
pub struct ObjectDifference {
    pub name: String,
    /// `left` as the old side and `right` as the new one.
    pub changes: Vec<FieldChange>,
    /// Changes beyond the cap of `changes`.
    pub more_changes: usize,
    pub left_text: String,
    pub right_text: String,
}

/// How many objects fall in each group, over every readable kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompareCounts {
    pub differ: usize,
    pub only_left: usize,
    pub only_right: usize,
    pub same: usize,
}

impl NamespaceComparison {
    pub fn counts(&self) -> CompareCounts {
        let mut counts = CompareCounts::default();
        for kind in &self.kinds {
            let KindOutcome::Compared {
                only_left,
                only_right,
                differs,
                same,
            } = &kind.outcome
            else {
                continue;
            };
            counts.differ += differs.len();
            counts.only_left += only_left.len();
            counts.only_right += only_right.len();
            counts.same += same;
        }
        counts
    }
}

/// The reduced objects of one kind in one namespace, by name.
type KindObjects = BTreeMap<String, Value>;

impl ClusterConnection {
    /// Lists the eleven kinds of both namespaces at once and compares them. A kind that cannot be
    /// listed is `Unreadable` for the pair; the call itself does not fail. Read-only.
    pub async fn compare_namespaces(
        &self,
        left: &str,
        right: &str,
        env: EnvValues,
    ) -> NamespaceComparison {
        // One salt for both sides, so equal Secret values give equal tokens.
        let salt = RandomState::new();
        let reads = COMPARE_KINDS
            .iter()
            .flat_map(|kind| [(*kind, left), (*kind, right)])
            .map(|(kind, namespace)| self.read_kind(kind, namespace, env, &salt));
        let mut reads = join_all(reads).await.into_iter();
        let mut hidden_env_values = 0;
        let mut kinds = Vec::with_capacity(COMPARE_KINDS.len());
        for kind in COMPARE_KINDS {
            let (Some(left_read), Some(right_read)) = (reads.next(), reads.next()) else {
                break;
            };
            let outcome = match (left_read, right_read) {
                (Ok(left_read), Ok(right_read)) => {
                    hidden_env_values += left_read.hidden_env_values + right_read.hidden_env_values;
                    compare_kind(left_read.objects, right_read.objects)
                }
                (Err(error), _) | (_, Err(error)) => KindOutcome::Unreadable(error.to_string()),
            };
            kinds.push(KindComparison { kind, outcome });
        }
        NamespaceComparison {
            left: left.to_owned(),
            right: right.to_owned(),
            kinds,
            hidden_env_values,
        }
    }

    async fn read_kind(
        &self,
        kind: ObjectKind,
        namespace: &str,
        env: EnvValues,
        salt: &RandomState,
    ) -> Result<KindRead, ClusterError> {
        let api: Api<DynamicObject> =
            Api::namespaced_with(self.client().clone(), namespace, &api_resource(kind));
        let items = self.list_all(api, ACTION).await?;
        let mut read = KindRead::default();
        for item in items {
            let value = serde_json::to_value(&item)
                .map_err(|_| self.unexpected_response(ACTION, CONVERSION_FAILURE))?;
            if let Some((name, object, hidden)) = reduce_object(kind, value, env, salt) {
                read.hidden_env_values += hidden;
                read.objects.insert(name, object);
            }
        }
        Ok(read)
    }
}

#[derive(Default)]
struct KindRead {
    objects: KindObjects,
    hidden_env_values: usize,
}

/// The name and the comparable form of one listed item, with the env literals it hid. `None` for an
/// item without a name and for a Helm release record, which is no object of the namespace.
fn reduce_object(
    kind: ObjectKind,
    mut object: Value,
    env: EnvValues,
    salt: &RandomState,
) -> Option<(String, Value, usize)> {
    let name = object.pointer("/metadata/name")?.as_str()?.to_owned();
    // List items carry no `kind`, which the masking keys on.
    object["kind"] = Value::from(kind.name());
    let hidden_env_values = if kind == ObjectKind::Secret {
        if object.get("type").and_then(Value::as_str) == Some(HELM_RELEASE_SECRET_TYPE) {
            return None;
        }
        if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
            metadata.remove("managedFields");
        }
        mask_manifest_annotations(&mut object, false);
        tokenize_secret_values(&mut object, salt);
        0
    } else {
        mask_object(&mut object, env, |_| 0).hidden_env_values
    };
    clean_value(&mut object);
    drop_namespace_noise(kind, &mut object);
    object.sort_all_objects();
    Some((name, object, hidden_env_values))
}

/// Replaces every `data` and `stringData` value of a Secret with a token that is equal exactly when
/// the values are equal. The salt is random per comparison, so a token says nothing outside it.
fn tokenize_secret_values(object: &mut Value, salt: &RandomState) {
    for field in ["data", "stringData"] {
        let Some(map) = object.get_mut(field).and_then(Value::as_object_mut) else {
            continue;
        };
        for value in map.values_mut() {
            let token = salt.hash_one(value.as_str().unwrap_or_default());
            *value = Value::from(format!("<hash:{token:016x}>"));
        }
    }
}

/// Removes what differs between two namespaces for no configured reason: the namespace itself, the
/// owners, annotations the controllers write, and the addresses the cluster assigns.
fn drop_namespace_noise(kind: ObjectKind, object: &mut Value) {
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove("namespace");
        metadata.remove("ownerReferences");
        let annotations_left_empty = metadata
            .get_mut("annotations")
            .and_then(Value::as_object_mut)
            .is_some_and(|annotations| {
                for key in VOLATILE_ANNOTATIONS {
                    annotations.remove(key);
                }
                annotations.is_empty()
            });
        if annotations_left_empty {
            metadata.remove("annotations");
        }
    }
    let Some(spec) = object.get_mut("spec").and_then(Value::as_object_mut) else {
        return;
    };
    match kind {
        ObjectKind::Service => {
            spec.remove("clusterIP");
            spec.remove("clusterIPs");
        }
        ObjectKind::PersistentVolumeClaim => {
            spec.remove("volumeName");
        }
        _ => {}
    }
}

/// Matches the two sides by name; both maps are sorted, so every list comes out sorted.
fn compare_kind(left: KindObjects, mut right: KindObjects) -> KindOutcome {
    let mut only_left = Vec::new();
    let mut differs = Vec::new();
    let mut same = 0;
    for (name, left_object) in left {
        let Some(right_object) = right.remove(&name) else {
            only_left.push(name);
            continue;
        };
        if left_object == right_object {
            same += 1;
            continue;
        }
        differs.push(difference(name, &left_object, &right_object));
    }
    KindOutcome::Compared {
        only_left,
        only_right: right.into_keys().collect(),
        differs,
        same,
    }
}

fn difference(name: String, left: &Value, right: &Value) -> ObjectDifference {
    let paths = field_paths(left, right);
    let (changes, more_changes) = field_changes(left, right, &paths);
    // A serialization failure leaves the text empty: the change lines still say what differs.
    let text = |object: &Value| to_yaml_text(object, None).unwrap_or_default();
    ObjectDifference {
        name,
        changes,
        more_changes,
        left_text: text(left),
        right_text: text(right),
    }
}

#[cfg(test)]
#[path = "namespace_compare_tests.rs"]
mod namespace_compare_tests;
