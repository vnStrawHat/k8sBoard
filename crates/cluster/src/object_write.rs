//! The one write path (spec 0030). Every mutating request goes through
//! `ClusterConnection::write`; the `WriteOperation` enum is the allow-list, and clippy
//! `disallowed-methods` keeps every other call site out of the crate.
//!
//! Nothing here logs or keeps a request body or a field value.

use std::borrow::Cow;
use std::fmt;
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::{
    DeleteParams, DynamicObject, Patch, PatchParams, PostParams, Preconditions, PropagationPolicy,
};
use kube::core::Status;
use serde_json::{Value, json};
use tokio::time::error::Elapsed;

use crate::access_review::AccessCheck;
use crate::certificate_renewal::{RenewRefusal, renewal_status_body};
use crate::config_values::{ValuesEdit, values_patch};
use crate::connection::{ClusterConnection, ClusterError, classify_error, run_raw};
use crate::custom_resource_definition::{CustomResourceType, ResourceScope};
use crate::debug_pod_bodies::{
    DEBUG_CONTAINER_PREFIX, NODE_SHELL_PREFIX, NodeShellPod, debug_container_patch,
    is_container_name, is_label_value, is_valid_debug_image, node_shell_pod,
};
use crate::dns_name::{is_dns_label, is_dns_subdomain, is_path_segment_name};
use crate::edit_placeholders::{self, Restored};
use crate::edit_preview::{EditPreview, build_preview};
use crate::node::NodeTaint;
use crate::node_maintenance_bodies::{
    GracePeriod, LabelChange, are_valid_label_changes, are_valid_taints, eviction_body,
    is_valid_uid, labels_patch, taint_changes, taints_patch,
};
use crate::object_create::{ObjectDraft, is_valid_name, missing_paths};
use crate::object_edit::{ObjectEdit, is_helm_release};
use crate::object_metadata::{are_valid_metadata_changes, metadata_patch};
use crate::object_yaml::{ObjectKind, ObjectRef};
use crate::quantity::ByteAmount;
use crate::volume_write_bodies::{
    ReclaimPolicy, RecreateRefusal, reclaim_policy_patch, recreated_claim_body,
};
use crate::workload_write_bodies::{
    RERUN_BASE_CHARS, RERUN_SUFFIX, RollBackRefusal, TRIGGER_BASE_CHARS, TRIGGER_SUFFIX,
    generate_name, is_valid_change_cause, rerun_job_body, rollback_operations, set_image_patch,
    trigger_job_body,
};

/// The server-side field manager of every write k8sBoard sends.
const FIELD_MANAGER: &str = "k8sboard";
const KIND_SECRET: &str = "Secret";
/// What an operation reads before it sends, so the request can carry the current object's data.
const READ_ACTION: &str = "reading the object before the change";
/// The annotation `kubectl rollout restart` sets (kube's own `Api::restart` writes another one).
const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";
/// The path a change cause is listed under in the confirm summary and the audit line.
const CHANGE_CAUSE_PATH: &str = "metadata.annotations[kubernetes.io/change-cause]";
/// Fixed text: a library error could quote the object's content.
const UNUSABLE_OBJECT: &str = "the object could not be used for this change";
const STALE_TEMPLATE: &str = "the deployment was replaced since it was read";
const REJECTED_OBJECT: &str = "the server rejected the generated object";
const REJECTED_TEMPLATE: &str = "the server rejected the template of that revision";
/// A `replicas` field is an `int32` on the server.
const MAX_REPLICAS: u32 = i32::MAX as u32;
/// A uid is a UUID; the bound only keeps a garbage value out of the delete body.
const MAX_UID_LENGTH: usize = 64;
/// The annotation that marks a StorageClass as the default one, and its beta spelling: 0014 reads
/// either key, so an unset clears both.
const DEFAULT_CLASS_ANNOTATION: &str = "storageclass.kubernetes.io/is-default-class";
const DEFAULT_CLASS_BETA_ANNOTATION: &str = "storageclass.beta.kubernetes.io/is-default-class";
/// How long a recreate waits for the old claim to go: 20 polls, half a second apart.
const GONE_POLLS: u32 = 20;
const GONE_POLL: Duration = Duration::from_millis(500);
const UNUSABLE_EVICTION_ANSWER: &str = "the eviction answer was not a success";

/// One allow-listed mutation. Adding a variant is the only way to add a write (C3).
// Debug is manual: the variant name only.
#[derive(Clone, PartialEq, Eq)]
pub enum WriteOperation {
    /// JSON merge patch `{"spec":{"unschedulable": !schedulable}}` on a Node (cordon or uncordon).
    SetNodeSchedulable { schedulable: bool },
    /// Merge patch of the `scale` subresource of a Deployment or StatefulSet (0032). `previous`
    /// is never sent: it lets the summary and the audit line name the old count.
    ScaleWorkload { replicas: u32, previous: u32 },
    /// Merge patch of the pod template's `kubectl.kubernetes.io/restartedAt` annotation on a
    /// Deployment, StatefulSet, or DaemonSet (0032). Rounded to whole seconds on the wire, so a
    /// dry-run and its commit send the same body.
    RestartRollout { restarted_at: jiff::Timestamp },
    /// Merge patch `{"spec":{"paused": b}}` on a Deployment (0032).
    SetRolloutPaused { paused: bool },
    /// JSON Patch that replaces a Deployment's pod template with one of its ReplicaSets
    /// (`kubectl rollout undo --to-revision`, 0032).
    RollBackDeployment { replica_set: String, revision: u64 },
    /// Strategic merge patch of one container's image in the pod template of a Deployment,
    /// StatefulSet, or DaemonSet, with the `kubernetes.io/change-cause` annotation (`kubectl set
    /// image`, 0032). `previous_image` is never sent: it lets the summary name the change.
    SetContainerImage {
        container: String,
        image: String,
        previous_image: String,
        change_cause: Option<String>,
    },
    /// Merge patch `{"spec":{"suspend": b}}` on a CronJob (0032).
    SetCronJobSuspended { suspended: bool },
    /// Creates a Job from a CronJob's template (`kubectl create job --from`, 0032).
    TriggerCronJob,
    /// Creates a standalone copy of a Job (0032).
    RerunJob,
    /// `PUT` of an edited object with its base `resourceVersion` and `uid`, placeholders restored
    /// from a fresh GET (0031).
    ReplaceObject(Box<ObjectEdit>),
    /// `DELETE` pinned to `uid` (a recreated object of the same name is never deleted), with an
    /// explicit propagation policy (0033).
    DeleteObject {
        uid: String,
        propagation: DeletePropagation,
    },
    /// Strategic merge patch of the `ephemeralcontainers` subresource: appends one debug container
    /// that shares `target_container`'s process namespace (0037).
    AddDebugContainer {
        name: String,
        image: String,
        target_container: String,
    },
    /// Creates the privileged pod of a node shell on `node`; `instance` is the id of this app run
    /// (0037). The target is the pod to create, named `k8sboard-node-shell-…`.
    CreateNodeShellPod {
        node: String,
        image: String,
        user: Option<String>,
        instance: String,
    },
    /// Deletes a node shell pod this run (or a sweep) found, with a `uid` precondition and no grace
    /// period. Commit only: it has no dry-run (0037).
    DeleteNodeShellPod { uid: String },
    /// Merge patch of `spec.minReplicas` and `spec.maxReplicas` on a HorizontalPodAutoscaler
    /// (0032b). Only a field that differs from `previous_min` / `previous_max` is sent.
    SetHpaReplicaRange {
        min: u32,
        max: u32,
        previous_min: u32,
        previous_max: u32,
    },
    /// Merge patch of a PersistentVolumeClaim's storage request (0032b). `storage` is a
    /// Kubernetes quantity, stored trimmed and sent as typed.
    ExpandClaim { storage: String },
    /// Merge patch of a StorageClass's default-class annotation (0032b).
    SetDefaultStorageClass { is_default: bool },
    /// Merge patch of a PersistentVolume's `spec.persistentVolumeReclaimPolicy` (0032b, UX round 3).
    /// `previous` is never sent: it lets the summary and the audit line name the old policy.
    SetReclaimPolicy {
        policy: ReclaimPolicy,
        previous: ReclaimPolicy,
    },
    /// Deletes a Pending, unbound PersistentVolumeClaim pinned to `uid`, waits until it is gone,
    /// and creates it again with `storage_class` and the same size, access modes, volume mode,
    /// selector, and labels (0032b, UX round 3). The dry-run checks the claim and dry-runs the
    /// delete; the create cannot be dry-run while the old claim exists. `previous_class` is never
    /// sent: it lets the summary and the audit line name the old class.
    RecreateClaim {
        uid: String,
        storage_class: String,
        previous_class: Option<String>,
    },
    /// `POST` of a `policy/v1` Eviction to a pod, pinned to `uid` so a recreated pod of the same
    /// name is never evicted. The API server checks the pod's PodDisruptionBudget (0034).
    EvictPod { uid: String, grace: GracePeriod },
    /// Merge patch of `spec.taints` (the whole list) guarded by `resource_version`, so a change
    /// made meanwhile is a 409 instead of a lost update (0034).
    SetNodeTaints {
        taints: Vec<NodeTaint>,
        resource_version: String,
        /// The taints the edit was made against: never sent, they let the summary and the audit line
        /// name each taint that changed.
        previous: Vec<NodeTaint>,
    },
    /// Per-key merge patch of `metadata.labels` (0034).
    SetNodeLabels { changes: Vec<LabelChange> },
    /// Per-key merge patch of `metadata.labels` and `metadata.annotations` of a Pod or a workload
    /// (0032b). Annotation values are never audited.
    SetObjectMetadata {
        labels: Vec<LabelChange>,
        annotations: Vec<LabelChange>,
    },
    /// Merge patch of the changed keys of a ConfigMap or Secret, guarded by the base
    /// `resourceVersion` (0047). The edit holds new values: nothing prints it.
    SetDataValues(Box<ValuesEdit>),
    /// `POST` of a new object of a creatable kind (0042). The draft holds user text: nothing
    /// prints it.
    CreateObject(Box<ObjectDraft>),
    /// `PUT` of a cert-manager `Certificate`'s `status` adding `Issuing=True` (`cmctl renew`, 0018
    /// step 6). `requested_at` is the condition time, fixed so the dry-run and the commit send
    /// one body.
    RenewCertificate { requested_at: jiff::Timestamp },
}

impl WriteOperation {
    fn name(&self) -> &'static str {
        match self {
            Self::SetNodeSchedulable { .. } => "SetNodeSchedulable",
            Self::ScaleWorkload { .. } => "ScaleWorkload",
            Self::RestartRollout { .. } => "RestartRollout",
            Self::SetRolloutPaused { .. } => "SetRolloutPaused",
            Self::RollBackDeployment { .. } => "RollBackDeployment",
            Self::SetContainerImage { .. } => "SetContainerImage",
            Self::SetCronJobSuspended { .. } => "SetCronJobSuspended",
            Self::TriggerCronJob => "TriggerCronJob",
            Self::RerunJob => "RerunJob",
            Self::ReplaceObject(_) => "ReplaceObject",
            Self::DeleteObject { .. } => "DeleteObject",
            Self::AddDebugContainer { .. } => "AddDebugContainer",
            Self::CreateNodeShellPod { .. } => "CreateNodeShellPod",
            Self::DeleteNodeShellPod { .. } => "DeleteNodeShellPod",
            Self::SetHpaReplicaRange { .. } => "SetHpaReplicaRange",
            Self::ExpandClaim { .. } => "ExpandClaim",
            Self::SetDefaultStorageClass { .. } => "SetDefaultStorageClass",
            Self::SetReclaimPolicy { .. } => "SetReclaimPolicy",
            Self::RecreateClaim { .. } => "RecreateClaim",
            Self::EvictPod { .. } => "EvictPod",
            Self::SetNodeTaints { .. } => "SetNodeTaints",
            Self::SetNodeLabels { .. } => "SetNodeLabels",
            Self::SetObjectMetadata { .. } => "SetObjectMetadata",
            Self::SetDataValues(_) => "SetDataValues",
            Self::CreateObject(_) => "CreateObject",
            Self::RenewCertificate { .. } => "RenewCertificate",
        }
    }
}

impl fmt::Debug for WriteOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// What happens to the objects a deleted object owns (0033 decision 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeletePropagation {
    /// The owner goes at once; a garbage collector deletes the dependents afterwards.
    #[default]
    Background,
    /// The owner stays until its dependents are gone.
    Foreground,
    /// The dependents keep running without an owner.
    Orphan,
}

impl DeletePropagation {
    /// The API policy name; also the audit value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Background => "Background",
            Self::Foreground => "Foreground",
            Self::Orphan => "Orphan",
        }
    }

    fn policy(self) -> PropagationPolicy {
        match self {
            Self::Background => PropagationPolicy::Background,
            Self::Foreground => PropagationPolicy::Foreground,
            Self::Orphan => PropagationPolicy::Orphan,
        }
    }
}

/// A write whose target kind fits its operation. Always valid.
// Debug is manual: operation, kind, namespace, name; never a body or a field value.
#[derive(Clone, PartialEq, Eq)]
pub struct WriteRequest {
    target: ObjectRef,
    operation: WriteOperation,
    access_check: AccessCheck,
}

impl fmt::Debug for WriteRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteRequest")
            .field("operation", &self.operation)
            .field("kind", &format_args!("{}", self.target.kind_name()))
            .field("namespace", &self.target.namespace())
            .field("name", &self.target.name())
            .finish()
    }
}

/// Whether a request only asks the server to check the change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    DryRun,
    Commit,
}

/// What the server did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteEffect {
    Patched,
    /// A create (Trigger now, Re-run); a commit reports `WriteOutcome.created_name`.
    Created,
    /// A replace (Edit YAML): what changed, masked.
    Replaced(EditPreview),
    /// A delete the server completed.
    Deleted,
    /// A delete the server accepted that waits for finalizers or a grace period.
    DeletionPending {
        finalizers: Vec<String>,
    },
}

/// What a finished write reports. `created_name` and `uid` are `None` on a dry-run and when the
/// operation creates or replaces nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteOutcome {
    pub mode: WriteMode,
    pub elapsed: Duration,
    pub effect: WriteEffect,
    pub created_name: Option<String>,
    pub uid: Option<String>,
    /// Paths of the draft the server dropped (0042 decision 14); empty for every other operation.
    pub dropped_fields: Vec<String>,
}

/// One field a write changes, for the confirm summary and the audit line. `None` means the value
/// is not recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedField {
    pub path: Cow<'static, str>,
    pub value: Option<String>,
    /// The value the field has now, when the request knows it and may record it.
    pub from: Option<String>,
}

/// The `lab-writes` build: the screenshot build with the kind-lab opt-in (see `WritePolicy::of_build`).
const IS_LAB_BUILD: bool = cfg!(all(feature = "lab-writes", feature = "block-writes"));

/// Why a write is refused, for the notice of the user.
pub(crate) const WRITES_BLOCKED_MESSAGE: &str = if IS_LAB_BUILD {
    "writes stay blocked outside a kind-* context (lab-writes build)"
} else {
    "writes are blocked in this screenshot build"
};

/// Whether a connection may send writes at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritePolicy {
    Allowed,
    Blocked,
}

impl WritePolicy {
    /// The policy of this build for `context`: every build writes except the screenshot build
    /// (`block-writes`), and its `lab-writes` variant writes only on a context named `kind-*`.
    pub(crate) fn of_build(context: &str) -> Self {
        Self::policy(cfg!(feature = "block-writes"), IS_LAB_BUILD, context)
    }

    fn policy(blocks_writes: bool, is_lab_build: bool, context: &str) -> Self {
        if !blocks_writes || (is_lab_build && context.starts_with("kind-")) {
            Self::Allowed
        } else {
            Self::Blocked
        }
    }
}

impl WriteRequest {
    /// `None` when the target kind does not fit the operation, or when its name or namespace is
    /// not a DNS-1123 subdomain: kube does not encode them, so any other text could change the
    /// path the request goes to. The RBAC kinds of `ReplaceObject` and `DeleteObject` may use `:` in a name, so they
    /// follow the path-segment rule instead.
    pub fn new(target: ObjectRef, operation: WriteOperation) -> Option<Self> {
        // `replicas` is an int32: a larger number can only be a typo, and the server would refuse it.
        if matches!(operation, WriteOperation::ScaleWorkload { replicas, .. } if replicas > MAX_REPLICAS)
        {
            return None;
        }
        // Without a uid the delete could hit a recreated object of the same name.
        if matches!(&operation, WriteOperation::DeleteObject { uid, .. } if uid.is_empty()) {
            return None;
        }
        let operation = checked_operation(operation)?;
        let access_check = fitting_access_check(&target, &operation)?;
        is_safe_path(&target, &operation).then_some(Self {
            target,
            operation,
            access_check,
        })
    }

    pub fn target(&self) -> &ObjectRef {
        &self.target
    }

    pub fn operation(&self) -> &WriteOperation {
        &self.operation
    }

    /// The permission the operation needs; the app gate reads the same value.
    pub fn access_check(&self) -> AccessCheck {
        self.access_check
    }

    pub fn changed_fields(&self) -> Vec<ChangedField> {
        let field = |path: &'static str, value: String| ChangedField {
            path: Cow::Borrowed(path),
            value: Some(value),
            from: None,
        };
        let changed = |path: &'static str, from: u32, to: u32| ChangedField {
            path: Cow::Borrowed(path),
            value: Some(to.to_string()),
            from: Some(from.to_string()),
        };
        match &self.operation {
            WriteOperation::SetNodeSchedulable { schedulable } => {
                vec![field("spec.unschedulable", (!schedulable).to_string())]
            }
            WriteOperation::ScaleWorkload { replicas, previous } => {
                vec![changed("spec.replicas", *previous, *replicas)]
            }
            WriteOperation::RestartRollout { restarted_at } => vec![field(
                "spec.template.metadata.annotations[kubectl.kubernetes.io/restartedAt]",
                restart_stamp(restarted_at),
            )],
            WriteOperation::SetRolloutPaused { paused } => {
                vec![field("spec.paused", paused.to_string())]
            }
            WriteOperation::RollBackDeployment {
                replica_set,
                revision,
            } => vec![field(
                "spec.template",
                format!("rev {revision} ({replica_set})"),
            )],
            WriteOperation::SetContainerImage {
                container,
                image,
                change_cause,
                ..
            } => {
                let mut fields = vec![ChangedField {
                    path: Cow::Owned(format!("spec.template.spec.containers[{container}].image")),
                    value: Some(image.clone()),
                    from: None,
                }];
                fields.extend(change_cause.iter().map(|cause| ChangedField {
                    path: Cow::Borrowed(CHANGE_CAUSE_PATH),
                    value: Some(cause.clone()),
                    from: None,
                }));
                fields
            }
            WriteOperation::SetCronJobSuspended { suspended } => {
                vec![field("spec.suspend", suspended.to_string())]
            }
            WriteOperation::TriggerCronJob => vec![field(
                "metadata.generateName",
                generate_name(self.target.name(), TRIGGER_BASE_CHARS, TRIGGER_SUFFIX),
            )],
            WriteOperation::RerunJob => vec![field(
                "metadata.generateName",
                generate_name(self.target.name(), RERUN_BASE_CHARS, RERUN_SUFFIX),
            )],
            // Paths only: an edit's values can hold credentials.
            WriteOperation::ReplaceObject(edit) => edit
                .changed_paths()
                .iter()
                .map(|path| ChangedField {
                    path: Cow::Owned(path.to_string()),
                    value: None,
                    from: None,
                })
                .collect(),
            WriteOperation::DeleteObject { propagation, .. } => {
                vec![field(
                    "deleteOptions.propagationPolicy",
                    propagation.as_str().to_owned(),
                )]
            }
            WriteOperation::AddDebugContainer {
                name,
                image,
                target_container,
            } => vec![
                field("spec.ephemeralContainers[].name", name.clone()),
                field("spec.ephemeralContainers[].image", image.clone()),
                field(
                    "spec.ephemeralContainers[].targetContainerName",
                    target_container.clone(),
                ),
            ],
            WriteOperation::CreateNodeShellPod { node, image, .. } => vec![
                field("spec.nodeName", node.clone()),
                field("spec.hostPID", "true".to_owned()),
                field(
                    "spec.containers[0].securityContext.privileged",
                    "true".to_owned(),
                ),
                field("spec.containers[0].image", image.clone()),
            ],
            WriteOperation::DeleteNodeShellPod { uid } => {
                vec![field("metadata.uid", uid.clone())]
            }
            WriteOperation::SetHpaReplicaRange {
                min,
                max,
                previous_min,
                previous_max,
            } => [
                (min != previous_min).then(|| changed("spec.minReplicas", *previous_min, *min)),
                (max != previous_max).then(|| changed("spec.maxReplicas", *previous_max, *max)),
            ]
            .into_iter()
            .flatten()
            .collect(),
            WriteOperation::ExpandClaim { storage } => {
                vec![field("spec.resources.requests.storage", storage.clone())]
            }
            WriteOperation::SetDefaultStorageClass { is_default: true } => vec![field(
                "metadata.annotations[storageclass.kubernetes.io/is-default-class]",
                "true".to_owned(),
            )],
            // The beta key is removed, not set, so it has no value.
            WriteOperation::SetDefaultStorageClass { is_default: false } => vec![
                field(
                    "metadata.annotations[storageclass.kubernetes.io/is-default-class]",
                    "false".to_owned(),
                ),
                ChangedField {
                    path: Cow::Borrowed(
                        "metadata.annotations[storageclass.beta.kubernetes.io/is-default-class]",
                    ),
                    value: None,
                    from: None,
                },
            ],
            WriteOperation::SetReclaimPolicy { policy, previous } => vec![ChangedField {
                path: Cow::Borrowed("spec.persistentVolumeReclaimPolicy"),
                value: Some(policy.to_string()),
                from: Some(previous.to_string()),
            }],
            WriteOperation::RecreateClaim {
                storage_class,
                previous_class,
                ..
            } => vec![ChangedField {
                path: Cow::Borrowed("spec.storageClassName"),
                value: Some(storage_class.clone()),
                from: previous_class.clone(),
            }],
            WriteOperation::EvictPod { grace, .. } => vec![field(
                "pods/eviction",
                match grace {
                    GracePeriod::PodDefault => "grace pod default".to_owned(),
                    GracePeriod::Seconds(seconds) => format!("grace {seconds}s"),
                },
            )],
            WriteOperation::SetNodeTaints {
                taints, previous, ..
            } => {
                let changes = taint_changes(previous, taints);
                if changes.is_empty() {
                    vec![field("spec.taints", "unchanged".to_owned())]
                } else {
                    changes
                        .into_iter()
                        .map(|text| ChangedField {
                            path: Cow::Owned(text),
                            value: None,
                            from: None,
                        })
                        .collect()
                }
            }
            WriteOperation::SetNodeLabels { changes } => vec![field(
                "metadata.labels",
                changes
                    .iter()
                    .map(|change| match &change.value {
                        Some(value) => format!("{}={value}", change.key),
                        None => format!("-{}", change.key),
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
            )],
            // Label values are audited like a node's; an annotation is named, never quoted: it can
            // hold anything a tool wrote.
            WriteOperation::SetObjectMetadata {
                labels,
                annotations,
            } => labels
                .iter()
                .map(|change| ChangedField {
                    path: Cow::Owned(format!("metadata.labels[{}]", change.key)),
                    value: change.value.clone(),
                    from: None,
                })
                .chain(annotations.iter().map(|change| ChangedField {
                    path: Cow::Owned(format!("metadata.annotations[{}]", change.key)),
                    value: None,
                    from: None,
                }))
                .collect(),
            // Names and markers only: a value never reaches the dialog or the audit line (0047).
            WriteOperation::SetDataValues(edit) => edit
                .change_paths()
                .map(|path| ChangedField {
                    path: Cow::Owned(path),
                    value: None,
                    from: None,
                })
                .collect(),
            // Names and paths only: a ConfigMap value never reaches the dialog or the audit line.
            WriteOperation::CreateObject(draft) => draft.changed_fields(),
            WriteOperation::RenewCertificate { .. } => vec![field(
                "status.conditions[Issuing]",
                "True (ManuallyTriggered)".to_owned(),
            )],
        }
    }

    /// False only for operations the server cannot dry-run: the node shell delete is a commit
    /// only (0030 decision 5), because its `uid` precondition already makes it exact.
    pub fn supports_dry_run(&self) -> bool {
        match self.operation {
            WriteOperation::DeleteNodeShellPod { .. } => false,
            WriteOperation::AddDebugContainer { .. }
            | WriteOperation::CreateNodeShellPod { .. } => true,
            WriteOperation::SetNodeSchedulable { .. }
            | WriteOperation::ScaleWorkload { .. }
            | WriteOperation::RestartRollout { .. }
            | WriteOperation::SetRolloutPaused { .. }
            | WriteOperation::RollBackDeployment { .. }
            | WriteOperation::SetContainerImage { .. }
            | WriteOperation::SetCronJobSuspended { .. }
            | WriteOperation::TriggerCronJob
            | WriteOperation::RerunJob
            | WriteOperation::ReplaceObject(_)
            | WriteOperation::DeleteObject { .. }
            | WriteOperation::SetHpaReplicaRange { .. }
            | WriteOperation::ExpandClaim { .. }
            | WriteOperation::SetDefaultStorageClass { .. }
            | WriteOperation::SetReclaimPolicy { .. }
            | WriteOperation::RecreateClaim { .. }
            | WriteOperation::EvictPod { .. }
            | WriteOperation::SetNodeTaints { .. }
            | WriteOperation::SetNodeLabels { .. }
            | WriteOperation::SetObjectMetadata { .. }
            | WriteOperation::SetDataValues(_)
            | WriteOperation::CreateObject(_)
            | WriteOperation::RenewCertificate { .. } => true,
        }
    }
}

/// The operation with its values validated: `None` for a request the API server would refuse
/// (0032b decision 2). An expand's quantity is stored trimmed.
fn checked_operation(operation: WriteOperation) -> Option<WriteOperation> {
    match operation {
        // `minReplicas: 0` needs an alpha feature gate and the API rejects `min > max`; both
        // fields are int32.
        WriteOperation::SetHpaReplicaRange { min, max, .. }
            if min == 0 || min > max || max > MAX_REPLICAS =>
        {
            None
        }
        WriteOperation::ExpandClaim { storage } => {
            let storage = storage.trim();
            let amount = ByteAmount::parse(storage)?;
            (amount.bytes() > 0).then(|| WriteOperation::ExpandClaim {
                storage: storage.to_owned(),
            })
        }
        WriteOperation::EvictPod { ref uid, .. } if !is_valid_uid(uid) => None,
        // A class name is a DNS subdomain, which is also what keeps it out of the body's structure.
        WriteOperation::RecreateClaim {
            ref uid,
            ref storage_class,
            ..
        } if !is_valid_uid(uid) || !is_dns_subdomain(storage_class) => None,
        WriteOperation::SetNodeTaints {
            ref taints,
            ref resource_version,
            ..
        } if resource_version.is_empty() || !are_valid_taints(taints) => None,
        WriteOperation::SetNodeLabels { ref changes } if !are_valid_label_changes(changes) => None,
        WriteOperation::SetObjectMetadata {
            ref labels,
            ref annotations,
        } if !are_valid_metadata_changes(labels, annotations) => None,
        // A draft is checked again here: its body must still agree with its target (0042 decision 5).
        WriteOperation::CreateObject(draft) => draft
            .is_consistent()
            .then_some(WriteOperation::CreateObject(draft)),
        // A cause is stored trimmed, and an empty one means none.
        WriteOperation::SetContainerImage {
            container,
            image,
            previous_image,
            change_cause,
        } => {
            let change_cause = change_cause
                .map(|cause| cause.trim().to_owned())
                .filter(|cause| !cause.is_empty());
            let is_valid = is_container_name(&container)
                && is_valid_debug_image(&image)
                && change_cause.as_deref().is_none_or(is_valid_change_cause);
            is_valid.then_some(WriteOperation::SetContainerImage {
                container,
                image,
                previous_image,
                change_cause,
            })
        }
        // Listed one by one, not as `other`: a new operation does not compile until it gets a
        // validation decision here.
        operation @ (WriteOperation::SetNodeSchedulable { .. }
        | WriteOperation::ScaleWorkload { .. }
        | WriteOperation::RestartRollout { .. }
        | WriteOperation::SetRolloutPaused { .. }
        | WriteOperation::RollBackDeployment { .. }
        | WriteOperation::SetCronJobSuspended { .. }
        | WriteOperation::TriggerCronJob
        | WriteOperation::RerunJob
        | WriteOperation::ReplaceObject(_)
        | WriteOperation::DeleteObject { .. }
        | WriteOperation::AddDebugContainer { .. }
        | WriteOperation::CreateNodeShellPod { .. }
        | WriteOperation::DeleteNodeShellPod { .. }
        | WriteOperation::SetHpaReplicaRange { .. }
        | WriteOperation::SetDefaultStorageClass { .. }
        | WriteOperation::SetReclaimPolicy { .. }
        | WriteOperation::RecreateClaim { .. }
        | WriteOperation::EvictPod { .. }
        | WriteOperation::SetNodeTaints { .. }
        | WriteOperation::SetNodeLabels { .. }
        | WriteOperation::SetObjectMetadata { .. }
        | WriteOperation::SetDataValues(_)
        // The target rule (`fitting_access_check`) carries the check: the operation has no value
        // of its own to validate.
        | WriteOperation::RenewCertificate { .. }) => Some(operation),
    }
}

/// The one custom resource a write may reach: cert-manager `Certificate` at `v1`, namespaced.
fn is_cert_manager_certificate(resource: &CustomResourceType) -> bool {
    resource.group == "cert-manager.io"
        && resource.version == "v1"
        && resource.plural == "certificates"
        && resource.kind == "Certificate"
        && resource.scope == ResourceScope::Namespaced
}

/// The permission `operation` needs on `target`, or `None` when the kind does not fit.
fn fitting_access_check(target: &ObjectRef, operation: &WriteOperation) -> Option<AccessCheck> {
    // Before `builtin_kind`, which is `None` for every custom target: only Renew now reaches one.
    if let Some((resource, namespace, _)) = target.as_custom() {
        return (matches!(operation, WriteOperation::RenewCertificate { .. })
            && is_cert_manager_certificate(resource)
            && namespace.is_some())
        .then_some(AccessCheck::UpdateCertificateStatus);
    }
    let kind = target.builtin_kind()?;
    Some(match (operation, kind) {
        (
            WriteOperation::SetNodeSchedulable { .. }
            | WriteOperation::SetNodeTaints { .. }
            | WriteOperation::SetNodeLabels { .. },
            ObjectKind::Node,
        ) => AccessCheck::PatchNodes,
        (WriteOperation::EvictPod { .. }, ObjectKind::Pod) => AccessCheck::CreatePodEviction,
        (
            WriteOperation::SetObjectMetadata { .. },
            kind @ (ObjectKind::Pod
            | ObjectKind::Deployment
            | ObjectKind::StatefulSet
            | ObjectKind::DaemonSet
            | ObjectKind::ReplicaSet
            | ObjectKind::Job
            | ObjectKind::CronJob),
        ) => AccessCheck::Patch(kind),
        (WriteOperation::ScaleWorkload { .. }, ObjectKind::Deployment) => {
            AccessCheck::PatchDeploymentScale
        }
        (WriteOperation::ScaleWorkload { .. }, ObjectKind::StatefulSet) => {
            AccessCheck::PatchStatefulSetScale
        }
        (
            WriteOperation::RestartRollout { .. } | WriteOperation::SetContainerImage { .. },
            ObjectKind::Deployment,
        ) => AccessCheck::PatchDeployments,
        (
            WriteOperation::RestartRollout { .. } | WriteOperation::SetContainerImage { .. },
            ObjectKind::StatefulSet,
        ) => AccessCheck::PatchStatefulSets,
        (
            WriteOperation::RestartRollout { .. } | WriteOperation::SetContainerImage { .. },
            ObjectKind::DaemonSet,
        ) => AccessCheck::PatchDaemonSets,
        (
            WriteOperation::SetRolloutPaused { .. } | WriteOperation::RollBackDeployment { .. },
            ObjectKind::Deployment,
        ) => AccessCheck::PatchDeployments,
        (WriteOperation::SetCronJobSuspended { .. }, ObjectKind::CronJob) => {
            AccessCheck::PatchCronJobs
        }
        (WriteOperation::TriggerCronJob, ObjectKind::CronJob)
        | (WriteOperation::RerunJob, ObjectKind::Job) => AccessCheck::CreateJobs,
        (WriteOperation::ReplaceObject(edit), kind)
            if edit.target() == target && kind.is_editable() =>
        {
            AccessCheck::Update(kind)
        }
        (WriteOperation::DeleteObject { .. }, kind) => AccessCheck::Delete(kind),
        (
            WriteOperation::SetDataValues(edit),
            kind @ (ObjectKind::ConfigMap | ObjectKind::Secret),
        ) if edit.target() == target => AccessCheck::Patch(kind),
        (WriteOperation::CreateObject(draft), kind)
            if draft.target() == target && kind.is_creatable() =>
        {
            AccessCheck::Create(kind)
        }
        (WriteOperation::AddDebugContainer { .. }, ObjectKind::Pod) => {
            AccessCheck::PatchPodEphemeralContainers
        }
        (WriteOperation::CreateNodeShellPod { .. }, ObjectKind::Pod) => AccessCheck::CreatePods,
        (WriteOperation::DeleteNodeShellPod { .. }, ObjectKind::Pod) => AccessCheck::DeletePods,
        (WriteOperation::SetHpaReplicaRange { .. }, ObjectKind::HorizontalPodAutoscaler) => {
            AccessCheck::PatchHorizontalPodAutoscalers
        }
        (WriteOperation::ExpandClaim { .. }, ObjectKind::PersistentVolumeClaim) => {
            AccessCheck::PatchPersistentVolumeClaims
        }
        (WriteOperation::SetDefaultStorageClass { .. }, ObjectKind::StorageClass) => {
            AccessCheck::PatchStorageClasses
        }
        (WriteOperation::SetReclaimPolicy { .. }, ObjectKind::PersistentVolume) => {
            AccessCheck::Patch(ObjectKind::PersistentVolume)
        }
        // The create is the server's to refuse: the gate reads both rights, the request names the
        // one a dry-run can show.
        (WriteOperation::RecreateClaim { .. }, ObjectKind::PersistentVolumeClaim) => {
            AccessCheck::Delete(ObjectKind::PersistentVolumeClaim)
        }
        _ => return None,
    })
}

/// Whether every name that ends up in the request path is safe to put there.
fn is_safe_path(target: &ObjectRef, operation: &WriteOperation) -> bool {
    let is_rbac = matches!(
        target.builtin_kind(),
        Some(
            ObjectKind::Role
                | ObjectKind::ClusterRole
                | ObjectKind::RoleBinding
                | ObjectKind::ClusterRoleBinding
        )
    );
    let is_safe_name = match operation {
        WriteOperation::ReplaceObject(_) | WriteOperation::DeleteObject { .. } if is_rbac => {
            is_path_segment_name(target.name())
        }
        WriteOperation::CreateObject(_) => target
            .builtin_kind()
            .is_some_and(|kind| is_valid_name(kind, target.name())),
        _ => is_dns_subdomain(target.name()),
    };
    // A created object's namespace is in both the path and the body: a DNS label.
    let is_safe_namespace = match operation {
        WriteOperation::CreateObject(_) => target.namespace().is_none_or(is_dns_label),
        _ => target.namespace().is_none_or(is_dns_subdomain),
    };
    let is_safe_replica_set = match operation {
        WriteOperation::RollBackDeployment { replica_set, .. } => is_dns_subdomain(replica_set),
        _ => true,
    };
    is_safe_name
        && is_safe_replica_set
        && is_safe_namespace
        && is_fit_debug_operation(target, operation)
}

/// What the debug operations need beyond a pod target: the names that mark a k8sBoard object, so
/// the privileged create and the delete bypass reach nothing else, and values the server accepts.
fn is_fit_debug_operation(target: &ObjectRef, operation: &WriteOperation) -> bool {
    let is_node_shell_pod = target.name().starts_with(NODE_SHELL_PREFIX);
    match operation {
        WriteOperation::AddDebugContainer {
            name,
            image,
            target_container,
        } => {
            name.starts_with(DEBUG_CONTAINER_PREFIX)
                && is_container_name(name)
                && is_container_name(target_container)
                && is_valid_debug_image(image)
        }
        WriteOperation::CreateNodeShellPod {
            node,
            image,
            instance,
            ..
        } => {
            is_node_shell_pod
                && is_dns_subdomain(node)
                && is_valid_debug_image(image)
                && !instance.is_empty()
                && is_label_value(instance)
        }
        WriteOperation::DeleteNodeShellPod { uid } => {
            is_node_shell_pod
                && (1..=MAX_UID_LENGTH).contains(&uid.len())
                && uid
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        }
        _ => true,
    }
}

/// Whole seconds, UTC, like kubectl writes it.
fn restart_stamp(timestamp: &jiff::Timestamp) -> String {
    timestamp.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// A failed write, sorted by what the user can do about it.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("{}", WRITES_BLOCKED_MESSAGE)]
    WritesBlocked,
    /// A 403 of the RBAC form (`is forbidden: User`).
    #[error("not permitted: {message}")]
    Denied { message: String },
    #[error("the object no longer exists")]
    NotFound,
    /// A 409. `managers` stays empty: there is no server-side apply.
    #[error("the object changed since it was read: {message}")]
    Conflict {
        message: String,
        managers: Vec<String>,
    },
    /// A 422, and any other 403 (an admission plugin or webhook refusal).
    #[error("the change is invalid: {message}")]
    Invalid {
        message: String,
        fields: Vec<String>,
    },
    /// A 429 on either mode; nothing changed.
    #[error("refused for now: {message}")]
    TooManyRequests {
        message: String,
        retry_after: Option<Duration>,
    },
    #[error(
        "an admission webhook does not support dry-run, so the change cannot be checked: {reason}"
    )]
    DryRunRejected { reason: String },
    /// A commit whose request may have left the client before it failed.
    #[error("no answer in time; the change may have been applied")]
    OutcomeUnknown,
    #[error(transparent)]
    Cluster(#[from] ClusterError),
}

/// What `send` learned from the server; `write` shapes it into a `WriteOutcome`. Each operation
/// builds its own effect, so no operation reports another one's.
struct Answer {
    effect: WriteEffect,
    created_name: Option<String>,
    uid: Option<String>,
    dropped_fields: Vec<String>,
}

impl Answer {
    fn patched() -> Self {
        Self::of(WriteEffect::Patched)
    }

    fn of(effect: WriteEffect) -> Self {
        Self {
            effect,
            created_name: None,
            uid: None,
            dropped_fields: Vec::new(),
        }
    }

    /// `kept` is the object the server answered with while it waits (`Left` of kube's delete); a
    /// plain status (`Right`) is `None`. Only finalizer names survive: a Secret's data must not
    /// outlive this call.
    fn deleted(kept: Option<DynamicObject>) -> Self {
        let pending = kept.filter(|object| object.metadata.deletion_timestamp.is_some());
        match pending {
            Some(object) => Self::of(WriteEffect::DeletionPending {
                finalizers: object.metadata.finalizers.unwrap_or_default(),
            }),
            None => Self::of(WriteEffect::Deleted),
        }
    }

    /// A commit reports the name the server picked; a dry-run reports none.
    fn created(object: &DynamicObject, mode: WriteMode) -> Self {
        let created_name = match mode {
            WriteMode::Commit => object.metadata.name.clone(),
            WriteMode::DryRun => None,
        };
        Self {
            effect: WriteEffect::Created,
            created_name,
            uid: committed_uid(object, mode),
            dropped_fields: Vec::new(),
        }
    }
}

/// A commit reports the uid of the object the server created or replaced; a dry-run reports none.
fn committed_uid(object: &DynamicObject, mode: WriteMode) -> Option<String> {
    match mode {
        WriteMode::Commit => object.metadata.uid.clone(),
        WriteMode::DryRun => None,
    }
}

/// What a replace sends, and the fresh object it was built from.
struct Replacement {
    body: DynamicObject,
    fresh: Value,
    restored: Restored,
}

impl ClusterConnection {
    /// The only function that sends a mutating request. Returns `WritesBlocked` before building any
    /// request when the connection's policy is `Blocked`. Every request sets `fieldManager=k8sboard`
    /// in both modes (a delete has no query and sends `dryRun` in its body), and `DryRun` adds
    /// `dryRun=All`. The caller passes the connection of the
    /// target's own cluster; there is no implicit current session.
    pub async fn write(
        &self,
        request: &WriteRequest,
        mode: WriteMode,
    ) -> Result<WriteOutcome, WriteError> {
        if self.write_policy() == WritePolicy::Blocked {
            return Err(WriteError::WritesBlocked);
        }
        // A delete that is exact by its precondition has no dry-run; asking for one would send the
        // real delete.
        if mode == WriteMode::DryRun && !request.supports_dry_run() {
            return Err(WriteError::Invalid {
                message: "this change has no dry-run".to_owned(),
                fields: Vec::new(),
            });
        }
        let started = Instant::now();
        let sent = self.send(request, mode).await;
        let elapsed = started.elapsed();
        tracing::debug!(
            context = %self.context(),
            operation = ?request.operation,
            kind = request.target.kind_name(),
            namespace = request.target.namespace(),
            name = request.target.name(),
            ?mode,
            ?elapsed,
            is_ok = sent.is_ok(),
            "write finished"
        );
        let answer = sent?;
        Ok(WriteOutcome {
            mode,
            elapsed,
            effect: answer.effect,
            created_name: answer.created_name,
            uid: answer.uid,
            dropped_fields: answer.dropped_fields,
        })
    }

    /// Sends the request of one allow-listed operation: one arm per `WriteOperation`, the
    /// object_write.rs row of the 0030 exception table. An operation that needs the object's data
    /// reads it first through `self.run` (a failed read is a `Cluster` error: nothing was sent).
    #[allow(clippy::disallowed_methods)]
    async fn send(&self, request: &WriteRequest, mode: WriteMode) -> Result<Answer, WriteError> {
        let api = self.object_api(&request.target);
        let name = request.target.name();
        let params = patch_params(mode);
        match &request.operation {
            WriteOperation::SetNodeSchedulable { schedulable } => {
                let body = json!({ "spec": { "unschedulable": !schedulable } });
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::ScaleWorkload { replicas, .. } => {
                let body = json!({ "spec": { "replicas": replicas } });
                let sent = run_raw(api.patch_scale(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::RestartRollout { restarted_at } => {
                let annotations = json!({ RESTARTED_AT: restart_stamp(restarted_at) });
                let body = json!({
                    "spec": { "template": { "metadata": { "annotations": annotations } } }
                });
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::SetRolloutPaused { paused } => {
                let body = json!({ "spec": { "paused": paused } });
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::RollBackDeployment {
                replica_set,
                revision,
            } => {
                let operations = self
                    .roll_back_operations(&request.target, replica_set, *revision, mode)
                    .await?;
                // JSON Patch, not a merge patch: a merge patch would keep map keys the old
                // revision's template lacks.
                let operations =
                    serde_json::from_value(operations).map_err(|_| self.unusable_object(mode))?;
                let patch = Patch::<()>::Json(operations);
                let sent = run_raw(api.patch(name, &params, &patch)).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::SetContainerImage {
                container,
                image,
                change_cause,
                ..
            } => {
                let body = set_image_patch(container, image, change_cause.as_deref());
                let sent = run_raw(api.patch(name, &params, &Patch::Strategic(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::SetCronJobSuspended { suspended } => {
                let body = json!({ "spec": { "suspend": suspended } });
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::TriggerCronJob => {
                let (jobs, job) = self.trigger_job(&request.target, mode).await?;
                let sent = run_raw(self.object_api(&jobs).create(&post_params(mode), &job)).await;
                let created = self.settle(request, mode, sent)?;
                Ok(Answer::created(&created, mode))
            }
            WriteOperation::RerunJob => {
                let job = self.rerun_job(&request.target, mode).await?;
                let sent = run_raw(api.create(&post_params(mode), &job)).await;
                let created = self.settle(request, mode, sent)?;
                Ok(Answer::created(&created, mode))
            }
            WriteOperation::ReplaceObject(edit) => {
                let replacement = self.replacement(edit, mode).await?;
                let sent = run_raw(api.replace(name, &post_params(mode), &replacement.body)).await;
                let response = self.settle(request, mode, sent)?;
                let uid = committed_uid(&response, mode);
                let response =
                    serde_json::to_value(&response).map_err(|_| self.unusable_object(mode))?;
                let preview =
                    build_preview(edit, replacement.fresh, response, &replacement.restored)
                        .map_err(|message| self.unusable_response(mode, message))?;
                Ok(Answer {
                    effect: WriteEffect::Replaced(preview),
                    created_name: None,
                    uid,
                    dropped_fields: Vec::new(),
                })
            }
            WriteOperation::DeleteObject { uid, propagation } => {
                let params = DeleteParams {
                    dry_run: mode == WriteMode::DryRun,
                    grace_period_seconds: None,
                    propagation_policy: Some(propagation.policy()),
                    preconditions: Some(Preconditions {
                        resource_version: None,
                        uid: Some(uid.clone()),
                    }),
                };
                let sent = run_raw(api.delete(name, &params)).await;
                let response = self.settle(request, mode, sent)?;
                Ok(Answer::deleted(response.left()))
            }
            WriteOperation::AddDebugContainer {
                name: container,
                image,
                target_container,
            } => {
                let pods = self.pod_api(&request.target, mode)?;
                let body = debug_container_patch(container, image, target_container);
                let sent = run_raw(pods.patch_ephemeral_containers(
                    name,
                    &params,
                    &Patch::Strategic(&body),
                ))
                .await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::CreateNodeShellPod {
                node,
                image,
                user,
                instance,
            } => {
                let pods = self.pod_api(&request.target, mode)?;
                let body = node_shell_pod(&NodeShellPod {
                    namespace: request.target.namespace().unwrap_or_default(),
                    name,
                    node,
                    image,
                    user: user.as_deref(),
                    instance,
                });
                let pod: Pod =
                    serde_json::from_value(body).map_err(|_| self.unusable_object(mode))?;
                let sent = run_raw(pods.create(&post_params(mode), &pod)).await;
                let created = self.settle(request, mode, sent)?;
                let is_commit = mode == WriteMode::Commit;
                Ok(Answer {
                    effect: WriteEffect::Created,
                    created_name: created.metadata.name.clone().filter(|_| is_commit),
                    uid: created.metadata.uid.clone().filter(|_| is_commit),
                    dropped_fields: Vec::new(),
                })
            }
            WriteOperation::EvictPod { uid, grace } => {
                let pods = self.pod_api(&request.target, mode)?;
                let namespace = request.target.namespace().unwrap_or_default();
                let body = eviction_body(namespace, name, uid, *grace);
                let sent = run_raw(pods.create_subresource::<Value, Status>(
                    "eviction",
                    name,
                    &post_params(mode),
                    &body,
                ))
                .await;
                let status = self.settle(request, mode, sent)?;
                // The API server can answer HTTP 201 with a `Failure` body (a pod matched by two
                // budgets), so the body's own status is checked.
                if !status.is_success() {
                    return Err(self.eviction_failure(request, mode, status));
                }
                Ok(Answer::of(WriteEffect::Created))
            }
            WriteOperation::SetNodeTaints {
                taints,
                resource_version,
                ..
            } => {
                let body = taints_patch(taints, resource_version);
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::SetNodeLabels { changes } => {
                let body = labels_patch(changes);
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::SetObjectMetadata {
                labels,
                annotations,
            } => {
                let body = metadata_patch(labels, annotations);
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::SetDataValues(edit) => {
                let body = values_patch(edit);
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                // The answer holds every value of a Secret: it is dropped at once.
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::CreateObject(draft) => {
                // `api` addresses the collection of the target's kind, never a path from the text.
                let body = serde_json::from_value::<DynamicObject>(draft.body().clone())
                    .map_err(|_| self.unusable_object(mode))?;
                let sent = run_raw(api.create(&post_params(mode), &body)).await;
                let created = self.settle(request, mode, sent)?;
                let answer =
                    serde_json::to_value(&created).map_err(|_| self.unusable_object(mode))?;
                // The answer holds the data of a ConfigMap: only paths are read, then it is dropped.
                let dropped_fields = missing_paths(draft.body(), &answer)
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                Ok(Answer {
                    dropped_fields,
                    ..Answer::created(&created, mode)
                })
            }
            WriteOperation::DeleteNodeShellPod { uid } => {
                let pods = self.pod_api(&request.target, mode)?;
                let delete = DeleteParams {
                    grace_period_seconds: Some(0),
                    preconditions: Some(Preconditions {
                        uid: Some(uid.clone()),
                        resource_version: None,
                    }),
                    ..DeleteParams::default()
                };
                let sent = run_raw(pods.delete(name, &delete)).await;
                self.settle(request, mode, sent)?;
                Ok(Answer {
                    effect: WriteEffect::Deleted,
                    created_name: None,
                    uid: None,
                    dropped_fields: Vec::new(),
                })
            }
            WriteOperation::SetHpaReplicaRange {
                min,
                max,
                previous_min,
                previous_max,
            } => {
                // A field the HPA has already is left out, so the patch changes no more than the
                // confirm listed.
                let mut spec = serde_json::Map::new();
                if min != previous_min {
                    spec.insert("minReplicas".to_owned(), json!(min));
                }
                if max != previous_max {
                    spec.insert("maxReplicas".to_owned(), json!(max));
                }
                let body = json!({ "spec": spec });
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::ExpandClaim { storage } => {
                let body =
                    json!({ "spec": { "resources": { "requests": { "storage": storage } } } });
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::SetReclaimPolicy { policy, .. } => {
                let body = reclaim_policy_patch(*policy);
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::RecreateClaim {
                uid, storage_class, ..
            } => {
                self.recreate_claim(request, &api, (uid, storage_class), mode)
                    .await
            }
            WriteOperation::SetDefaultStorageClass { is_default } => {
                // An unset removes the beta key (`null` in a merge patch), which 0014 also reads.
                let annotations = if *is_default {
                    json!({ DEFAULT_CLASS_ANNOTATION: "true" })
                } else {
                    json!({ DEFAULT_CLASS_ANNOTATION: "false", DEFAULT_CLASS_BETA_ANNOTATION: null })
                };
                let body = json!({ "metadata": { "annotations": annotations } });
                let sent = run_raw(api.patch(name, &params, &Patch::Merge(&body))).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
            WriteOperation::RenewCertificate { requested_at } => {
                let object = self
                    .renewal_status(&request.target, *requested_at, mode)
                    .await?;
                let sent = run_raw(api.replace_status(name, &post_params(mode), &object)).await;
                self.settle(request, mode, sent)?;
                Ok(Answer::patched())
            }
        }
    }

    /// The typed pod API of a pod target: the ephemeral container patch and the typed create need it.
    fn pod_api(&self, target: &ObjectRef, mode: WriteMode) -> Result<Api<Pod>, WriteError> {
        let namespace = target
            .namespace()
            .ok_or_else(|| self.unusable_object(mode))?;
        Ok(Api::namespaced(self.client().clone(), namespace))
    }

    /// The outcome of one request: its object, or the `WriteError` the failure maps to.
    fn settle<T>(
        &self,
        request: &WriteRequest,
        mode: WriteMode,
        sent: Result<Result<T, kube::Error>, Elapsed>,
    ) -> Result<T, WriteError> {
        match sent {
            Ok(Ok(object)) => Ok(object),
            Ok(Err(error)) => Err(self.write_error(request, mode, error)),
            Err(_elapsed) => Err(self.timed_out(mode)),
        }
    }

    /// Recreate with another class: reads the claim, deletes it pinned to its uid, waits until it
    /// is gone, and creates the new one. A dry-run stops after the dry-run of the delete: the
    /// create would meet the old claim and read as a name clash. A create that fails after the
    /// delete says so, since the claim is gone by then.
    #[allow(clippy::disallowed_methods)]
    async fn recreate_claim(
        &self,
        request: &WriteRequest,
        api: &Api<DynamicObject>,
        (uid, storage_class): (&str, &str),
        mode: WriteMode,
    ) -> Result<Answer, WriteError> {
        let name = request.target.name();
        let fresh = self.get_object(&request.target, READ_ACTION).await?;
        let body =
            recreated_claim_body(&fresh, uid, storage_class).map_err(|refusal| match refusal {
                RecreateRefusal::Replaced => invalid("the claim was replaced since it was read"),
                RecreateRefusal::Bound => {
                    invalid("only a claim that no volume is bound to can be recreated")
                }
                RecreateRefusal::Terminating => invalid("the claim is already being deleted"),
                RecreateRefusal::Unreadable => self.unusable_object(mode),
            })?;
        let delete = DeleteParams {
            dry_run: mode == WriteMode::DryRun,
            grace_period_seconds: None,
            propagation_policy: Some(PropagationPolicy::Background),
            preconditions: Some(Preconditions {
                resource_version: None,
                uid: Some(uid.to_owned()),
            }),
        };
        let sent = run_raw(api.delete(name, &delete)).await;
        self.settle(request, mode, sent)?;
        if mode == WriteMode::DryRun {
            return Ok(Answer::of(WriteEffect::Created));
        }
        let after_delete = |error: WriteError| {
            invalid(format!(
                "claim {name} was deleted, but creating it again failed: {error}"
            ))
        };
        self.wait_until_gone(api, name, uid)
            .await
            .map_err(after_delete)?;
        let body = serde_json::from_value::<DynamicObject>(body)
            .map_err(|_| after_delete(self.unusable_object(mode)))?;
        let sent = run_raw(api.create(&post_params(mode), &body)).await;
        let created = self.settle(request, mode, sent).map_err(after_delete)?;
        // The new claim has the name of the old one, so there is no created name to report.
        Ok(Answer {
            effect: WriteEffect::Created,
            created_name: None,
            uid: committed_uid(&created, mode),
            dropped_fields: Vec::new(),
        })
    }

    /// Polls until no claim with `uid` is left under `name`: a finalizer can hold a deleted claim.
    async fn wait_until_gone(
        &self,
        api: &Api<DynamicObject>,
        name: &str,
        uid: &str,
    ) -> Result<(), WriteError> {
        for _ in 0..GONE_POLLS {
            let found = self.run(READ_ACTION, api.get_opt(name)).await?;
            if found.is_none_or(|object| object.metadata.uid.as_deref() != Some(uid)) {
                return Ok(());
            }
            tokio::time::sleep(GONE_POLL).await;
        }
        Err(invalid(
            "the old claim is still being deleted, so the new one was not created",
        ))
    }

    /// The JSON Patch of a Roll back (`kubectl rollout undo --to-revision` parity): reads the
    /// Deployment and the ReplicaSet, and refuses a ReplicaSet that is not this Deployment's, not
    /// that revision, or the same template as now. The template stays inside this call.
    async fn roll_back_operations(
        &self,
        target: &ObjectRef,
        replica_set: &str,
        revision: u64,
        mode: WriteMode,
    ) -> Result<Value, WriteError> {
        let deployment = self.get_object(target, READ_ACTION).await?;
        let replica_set = ObjectRef::new(
            ObjectKind::ReplicaSet,
            target.namespace().map(str::to_owned),
            replica_set.to_owned(),
        )
        .ok_or_else(|| self.unusable_object(mode))?;
        let replica_set = self.get_object(&replica_set, READ_ACTION).await?;
        match rollback_operations(&deployment, &replica_set, revision) {
            Ok(operations) => Ok(operations),
            Err(RollBackRefusal::Unreadable) => Err(self.unusable_object(mode)),
            Err(RollBackRefusal::ForeignReplicaSet) => Err(WriteError::NotFound),
            Err(RollBackRefusal::SameTemplate) => Err(WriteError::Invalid {
                message: format!("revision {revision} has the same template as the current one"),
                fields: Vec::new(),
            }),
        }
    }

    /// The Certificate to `PUT` to `/status` for a Renew now: a fresh GET (a 404 is `NotFound`),
    /// refused when it is already issuing or being deleted, with the `Issuing` condition added.
    async fn renewal_status(
        &self,
        target: &ObjectRef,
        requested_at: jiff::Timestamp,
        mode: WriteMode,
    ) -> Result<DynamicObject, WriteError> {
        let fresh = match self.get_object(target, READ_ACTION).await {
            Err(ClusterError::Api { code: 404, .. }) => return Err(WriteError::NotFound),
            read => read?,
        };
        let body = renewal_status_body(fresh, requested_at).map_err(|refusal| match refusal {
            RenewRefusal::AlreadyIssuing => WriteError::Invalid {
                message: "the certificate is already being issued".to_owned(),
                fields: vec!["status.conditions[Issuing]".to_owned()],
            },
            RenewRefusal::Deleting => WriteError::Invalid {
                message: "the certificate is being deleted".to_owned(),
                fields: Vec::new(),
            },
            RenewRefusal::Unreadable => self.unusable_object(mode),
        })?;
        serde_json::from_value(body).map_err(|_| self.unusable_object(mode))
    }

    /// The Job to create from a CronJob, and where to create it.
    async fn trigger_job(
        &self,
        target: &ObjectRef,
        mode: WriteMode,
    ) -> Result<(ObjectRef, DynamicObject), WriteError> {
        let cron_job = self.get_object(target, READ_ACTION).await?;
        let jobs = ObjectRef::new(
            ObjectKind::Job,
            target.namespace().map(str::to_owned),
            target.name().to_owned(),
        );
        let job = trigger_job_body(&cron_job, target.namespace(), target.name())
            .and_then(|body| serde_json::from_value(body).ok());
        match (jobs, job) {
            (Some(jobs), Some(job)) => Ok((jobs, job)),
            _ => Err(self.unusable_object(mode)),
        }
    }

    /// The Job to create as a copy of `target`.
    async fn rerun_job(
        &self,
        target: &ObjectRef,
        mode: WriteMode,
    ) -> Result<DynamicObject, WriteError> {
        let job = self.get_object(target, READ_ACTION).await?;
        rerun_job_body(&job, target.namespace(), target.name())
            .and_then(|body| serde_json::from_value(body).ok())
            .ok_or_else(|| self.unusable_object(mode))
    }

    /// The body of an Edit YAML replace (0031 write path): a fresh GET, the base
    /// `resourceVersion` check, the placeholders restored from it, and the base identity stamped.
    async fn replacement(
        &self,
        edit: &ObjectEdit,
        mode: WriteMode,
    ) -> Result<Replacement, WriteError> {
        let fresh = self.get_object(edit.target(), READ_ACTION).await?;
        // A Helm release record is edited through 0038 only, whatever the editor was opened on.
        if is_helm_release(&fresh) {
            return Err(self.unusable_object(mode));
        }
        let fresh_version = fresh
            .pointer("/metadata/resourceVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| self.unusable_object(mode))?;
        let base_version = edit.base_resource_version();
        if fresh_version != base_version {
            return Err(stale_object(base_version, fresh_version));
        }
        let mut body = edit.edited().clone();
        // Cannot fail after the version check (0031 decision 7); the same answer if it does.
        let restored = edit_placeholders::restore(&mut body, &fresh)
            .map_err(|_unmatched| stale_object(base_version, fresh_version))?;
        let metadata = body
            .get_mut("metadata")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| self.unusable_object(mode))?;
        metadata.insert("resourceVersion".to_owned(), Value::from(base_version));
        metadata.insert("uid".to_owned(), Value::from(edit.base_uid()));
        let body = serde_json::from_value(body).map_err(|_| self.unusable_object(mode))?;
        Ok(Replacement {
            body,
            fresh,
            restored,
        })
    }

    /// An eviction answered with a body that is not a `Success`: sorted from the body's own code
    /// and message like an HTTP error of that code.
    fn eviction_failure(
        &self,
        request: &WriteRequest,
        mode: WriteMode,
        status: Status,
    ) -> WriteError {
        if status.code == 0 {
            // The server answered 2xx but did not say whether it evicted: a commit may have landed,
            // and a repeat is safe because the eviction carries the uid precondition.
            return match mode {
                WriteMode::Commit => WriteError::OutcomeUnknown,
                WriteMode::DryRun => self.unusable_response(mode, UNUSABLE_EVICTION_ANSWER),
            };
        }
        error_from_status(self.context(), mode, request.target.kind_name(), status)
    }

    /// An answer or an object that cannot be used for the change; nothing was sent.
    fn unusable_object(&self, mode: WriteMode) -> WriteError {
        self.unusable_response(mode, UNUSABLE_OBJECT)
    }

    fn unusable_response(&self, mode: WriteMode, message: &'static str) -> WriteError {
        WriteError::Cluster(self.unexpected_response(action_of(mode), message))
    }

    fn timed_out(&self, mode: WriteMode) -> WriteError {
        match mode {
            WriteMode::Commit => WriteError::OutcomeUnknown,
            WriteMode::DryRun => WriteError::Cluster(ClusterError::TimedOut {
                context: self.context().to_owned(),
                action: action_of(mode),
            }),
        }
    }

    fn write_error(
        &self,
        request: &WriteRequest,
        mode: WriteMode,
        error: kube::Error,
    ) -> WriteError {
        match error {
            kube::Error::Api(status) => {
                if status.code == 422
                    && let Some(error) = unprocessable(&request.operation, &status)
                {
                    return error;
                }
                if let Some(error) = create_failure(request, &status) {
                    return error;
                }
                error_from_status(self.context(), mode, request.target.kind_name(), *status)
            }
            // These fail while the request is built, before anything is sent.
            built @ (kube::Error::BuildRequest(_) | kube::Error::HttpError(_)) => {
                WriteError::Cluster(classify_error(self.context(), action_of(mode), built))
            }
            // Any other failure of a commit can come after the request left the client, so the
            // change might have been applied.
            _ if mode == WriteMode::Commit => WriteError::OutcomeUnknown,
            other => WriteError::Cluster(classify_error(self.context(), action_of(mode), other)),
        }
    }
}

/// `PUT` answered the object changed while the editor was open.
fn stale_object(base_version: &str, fresh_version: &str) -> WriteError {
    WriteError::Conflict {
        message: format!(
            "the object changed since it was opened (resourceVersion {base_version} \u{2192} {fresh_version})"
        ),
        managers: Vec::new(),
    }
}

/// The operations whose 422 does not read as a plain `Invalid`: a failed `test` op of a Roll back
/// means the Deployment was replaced, and the server text of a generated Job can quote template
/// values such as env literals, so only its field paths are kept.
fn unprocessable(operation: &WriteOperation, status: &Status) -> Option<WriteError> {
    match operation {
        // A failed `test` op reads like another 422 reason; a template the server finds invalid
        // is `Invalid`, with field paths only (the text could quote env literals).
        WriteOperation::RollBackDeployment { .. } if status.reason != "Invalid" => {
            Some(WriteError::Conflict {
                message: STALE_TEMPLATE.to_owned(),
                managers: Vec::new(),
            })
        }
        WriteOperation::RollBackDeployment { .. } => Some(WriteError::Invalid {
            message: REJECTED_TEMPLATE.to_owned(),
            fields: cause_fields(status),
        }),
        WriteOperation::TriggerCronJob | WriteOperation::RerunJob => Some(WriteError::Invalid {
            message: REJECTED_OBJECT.to_owned(),
            fields: cause_fields(status),
        }),
        _ => None,
    }
}

/// The two answers of a create that read wrong as the generic mapping (0042 decision 6): a 409
/// `AlreadyExists` is not "changed since it was read", and the 404 of a POST means its namespace
/// is missing, not that "the object no longer exists".
fn create_failure(request: &WriteRequest, status: &Status) -> Option<WriteError> {
    if !matches!(request.operation, WriteOperation::CreateObject(_)) {
        return None;
    }
    let target = &request.target;
    // A Namespace has no namespace in its path, so its 404 can only mean the API is missing.
    if status.code == 404 && target.namespace().is_none() {
        return Some(WriteError::Invalid {
            message: format!(
                "The {} API is not available on this cluster",
                target.kind_name()
            ),
            fields: Vec::new(),
        });
    }
    match (status.code, target.namespace()) {
        (409, _) if status.reason == "AlreadyExists" => Some(WriteError::Invalid {
            message: format!("{} {} already exists", target.kind_name(), target.name()),
            fields: vec!["metadata.name".to_owned()],
        }),
        (404, Some(namespace)) => Some(WriteError::Invalid {
            message: format!("The namespace {namespace} does not exist"),
            fields: vec!["metadata.namespace".to_owned()],
        }),
        _ => None,
    }
}

/// `fieldManager=k8sboard` in both modes; `DryRun` adds `dryRun=All`.
fn invalid(message: impl Into<String>) -> WriteError {
    WriteError::Invalid {
        message: message.into(),
        fields: Vec::new(),
    }
}

fn patch_params(mode: WriteMode) -> PatchParams {
    PatchParams {
        dry_run: mode == WriteMode::DryRun,
        field_manager: Some(FIELD_MANAGER.to_owned()),
        ..PatchParams::default()
    }
}

/// The same query for the creates and the replace: `fieldManager=k8sboard`, plus `dryRun=All`.
fn post_params(mode: WriteMode) -> PostParams {
    PostParams {
        dry_run: mode == WriteMode::DryRun,
        field_manager: Some(FIELD_MANAGER.to_owned()),
    }
}

fn action_of(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::DryRun => "checking a change with a dry-run",
        WriteMode::Commit => "applying a change",
    }
}

fn map_status(context: &str, mode: WriteMode, status: Status) -> WriteError {
    let message = status.message.clone();
    match status.code {
        403 if message.contains("is forbidden: User") => WriteError::Denied { message },
        // An admission plugin or webhook refusal never reads "not permitted".
        403 | 422 => WriteError::Invalid {
            message,
            fields: cause_fields(&status),
        },
        404 => WriteError::NotFound,
        409 => WriteError::Conflict {
            message,
            managers: Vec::new(),
        },
        429 => WriteError::TooManyRequests {
            message: first_cause_message(&status).unwrap_or(message),
            retry_after: status
                .details
                .as_ref()
                .map(|details| details.retry_after_seconds)
                .filter(|seconds| *seconds > 0)
                .map(|seconds| Duration::from_secs(u64::from(seconds))),
        },
        400 if mode == WriteMode::DryRun && is_dry_run_unsupported(&message) => {
            WriteError::DryRunRejected { reason: message }
        }
        _ => WriteError::Cluster(classify_error(
            context,
            action_of(mode),
            kube::Error::Api(Box::new(status)),
        )),
    }
}

/// The first non-empty `details.causes[].message`: for a refused eviction it names the budget.
fn first_cause_message(status: &Status) -> Option<String> {
    status
        .details
        .iter()
        .flat_map(|details| &details.causes)
        .map(|cause| cause.message.clone())
        .find(|message| !message.is_empty())
}

/// The field paths of the causes, `details.causes[].field`.
fn cause_fields(status: &Status) -> Vec<String> {
    status
        .details
        .iter()
        .flat_map(|details| &details.causes)
        .map(|cause| cause.field.clone())
        .filter(|field| !field.is_empty())
        .collect()
}

/// A webhook that cannot honor `dryRun` answers 400 with a message naming dry-run support.
fn is_dry_run_unsupported(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase().replace("dry-run", "dry run");
    lowered.contains("does not support dry run")
}

/// Sorts a server `Status` into a `WriteError` by its raw code and text, then redacts the result
/// (decision 29): classification never depends on what redaction removes.
fn error_from_status(
    context: &str,
    mode: WriteMode,
    kind_name: &str,
    status: Status,
) -> WriteError {
    let reason = status.reason.clone();
    let fields = cause_fields(&status);
    let error = map_status(context, mode, status);
    redact_error(kind_name, error, &reason, &fields)
}

/// For a Secret target every server message of `error` becomes the status `reason` code plus the
/// field paths, because the text can quote values. Every variant that carries server text is
/// listed, so a new one cannot be forgotten.
fn redact_error(
    kind_name: &str,
    mut error: WriteError,
    reason: &str,
    fields: &[String],
) -> WriteError {
    let redact =
        |message: &mut String| *message = redact_message(kind_name, message, reason, fields);
    match &mut error {
        WriteError::Denied { message }
        | WriteError::Conflict { message, .. }
        | WriteError::Invalid { message, .. }
        | WriteError::TooManyRequests { message, .. } => redact(message),
        WriteError::DryRunRejected { reason: text } => redact(text),
        WriteError::Cluster(
            ClusterError::Api { message, .. }
            | ClusterError::Forbidden { message, .. }
            | ClusterError::Unauthorized { message, .. },
        ) => redact(message),
        WriteError::WritesBlocked
        | WriteError::NotFound
        | WriteError::OutcomeUnknown
        | WriteError::Cluster(_) => {}
    }
    error
}

/// The text a user may see for a server message about an object of `kind_name`. Unchanged unless
/// the kind is `Secret`, where it becomes the status `reason` code plus the field paths.
fn redact_message(kind_name: &str, message: &str, reason: &str, fields: &[String]) -> String {
    if kind_name != KIND_SECRET {
        return message.to_owned();
    }
    let reason = if reason.is_empty() { "Failure" } else { reason };
    if fields.is_empty() {
        reason.to_owned()
    } else {
        format!("{reason}: {}", fields.join(", "))
    }
}

// The write tests drive `write` directly; the clippy ban on `ClusterConnection::write` is for the
// app (spec 0030 AC 9), so each of these modules allows it.
#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_tests.rs"]
mod object_write_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_workload_tests.rs"]
mod object_write_workload_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_replace_tests.rs"]
mod object_write_replace_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_delete_tests.rs"]
mod object_write_delete_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_debug_tests.rs"]
mod object_write_debug_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_node_tests.rs"]
mod object_write_node_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_resource_edit_tests.rs"]
mod object_write_resource_edit_tests;
#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_volume_tests.rs"]
mod object_write_volume_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_values_tests.rs"]
mod object_write_values_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_create_tests.rs"]
mod object_write_create_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_certificate_tests.rs"]
mod object_write_certificate_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "object_write_metadata_tests.rs"]
mod object_write_metadata_tests;
