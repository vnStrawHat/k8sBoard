//! The one write path (spec 0030). Every mutating request goes through
//! `ClusterConnection::write`; the `WriteOperation` enum is the allow-list, and clippy
//! `disallowed-methods` keeps every other call site out of the crate.
//!
//! Nothing here logs or keeps a request body or a field value.

use std::borrow::Cow;
use std::fmt;
use std::time::{Duration, Instant};

use kube::api::{
    DeleteParams, DynamicObject, Patch, PatchParams, PostParams, Preconditions, PropagationPolicy,
};
use kube::core::Status;
use serde_json::{Value, json};
use tokio::time::error::Elapsed;

use crate::access_review::AccessCheck;
use crate::connection::{ClusterConnection, ClusterError, classify_error, run_raw};
use crate::dns_name::{is_dns_subdomain, is_path_segment_name};
use crate::edit_placeholders::{self, Restored};
use crate::edit_preview::{EditPreview, build_preview};
use crate::object_edit::{ObjectEdit, is_helm_release};
use crate::object_yaml::{ObjectKind, ObjectRef};
use crate::workload_write_bodies::{
    RERUN_BASE_CHARS, RERUN_SUFFIX, RollBackRefusal, TRIGGER_BASE_CHARS, TRIGGER_SUFFIX,
    generate_name, rerun_job_body, rollback_operations, trigger_job_body,
};

/// The server-side field manager of every write k8sBoard sends.
const FIELD_MANAGER: &str = "k8sboard";
/// Debug builds send no write unless this variable is `1`; agent runs never set it.
pub(crate) const ALLOW_WRITES_VARIABLE: &str = "K8SBOARD_ALLOW_WRITES";
const KIND_SECRET: &str = "Secret";
/// What an operation reads before it sends, so the request can carry the current object's data.
const READ_ACTION: &str = "reading the object before the change";
/// The annotation `kubectl rollout restart` sets (kube's own `Api::restart` writes another one).
const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";
/// Fixed text: a library error could quote the object's content.
const UNUSABLE_OBJECT: &str = "the object could not be used for this change";
const STALE_TEMPLATE: &str = "the deployment was replaced since it was read";
const REJECTED_OBJECT: &str = "the server rejected the generated object";
const REJECTED_TEMPLATE: &str = "the server rejected the template of that revision";
/// A `replicas` field is an `int32` on the server.
const MAX_REPLICAS: u32 = i32::MAX as u32;

/// One allow-listed mutation. Adding a variant is the only way to add a write (C3).
// Debug is manual: the variant name only.
#[derive(Clone, PartialEq, Eq)]
pub enum WriteOperation {
    /// JSON merge patch `{"spec":{"unschedulable": !schedulable}}` on a Node (cordon or uncordon).
    SetNodeSchedulable { schedulable: bool },
    /// Merge patch of the `scale` subresource of a Deployment or StatefulSet (0032).
    ScaleWorkload { replicas: u32 },
    /// Merge patch of the pod template's `kubectl.kubernetes.io/restartedAt` annotation on a
    /// Deployment, StatefulSet, or DaemonSet (0032). Rounded to whole seconds on the wire, so a
    /// dry-run and its commit send the same body.
    RestartRollout { restarted_at: jiff::Timestamp },
    /// Merge patch `{"spec":{"paused": b}}` on a Deployment (0032).
    SetRolloutPaused { paused: bool },
    /// JSON Patch that replaces a Deployment's pod template with one of its ReplicaSets
    /// (`kubectl rollout undo --to-revision`, 0032).
    RollBackDeployment { replica_set: String, revision: u64 },
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
}

impl WriteOperation {
    fn name(&self) -> &'static str {
        match self {
            Self::SetNodeSchedulable { .. } => "SetNodeSchedulable",
            Self::ScaleWorkload { .. } => "ScaleWorkload",
            Self::RestartRollout { .. } => "RestartRollout",
            Self::SetRolloutPaused { .. } => "SetRolloutPaused",
            Self::RollBackDeployment { .. } => "RollBackDeployment",
            Self::SetCronJobSuspended { .. } => "SetCronJobSuspended",
            Self::TriggerCronJob => "TriggerCronJob",
            Self::RerunJob => "RerunJob",
            Self::ReplaceObject(_) => "ReplaceObject",
            Self::DeleteObject { .. } => "DeleteObject",
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
}

/// One field a write changes, for the confirm summary and the audit line. `None` means the value
/// is not recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedField {
    pub path: Cow<'static, str>,
    pub value: Option<String>,
}

/// Whether a connection may send writes at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritePolicy {
    Allowed,
    Blocked,
}

impl WritePolicy {
    /// Release builds allow writes. Debug builds block them unless `opt_in` is `Some("1")`, the
    /// value of `K8SBOARD_ALLOW_WRITES`.
    pub fn resolve(is_debug_build: bool, opt_in: Option<&str>) -> Self {
        if !is_debug_build || opt_in == Some("1") {
            Self::Allowed
        } else {
            Self::Blocked
        }
    }

    /// The policy of this build. A build that `blocks_writes` (the screenshot build, through the
    /// `block-writes` feature) never writes, whatever the variable says; any other build follows
    /// `resolve`.
    pub(crate) fn of_build(
        blocks_writes: bool,
        is_debug_build: bool,
        opt_in: Option<&str>,
    ) -> Self {
        if blocks_writes {
            return Self::Blocked;
        }
        Self::resolve(is_debug_build, opt_in)
    }
}

impl WriteRequest {
    /// `None` when the target kind does not fit the operation, or when its name or namespace is
    /// not a DNS-1123 subdomain: kube does not encode them, so any other text could change the
    /// path the request goes to. The RBAC kinds of `ReplaceObject` and `DeleteObject` may use `:` in a name, so they
    /// follow the path-segment rule instead.
    pub fn new(target: ObjectRef, operation: WriteOperation) -> Option<Self> {
        // `replicas` is an int32: a larger number can only be a typo, and the server would refuse it.
        if matches!(operation, WriteOperation::ScaleWorkload { replicas } if replicas > MAX_REPLICAS)
        {
            return None;
        }
        // Without a uid the delete could hit a recreated object of the same name.
        if matches!(&operation, WriteOperation::DeleteObject { uid, .. } if uid.is_empty()) {
            return None;
        }
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
        };
        match &self.operation {
            WriteOperation::SetNodeSchedulable { schedulable } => {
                vec![field("spec.unschedulable", (!schedulable).to_string())]
            }
            WriteOperation::ScaleWorkload { replicas } => {
                vec![field("spec.replicas", replicas.to_string())]
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
                })
                .collect(),
            WriteOperation::DeleteObject { propagation, .. } => {
                vec![field(
                    "deleteOptions.propagationPolicy",
                    propagation.as_str().to_owned(),
                )]
            }
        }
    }

    /// False only for operations the server cannot dry-run; true for every one of 0030-0032.
    pub fn supports_dry_run(&self) -> bool {
        match self.operation {
            WriteOperation::SetNodeSchedulable { .. }
            | WriteOperation::ScaleWorkload { .. }
            | WriteOperation::RestartRollout { .. }
            | WriteOperation::SetRolloutPaused { .. }
            | WriteOperation::RollBackDeployment { .. }
            | WriteOperation::SetCronJobSuspended { .. }
            | WriteOperation::TriggerCronJob
            | WriteOperation::RerunJob
            | WriteOperation::ReplaceObject(_)
            | WriteOperation::DeleteObject { .. } => true,
        }
    }
}

/// The permission `operation` needs on `target`, or `None` when the kind does not fit.
fn fitting_access_check(target: &ObjectRef, operation: &WriteOperation) -> Option<AccessCheck> {
    let kind = target.builtin_kind()?;
    Some(match (operation, kind) {
        (WriteOperation::SetNodeSchedulable { .. }, ObjectKind::Node) => AccessCheck::PatchNodes,
        (WriteOperation::ScaleWorkload { .. }, ObjectKind::Deployment) => {
            AccessCheck::PatchDeploymentScale
        }
        (WriteOperation::ScaleWorkload { .. }, ObjectKind::StatefulSet) => {
            AccessCheck::PatchStatefulSetScale
        }
        (WriteOperation::RestartRollout { .. }, ObjectKind::Deployment) => {
            AccessCheck::PatchDeployments
        }
        (WriteOperation::RestartRollout { .. }, ObjectKind::StatefulSet) => {
            AccessCheck::PatchStatefulSets
        }
        (WriteOperation::RestartRollout { .. }, ObjectKind::DaemonSet) => {
            AccessCheck::PatchDaemonSets
        }
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
        _ => is_dns_subdomain(target.name()),
    };
    let is_safe_replica_set = match operation {
        WriteOperation::RollBackDeployment { replica_set, .. } => is_dns_subdomain(replica_set),
        _ => true,
    };
    is_safe_name && is_safe_replica_set && target.namespace().is_none_or(is_dns_subdomain)
}

/// Whole seconds, UTC, like kubectl writes it.
fn restart_stamp(timestamp: &jiff::Timestamp) -> String {
    timestamp.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// A failed write, sorted by what the user can do about it.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("writes are blocked in this debug build (set K8SBOARD_ALLOW_WRITES=1)")]
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
            WriteOperation::ScaleWorkload { replicas } => {
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
        }
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

/// `fieldManager=k8sboard` in both modes; `DryRun` adds `dryRun=All`.
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
            message,
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

#[cfg(test)]
#[path = "object_write_tests.rs"]
mod object_write_tests;

#[cfg(test)]
#[path = "object_write_workload_tests.rs"]
mod object_write_workload_tests;

#[cfg(test)]
#[path = "object_write_replace_tests.rs"]
mod object_write_replace_tests;

#[cfg(test)]
#[path = "object_write_delete_tests.rs"]
mod object_write_delete_tests;
