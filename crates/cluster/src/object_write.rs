//! The one write path (spec 0030). Every mutating request goes through
//! `ClusterConnection::write`; the `WriteOperation` enum is the allow-list, and clippy
//! `disallowed-methods` keeps every other call site out of the crate.
//!
//! Nothing here logs or keeps a request body or a field value.

use std::fmt;
use std::time::{Duration, Instant};

use kube::api::{DynamicObject, Patch, PatchParams};
use kube::core::Status;
use serde_json::json;
use tokio::time::error::Elapsed;

use crate::access_review::AccessCheck;
use crate::connection::{ClusterConnection, ClusterError, REQUEST_TIMEOUT, classify_error};
use crate::dns_name::is_dns_subdomain;
use crate::object_yaml::{ObjectKind, ObjectRef};

/// The server-side field manager of every write k8sBoard sends.
const FIELD_MANAGER: &str = "k8sboard";
/// Debug builds send no write unless this variable is `1`; agent runs never set it.
pub(crate) const ALLOW_WRITES_VARIABLE: &str = "K8SBOARD_ALLOW_WRITES";
const KIND_SECRET: &str = "Secret";

/// One allow-listed mutation. Adding a variant is the only way to add a write (C3).
// Debug is manual: the variant name only.
#[derive(Clone, PartialEq, Eq)]
pub enum WriteOperation {
    /// JSON merge patch `{"spec":{"unschedulable": !schedulable}}` on a Node (cordon or uncordon).
    SetNodeSchedulable { schedulable: bool },
}

impl fmt::Debug for WriteOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetNodeSchedulable { .. } => formatter.write_str("SetNodeSchedulable"),
        }
    }
}

/// A write whose target kind fits its operation. Always valid.
// Debug is manual: operation, kind, namespace, name; never a body or a field value.
#[derive(Clone, PartialEq, Eq)]
pub struct WriteRequest {
    target: ObjectRef,
    operation: WriteOperation,
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
    pub path: &'static str,
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
    /// path the request goes to.
    pub fn new(target: ObjectRef, operation: WriteOperation) -> Option<Self> {
        let is_fitting = match operation {
            WriteOperation::SetNodeSchedulable { .. } => {
                target.builtin_kind() == Some(ObjectKind::Node)
            }
        };
        let is_safe_path =
            is_dns_subdomain(target.name()) && target.namespace().is_none_or(is_dns_subdomain);
        (is_fitting && is_safe_path).then_some(Self { target, operation })
    }

    pub fn target(&self) -> &ObjectRef {
        &self.target
    }

    pub fn operation(&self) -> &WriteOperation {
        &self.operation
    }

    /// The permission the operation needs; the app gate reads the same value.
    pub fn access_check(&self) -> AccessCheck {
        match self.operation {
            WriteOperation::SetNodeSchedulable { .. } => AccessCheck::PatchNodes,
        }
    }

    pub fn changed_fields(&self) -> Vec<ChangedField> {
        match self.operation {
            WriteOperation::SetNodeSchedulable { schedulable } => vec![ChangedField {
                path: "spec.unschedulable",
                value: Some((!schedulable).to_string()),
            }],
        }
    }

    /// False only for operations the server cannot dry-run; true for every one of 0030.
    pub fn supports_dry_run(&self) -> bool {
        match self.operation {
            WriteOperation::SetNodeSchedulable { .. } => true,
        }
    }
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

impl ClusterConnection {
    /// The only function that sends a mutating request. Returns `WritesBlocked` before building any
    /// request when the connection's policy is `Blocked`. A patch sets `fieldManager=k8sboard` in
    /// both modes, and `DryRun` adds `dryRun=All`. The caller passes the connection of the target's
    /// own cluster; there is no implicit current session.
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
            is_ok = matches!(sent, Ok(Ok(_))),
            "write finished"
        );
        match sent {
            Ok(Ok(_object)) => Ok(WriteOutcome {
                mode,
                elapsed,
                effect: WriteEffect::Patched,
                created_name: None,
                uid: None,
            }),
            Ok(Err(error)) => Err(self.write_error(request, mode, error)),
            Err(_elapsed) => Err(self.timed_out(mode)),
        }
    }

    /// Sends the request of one allow-listed operation: one arm per `WriteOperation`, the
    /// object_write.rs row of the 0030 exception table.
    #[allow(clippy::disallowed_methods)]
    async fn send(
        &self,
        request: &WriteRequest,
        mode: WriteMode,
    ) -> Result<Result<DynamicObject, kube::Error>, Elapsed> {
        let api = self.object_api(&request.target);
        let params = patch_params(mode);
        match &request.operation {
            WriteOperation::SetNodeSchedulable { schedulable } => {
                let body = json!({ "spec": { "unschedulable": !schedulable } });
                run_raw(api.patch(request.target.name(), &params, &Patch::Merge(&body))).await
            }
        }
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

/// `fieldManager=k8sboard` in both modes; `DryRun` adds `dryRun=All`.
fn patch_params(mode: WriteMode) -> PatchParams {
    PatchParams {
        dry_run: mode == WriteMode::DryRun,
        field_manager: Some(FIELD_MANAGER.to_owned()),
        ..PatchParams::default()
    }
}

fn action_of(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::DryRun => "checking a change with a dry-run",
        WriteMode::Commit => "applying a change",
    }
}

/// Runs one request under `REQUEST_TIMEOUT`, keeping the timeout apart from the `kube::Error`
/// because `ClusterConnection::run` classifies errors before the `Status` can be read.
async fn run_raw<T>(
    request: impl Future<Output = Result<T, kube::Error>>,
) -> Result<Result<T, kube::Error>, Elapsed> {
    tokio::time::timeout(REQUEST_TIMEOUT, request).await
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
