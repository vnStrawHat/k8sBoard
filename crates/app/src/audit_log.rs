//! The local audit log (spec 0030): one JSON object per line in `<config>/audit.jsonl`, for lock
//! toggles now and for commits later. Pure: it sends nothing to any cluster.
//!
//! A line holds names and paths, never a request body, a token, or the value of a Secret field.

use std::fs::OpenOptions;
use std::future::Future;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{OnceLock, mpsc};
use std::task::{Context, Poll};
use std::thread;

use cluster::GracePeriod;
use futures::channel::oneshot;
use serde::Serialize;

use crate::app_shell::write_flow::WriteIntent;
use crate::drain_plan::{BudgetPolicy, DrainOptions};
use crate::drain_run::{NextStep, NodeSummary, SummaryOutcome};
use crate::resource_actions::{ResourceAction, action_label};
use crate::write_guard::{ClusterGuard, WriteLock};

const AUDIT_FILE: &str = "audit.jsonl";
const NOTE_LIMIT: usize = 500;
/// Kinds whose field values are never recorded: a Secret holds credentials, and a ConfigMap
/// often does too.
const PATH_ONLY_KINDS: [&str; 2] = ["Secret", "ConfigMap"];

/// One line of the log. Holds no secret by construction: the keys are an allow-list.
#[derive(Serialize)]
pub(crate) struct AuditEntry {
    /// RFC 3339, UTC, whole seconds.
    pub(crate) at: String,
    /// The cluster display name.
    pub(crate) cluster: String,
    pub(crate) context: String,
    /// The kubeconfig user entry name, not the identity the server sees.
    pub(crate) user: Option<String>,
    /// The action label (`Cordon`), or `Lock` or `Unlock`.
    pub(crate) action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) object: Option<AuditObject>,
    pub(crate) fields: Vec<AuditField>,
    pub(crate) outcome: AuditOutcome,
    /// The redacted `Display` of the failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct AuditObject {
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) namespace: Option<String>,
    pub(crate) name: String,
}

#[derive(Serialize)]
pub(crate) struct AuditField {
    pub(crate) path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AuditOutcome {
    Applied,
    Failed,
    /// A commit whose request may have left the client before it failed.
    Unknown,
    /// A session start that was closed or replaced before it reported: its request may have
    /// reached the server; or the summary line of the node a drain was working on when the app
    /// quit.
    Abandoned,
    /// The summary line of a node that was drained (spec 0034).
    Drained,
    /// The summary line of a node the drain could not finish: a timeout or a pod that failed.
    Stuck,
    /// The summary line of the node a cancelled drain was working on.
    Cancelled,
    /// The summary line of the node a stopped drain (a lock, a switch, a quit) was working on.
    Stopped,
}

/// The log file inside the settings folder `dir`.
pub(crate) fn audit_path(dir: &Path) -> PathBuf {
    dir.join(AUDIT_FILE)
}

/// A lock or unlock of the guard's cluster: no object, no fields. `lock` is the state the cluster
/// moved to, so `Locked` writes a `Lock` line.
pub(crate) fn lock_entry(guard: &ClusterGuard<'_>, lock: WriteLock) -> AuditEntry {
    let action = match lock {
        WriteLock::Locked => "Lock",
        WriteLock::Unlocked => "Unlock",
    };
    AuditEntry {
        at: timestamp_now(),
        cluster: guard.display_name().to_owned(),
        context: guard.summary.name.clone(),
        user: guard.summary.user.clone(),
        action: action.to_owned(),
        object: None,
        fields: Vec::new(),
        outcome: AuditOutcome::Applied,
        error: None,
        note: None,
    }
}

/// One guarded action (a write commit): the cluster, context, and user come from the guard of
/// `intent.cluster`, never from the primary; the fields are the paths the request changes, and
/// their values only when the kind allows recording them.
pub(crate) fn audit_entry(
    intent: &WriteIntent,
    guard: &ClusterGuard<'_>,
    outcome: AuditOutcome,
    error: Option<String>,
    note: Option<&str>,
) -> AuditEntry {
    let target = intent.request.target();
    let fields = intent
        .request
        .changed_fields()
        .into_iter()
        .map(|field| AuditField {
            path: field.path.into_owned(),
            value: field.value,
        })
        .collect();
    AuditEntry {
        at: timestamp_now(),
        cluster: guard.display_name().to_owned(),
        context: guard.summary.name.clone(),
        user: guard.summary.user.clone(),
        action: audit_action(intent),
        object: Some(AuditObject {
            kind: target.kind_name().to_owned(),
            namespace: target.namespace().map(str::to_owned),
            name: target.name().to_owned(),
        }),
        fields: recordable_fields(target.kind_name(), fields),
        outcome,
        error,
        note: note.and_then(clean_note),
    }
}

/// The action the line records: the button of the dialog (`Cordon`), except for an edit, whose
/// button says `Apply changes` and whose line says what was done: `Edit YAML` or `Edit values`.
fn audit_action(intent: &WriteIntent) -> String {
    match intent.action {
        ResourceAction::EditYaml(_) | ResourceAction::EditValues(_) => {
            action_label(intent.action).to_owned()
        }
        _ => intent.button.to_string(),
    }
}

/// One session start (an exec): the cluster, context, and user come from the guard of the target's
/// own cluster. `fields` name the parameters (container, command), never stream bytes.
pub(crate) fn connect_entry(
    action: &str,
    object: AuditObject,
    fields: Vec<AuditField>,
    guard: &ClusterGuard<'_>,
    outcome: AuditOutcome,
    error: Option<String>,
) -> AuditEntry {
    AuditEntry {
        at: timestamp_now(),
        cluster: guard.display_name().to_owned(),
        context: guard.summary.name.clone(),
        user: guard.summary.user.clone(),
        action: action.to_owned(),
        object: Some(object),
        fields,
        outcome,
        error,
        note: None,
    }
}

/// The cluster, context, and user of a run that writes lines after its session may be gone (a
/// drain). Copied from the guard when the run starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuditIdentity {
    cluster: String,
    context: String,
    user: Option<String>,
}

impl AuditIdentity {
    pub(crate) fn of(guard: &ClusterGuard<'_>) -> Self {
        Self {
            cluster: guard.display_name().to_owned(),
            context: guard.summary.name.clone(),
            user: guard.summary.user.clone(),
        }
    }
}

#[cfg(feature = "screenshot")]
impl AuditIdentity {
    /// The identity of a fixture cluster, for the screens drawn from fixed data.
    pub(crate) fn fixture(cluster: &str) -> Self {
        Self {
            cluster: cluster.to_owned(),
            context: cluster.to_owned(),
            user: None,
        }
    }
}

/// The one summary line of a drained node (spec 0034 decision 39): action `Drain`, the node, the
/// counts as field values, and how the node ended. Written besides the line of every cordon and
/// eviction commit, which `checked_write` writes.
pub(crate) fn drain_summary_entry(
    identity: &AuditIdentity,
    summary: &NodeSummary,
    budgets: BudgetPolicy,
    note: Option<&str>,
) -> AuditEntry {
    let count = |path: &str, value: usize| AuditField {
        path: path.to_owned(),
        value: Some(value.to_string()),
    };
    // A drain that skips the budgets deletes its pods directly: nothing was evicted.
    let removed = match budgets {
        BudgetPolicy::Respect => "evicted",
        BudgetPolicy::Skip => "deleted",
    };
    AuditEntry {
        at: timestamp_now(),
        cluster: identity.cluster.clone(),
        context: identity.context.clone(),
        user: identity.user.clone(),
        action: "Drain".to_owned(),
        object: Some(AuditObject {
            kind: "Node".to_owned(),
            namespace: None,
            name: summary.node.clone(),
        }),
        fields: [
            Some(count(removed, summary.evicted)),
            Some(count("refused", summary.refused)),
            Some(count("failed", summary.failed)),
            Some(count("skipped", summary.skipped)),
            // Only a run that ended with a request in the air has an unknown count.
            (summary.unknown > 0).then(|| count("unknown", summary.unknown)),
            // Only a drain that skipped the budgets says so (kubectl `--disable-eviction`).
            (budgets == BudgetPolicy::Skip).then(|| AuditField {
                path: "disable_eviction".to_owned(),
                value: Some("true".to_owned()),
            }),
        ]
        .into_iter()
        .flatten()
        .collect(),
        outcome: match summary.outcome {
            SummaryOutcome::Drained => AuditOutcome::Drained,
            SummaryOutcome::Stuck => AuditOutcome::Stuck,
            SummaryOutcome::Cancelled => AuditOutcome::Cancelled,
            SummaryOutcome::Stopped => AuditOutcome::Stopped,
            SummaryOutcome::Abandoned => AuditOutcome::Abandoned,
        },
        error: summary.reason.as_ref().map(ToString::to_string),
        note: note.and_then(clean_note),
    }
}

/// The line of a commit whose request was in the air when the app quit: its outcome is unknown,
/// because `checked_write` can no longer write the line when the answer comes. `None` for a step
/// that is not a commit.
pub(crate) fn drain_in_flight_entry(
    identity: &AuditIdentity,
    step: &NextStep,
    options: &DrainOptions,
    note: Option<&str>,
) -> Option<AuditEntry> {
    let (action, object, fields) = match step {
        NextStep::Evict(key) => {
            let pod = AuditObject {
                kind: "Pod".to_owned(),
                namespace: Some(key.namespace.clone()),
                name: key.name.clone(),
            };
            match options.budgets {
                BudgetPolicy::Respect => (
                    "Evict",
                    pod,
                    AuditField {
                        path: "pods/eviction".to_owned(),
                        value: Some(match options.grace {
                            GracePeriod::PodDefault => "grace pod default".to_owned(),
                            GracePeriod::Seconds(seconds) => format!("grace {seconds}s"),
                        }),
                    },
                ),
                BudgetPolicy::Skip => (
                    "Delete",
                    pod,
                    AuditField {
                        path: "deleteOptions.propagationPolicy".to_owned(),
                        value: Some("Background".to_owned()),
                    },
                ),
            }
        }
        NextStep::Cordon(node) => (
            "Cordon",
            AuditObject {
                kind: "Node".to_owned(),
                namespace: None,
                name: node.clone(),
            },
            AuditField {
                path: "spec.unschedulable".to_owned(),
                value: Some("true".to_owned()),
            },
        ),
        _ => return None,
    };
    Some(AuditEntry {
        at: timestamp_now(),
        cluster: identity.cluster.clone(),
        context: identity.context.clone(),
        user: identity.user.clone(),
        action: action.to_owned(),
        object: Some(object),
        fields: vec![fields],
        outcome: AuditOutcome::Unknown,
        error: Some("the app closed while the request was in flight".to_owned()),
        note: note.and_then(clean_note),
    })
}

/// The name the server gave an object a commit created (Trigger now, Re-run), which the request
/// could not name: the log records it beside the `generateName` of the request.
pub(crate) fn created_name_field(name: &str) -> AuditField {
    AuditField {
        path: "metadata.name".to_owned(),
        value: Some(name.to_owned()),
    }
}

/// One serialized line waiting for the writer.
struct Submission {
    dir: PathBuf,
    line: String,
    reply: oneshot::Sender<io::Result<()>>,
}

/// The one channel every audit line goes through, drained by one thread. The Open line of a
/// failed node shell (queued by the main thread) and the Delete line of its pod (queued by the
/// tokio runtime) used to race through separate executors; one queue makes the file order the
/// order in which `submit_audit` was called.
static WRITER: OnceLock<io::Result<mpsc::Sender<Submission>>> = OnceLock::new();

fn writer() -> io::Result<&'static mpsc::Sender<Submission>> {
    let started = WRITER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Submission>();
        thread::Builder::new()
            .name("audit-log".to_owned())
            .spawn(move || {
                for job in receiver {
                    // A caller that stopped waiting still gets its line written.
                    let _ = job.reply.send(write_line(&job.dir, &job.line));
                }
            })
            .map(|_| sender)
    });
    started
        .as_ref()
        .map_err(|error| io::Error::new(error.kind(), "the audit writer did not start"))
}

/// Resolves once the line is on disk, or with the error that stopped it.
pub(crate) struct AuditReceipt(oneshot::Receiver<io::Result<()>>);

impl Future for AuditReceipt {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx).map(|received| {
            received.unwrap_or_else(|_| Err(io::Error::other("the audit writer stopped")))
        })
    }
}

/// Queues `entry` as one line behind every line queued before it, and returns at once: the line
/// is queued when this is called, not when the receipt is first polled, so call order is file
/// order. The entry is serialized here, so the writer thread only does the file work.
pub(crate) fn submit_audit(dir: &Path, entry: &AuditEntry) -> AuditReceipt {
    let (reply, receipt) = oneshot::channel();
    let queued = serde_json::to_string(entry)
        .map_err(io::Error::other)
        .and_then(|mut line| {
            line.push('\n');
            writer()?
                .send(Submission {
                    dir: dir.to_path_buf(),
                    line,
                    reply,
                })
                .map_err(|_| io::Error::other("the audit writer stopped"))
        });
    // A failure to queue is reported through the receipt like a failure to write.
    if let Err(error) = queued {
        let (reply, failed) = oneshot::channel();
        let _ = reply.send(Err(error));
        return AuditReceipt(failed);
    }
    AuditReceipt(receipt)
}

/// Appends `entry` as one line and waits for it. Blocking: callers run it off the main thread
/// or where the process is about to end. It queues behind the lines `submit_audit` queued.
pub(crate) fn append_audit(dir: &Path, entry: &AuditEntry) -> io::Result<()> {
    futures::executor::block_on(submit_audit(dir, entry))
}

/// Writes `line`, which ends in a newline.
///
/// The whole line goes out in one `write_all` on an append-only handle, so lines do not
/// interleave; a crash can leave a truncated last line, which a reader skips.
fn write_line(dir: &Path, line: &str) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    // The file names clusters and objects, so only its owner may read it. The mode applies only
    // when this call creates the file; an existing file keeps its mode. Windows inherits the
    // ACL of the settings folder.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(audit_path(dir))?;
    file.write_all(line.as_bytes())?;
    file.sync_data()
}

/// The fields to record for an object of `kind_name`: every value is dropped for a kind whose
/// values may be credentials, whatever the operation.
pub(crate) fn recordable_fields(kind_name: &str, mut fields: Vec<AuditField>) -> Vec<AuditField> {
    if PATH_ONLY_KINDS.contains(&kind_name) {
        for field in &mut fields {
            field.value = None;
        }
    }
    fields
}

/// The user's note as one line: control characters become spaces, the ends are trimmed, and at
/// most `NOTE_LIMIT` characters stay. An empty note is no note.
pub(crate) fn clean_note(note: &str) -> Option<String> {
    let one_line: String = note
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let capped: String = one_line.trim().chars().take(NOTE_LIMIT).collect();
    let trimmed = capped.trim_end();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub(crate) fn timestamp_now() -> String {
    let now = jiff::Timestamp::now();
    // Whole seconds keep the lines short and stable to read.
    jiff::Timestamp::from_second(now.as_second())
        .map_or_else(|_| now.to_string(), |at| at.to_string())
}

#[cfg(test)]
#[path = "audit_log_tests.rs"]
mod audit_log_tests;
