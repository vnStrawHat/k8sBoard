# 0030 · Local audit log

[Back to index](README.md) · Step 3 (pure, sends nothing; first user: lock toggles) · Module: `audit_log.rs` (new) + `audit_log_tests.rs`. Decisions 19–22. C1, C2, C10. Wireframe: W10 checkbox "Add a note to the audit log", W10 note 4 ("who, when, what diff; exportable").

## File

- `<config>/audit.jsonl` (`AppSettings::config_dir`, 0025), one JSON object per line, UTF-8, `\n`.
- Append only: `OpenOptions::new().create(true).append(true)`, one `write_all` of the whole line, then `sync_data`. On Windows `append(true)` opens with `FILE_APPEND_DATA`, so each `WriteFile` lands at the current end of file; one process writes one line per call, so lines do not interleave, and a crash can leave at most a truncated last line (a reader skips a line that does not parse). Unix: created with mode `0o600` (it names clusters and objects). Windows: inherits the config folder ACL (0025 decision 9).
- Writes off (`config_dir` is `None`: screenshot runs, newer settings version, no home) → no line; the confirm dialog shows the muted line `Not recorded: settings are not saved this session`.
- No rotation in 0030 (about 300 bytes a line). Settings › Safety shows the path and `Show in folder`.

## Record

```rust
#[derive(Serialize)]
pub(crate) struct AuditEntry {                   // no Debug derive needed; holds no secret by construction
    pub(crate) at: String,                       // RFC 3339 UTC (`jiff::Timestamp::now()`)
    pub(crate) cluster: String,                  // display name
    pub(crate) context: String,
    pub(crate) user: Option<String>,             // kubeconfig user entry name (ContextSummary.user)
    pub(crate) action: String,                   // `ResourceAction` label ("Cordon"), or "Lock" / "Unlock"
    #[serde(skip_serializing_if = "Option::is_none")] pub(crate) object: Option<AuditObject>, // { kind, namespace?, name }; None for lock lines
    pub(crate) fields: Vec<AuditField>,          // { path, value? }
    pub(crate) outcome: AuditOutcome,            // "applied" | "failed" | "unknown" (`OutcomeUnknown`)
    #[serde(skip_serializing_if = "Option::is_none")] pub(crate) error: Option<String>, // redacted WriteError Display
    #[serde(skip_serializing_if = "Option::is_none")] pub(crate) note: Option<String>,
}
/// Step 3: a lock or unlock of the guard's cluster (no object, no fields, outcome "applied").
pub(crate) fn lock_entry(guard: &ClusterGuard, lock: WriteLock) -> AuditEntry;
/// Step 4: one guarded action (write commit or connect start). Cluster, context, user come from the
/// guard of `intent.cluster` (never the primary). A write maps its `Result<WriteOutcome, WriteError>`
/// to `outcome` and `error` as before; `fields` come from `changed_fields()` or the connect intent.
pub(crate) fn audit_entry(intent: &GuardedIntent, guard: &ClusterGuard, outcome: AuditOutcome,
    error: Option<String>, note: Option<&str>) -> AuditEntry;
pub(crate) fn append_audit(dir: &Path, entry: &AuditEntry) -> io::Result<()>;   // blocking
```

Example: `{"at":"2026-10-02T09:12:03Z","cluster":"uat-monitor","context":"readonly@Monitor","user":"readonly","action":"Cordon","object":{"kind":"Node","name":"wk-04"},"fields":[{"path":"spec.unschedulable","value":"true"}],"outcome":"applied"}`

Key allow-list (test `audit_keys_are_the_allow_list`): `at, cluster, context, user, action, object.{kind, namespace, name}, fields[].{path, value}, outcome, error, note`.

## What is recorded

- Commits, one line each, after the outcome is known (applied, failed, or unknown). Connect verbs: one line per session start. Lock and unlock toggles, one line each (step 3, so the module has a production user before any write exists). Dry-runs are not recorded.
- `fields` come from `WriteRequest::changed_fields()`: paths always, values only when the operation marks them recordable.
- **Secret rule (C1, C10)**: a pure `recordable_fields(kind_name, fields)` drops every `value` when the kind is `Secret`, regardless of the operation (enforced here, tested with the name; no Secret write exists yet). ConfigMap data values (0031) are recorded as paths only too, since they often hold credentials.
- **Never** a request body, a YAML document, a diff text, a token, or a server message of a Secret target.
- `note`: trimmed, control characters replaced by spaces, at most 500 chars; empty → `None`.

## Failure handling

- An append error does not undo or block the write (it already happened): `tracing::warn!(path, kind)` and a title-bar notice `Could not write the audit log ({kind})`.
- A crash between the commit and the append loses that line (known ceiling; an intent line before the commit would double the writes).

## Identity

`user` is the kubeconfig user entry name, not the server-side identity (decision 21, kept). `SelfSubjectReview` (`authentication.k8s.io/v1`, GA in 1.28) would give the real user name; it is a non-mutating POST like SSAR; a later item that needs user approval (README open item 3).
