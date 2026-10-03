//! The local audit log (spec 0030): one JSON object per line in `<config>/audit.jsonl`, for lock
//! toggles now and for commits later. Pure: it sends nothing to any cluster.
//!
//! A line holds names and paths, never a request body, a token, or the value of a Secret field.

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::app_shell::write_flow::WriteIntent;
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
            path: field.path.to_owned(),
            value: field.value,
        })
        .collect();
    AuditEntry {
        at: timestamp_now(),
        cluster: guard.display_name().to_owned(),
        context: guard.summary.name.clone(),
        user: guard.summary.user.clone(),
        action: intent.button.to_string(),
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

/// Appends `entry` as one line. Blocking: callers run it off the main thread.
///
/// The whole line goes out in one `write_all` on an append-only handle, so lines do not
/// interleave; a crash can leave a truncated last line, which a reader skips.
pub(crate) fn append_audit(dir: &Path, entry: &AuditEntry) -> io::Result<()> {
    let mut line = serde_json::to_string(entry).map_err(io::Error::other)?;
    line.push('\n');
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

fn timestamp_now() -> String {
    let now = jiff::Timestamp::now();
    // Whole seconds keep the lines short and stable to read.
    jiff::Timestamp::from_second(now.as_second())
        .map_or_else(|_| now.to_string(), |at| at.to_string())
}

#[cfg(test)]
#[path = "audit_log_tests.rs"]
mod audit_log_tests;
