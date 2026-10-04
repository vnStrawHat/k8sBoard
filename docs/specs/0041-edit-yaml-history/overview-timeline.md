# 0041 · Overview timeline: "who" and click-to-diff

[Back to index](README.md) · Steps 1 (cluster) and 4 (app) · Decisions 6–10. Wireframe W3 Recent changes, note 4 (the timeline joins revisions, managedFields, and events; a click on a row opens the diff, W10).

## What W3 n4 asks, and what is built

| W3 element | Needs stored history? | Built |
|---|---|---|
| "who" on a Deployment rollout (`ci-bot`) | no: `managedFields` holds the manager and time of each writer | yes (manager name) |
| "who" on an HPA rescale (`hpa`) | no | already: event source |
| "who" on a node change (`kubelet`) | no | already |
| ConfigMap row "changed 2 keys · an.nguyen" | **yes**: the key count needs the previous data | no (user decision 2026-10-03) |
| Click a row → diff (W10) | Deployments: no (old ReplicaSets keep old templates). Others: yes, or nothing to diff | Deployment rows only |

## Cluster crate: `DeploymentSummary.template_change`

```rust
/// The newest writer of the pod template, from `metadata.managedFields`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldWriter { pub manager: String, pub at: jiff::Timestamp }
pub struct DeploymentSummary { /* … */ pub template_change: Option<FieldWriter> }
```

- In `deployment_summary`: among `managed_fields` entries with `subresource` empty or absent, `operation` `Update` or `Apply`, a `time`, and `fieldsV1` holding `f:spec` › `f:template`, the newest by `time` (ties: the later entry). `manager` empty → skipped.
- Only the name and time are kept; `fieldsV1` is dropped in the summarizer. Every watch of Deployments carries the field; no new request.

## App: "who" (`recent_changes.rs`)

- `ChangeInputs` gains `deployments: Option<&[DeploymentSummary]>` (the Deployments condition feed's `KindObject::Deployment` items; `None` when the feed is off or loading).
- `event_entry` for `ChangeKind::Deployment`: the event's Deployment (namespace, name) found in `deployments`; its `template_change` used as `actor` when `event.last_seen − 30 min ≤ at ≤ event.last_seen + 60 s` (decision 7); else the event source actor (today).
- `ChangeEntry` gains `actor_source: ActorSource { FieldManager, EventSource }`; the row tooltip appends `· field manager` or `· event source`. Rendering of the actor text is unchanged.

## App: click-to-diff

```rust
impl AppShell {
    /// Lists the Deployment's revisions on tokio, then opens the 0039 dialog with the newest and the
    /// one before. Fewer than two numbered revisions → notice `No earlier revision kept for deployment/{name}`.
    pub(crate) fn open_latest_revision_diff(&mut self, deployment: ResourceKey, window: &mut Window, cx: &mut Context<Self>);
}
```

| Item | Rule |
|---|---|
| Row click | `ChangeKind::Deployment` with a target → `open_latest_revision_diff`; every other row → `reveal` (today) |
| Access | `ListReplicaSets` denied → notice `Revision diff is unavailable: Not permitted: list replicasets`; no request |
| Request | `deployment_revisions` (one LIST), then the dialog's own two GETs (0039) |
| Pair | `latest_pair(revision_list(..))` ([revision-history.md](revision-history.md)) → `diff_request(key, previous, newest)` |
| Busy | a second click while the LIST runs replaces the task (the field `revision_lookup: Option<Task<()>>`) |
| Failure | notice `Could not load revisions: {error_text}` |
| Dialog footer | `RevisionDiffView` gains an optional `go_to: Option<ResourceKey>`; when set, a ghost `Go to deployment` button reveals the key and closes the dialog. The drawer's Diff buttons pass `None` (they are already on the Deployment) |

## Async contract

1. `cx.spawn` → `ClusterRuntime::spawn(connection.deployment_revisions(&object))` on tokio; the connection is the active session's (0046).
2. Back on GPUI: if the session changed meanwhile (`entity_id` check, as `export_overview_report`), drop the result. Else build the pair and `open_dialog` with the view, as `open_revision_diff` does.
3. No cache; nothing logged.

## Screenshot

`--screen overview` is a live screen: the "who" rule is proven by unit tests, and the UAT run checks the column when a rollout falls in the window. `--screen revision-diff`: the fixture dialog with `go_to` set.
