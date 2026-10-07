# 0041 · Revision history tab

[Back to index](README.md) · Steps 1 (cluster) and 2 (app) · Decisions 2, 3, 8, 9. Wireframe W10 tabs.

## Cluster crate (`replica_set.rs`)

```rust
impl ClusterConnection {
    /// One LIST of `deployment`'s namespace's ReplicaSets with `labelSelector = selector` (kubectl
    /// syntax, as `watch_selected_replica_sets`), kept when their controller owner is that Deployment
    /// (kind `Deployment`, same name). An empty selector, or one with `<invalid>`, is `Ok(vec![])` with
    /// no request. Summaries only; read-only. `deployment` must be a Deployment `ObjectRef`, else
    /// `ClusterError::UnexpectedResponse` with fixed text.
    pub async fn deployment_revisions(&self, deployment: &ObjectRef, selector: &str) -> Result<Vec<ReplicaSetSummary>, ClusterError>;
}
```

- `Api::<ReplicaSet>::namespaced(..).list(&ListParams::default().labels(selector))` through `self.run(ACTION, ..)`, `ACTION = "listing the revisions of a deployment"`; `replica_set_summary` per item (existing). No tracing of content.
- Order: as returned; the app sorts.

## App: shared pure helpers (`revision_diff.rs`)

```rust
/// Newest first by revision number (missing numbers last, then by name). Pure.
pub(crate) fn revision_list(replica_sets: &[ReplicaSetSummary]) -> Vec<RevisionSide>;   // is_current = highest number
/// (newest, previous) of `revision_list`; `None` with fewer than two numbered revisions.
pub(crate) fn latest_pair(sides: &[RevisionSide]) -> Option<(RevisionSide, RevisionSide)>;
```

`RevisionSide::of` (0039) builds each side. `diff_request(deployment, clicked, current)` (0039) orders a pair.

## App: the tab

| Item | Rule |
|---|---|
| `EditTab` | gains `History`; `render_tabs` adds `Tab::new().label("Revision history")` only when `kind == Deployment` (index 2) |
| `YamlEditView` fields | `history: Option<Entity<RevisionHistory>>`, created on the first show of the tab, dropped with the view |
| `RevisionHistory` (new entity, `revision_history.rs`) | `deployment: ResourceKey`, `object: ObjectRef`, `connection: ClusterConnection`, `state: HistoryState`, `selected: Option<usize>`, `diff: Option<Entity<RevisionDiffView>>` |
| `HistoryState` | `Loading { _task }`, `Denied` (session access says `ListReplicaSets` denied; no request), `Failed(SharedString)` (`error_text`), `Ready(Vec<RevisionSide>)` |
| Layout | left list 240 px: `rev {n} · {tag}`, muted age, `current` pill; selected row tinted (theme `list_active`). Right: the selected diff (`RevisionDiffView` as a child element) |
| Default selection | `latest_pair`'s previous side; with one revision: `No earlier revision kept (revisionHistoryLimit)` |
| Current row selected | `This is the current revision.` (muted, centered), no diff entity |
| Texts | Loading `Loading revisions…` (spinner); Denied `Not permitted: list replicasets`; Failed `Could not load revisions: {error}` |
| Editor text | never changed by the tab; Ctrl S and Apply act on the editor whatever tab is shown (0031 flow) |

`RevisionDiffView` needs one change for embedding: its root is a sized `v_flex` (`size_full`) without the dialog height share, so the dialog keeps `h(HEIGHT_SHARE × window)` on its wrapper in `open_revision_diff`. No visual change to the dialog.

## Async contract

1. First show of the tab: if `live.access` denies `ListReplicaSets` → `Denied`. The selector is the edited Deployment's `DeploymentSummary.selector` joined by `,`, read from the session's Deployments kind list (or the Deployments condition feed); not found → `Failed("the deployment is not loaded yet")`, no request. Else `cx.spawn` → `ClusterRuntime::spawn(connection.deployment_revisions(&object, &selector))` on tokio → back on GPUI: `revision_list`, `Ready`, one `cx.notify()`.
2. A row click replaces `diff` with `cx.new(|cx| RevisionDiffView::new(diff_request(..), connection.clone(), cx))`; dropping the old entity cancels its GETs (0039 contract).
3. The list is read once per editor; reopening the editor reads again. No cache (0039 decision 11).
4. The connection is the one `open_edit` resolved for the view (the active cluster, 0046); a cluster switch closes the editor through `leaving_work`, dropping both entities.

## Screenshot

`--screen edit-yaml-history`: the 0031 fixture view on `History`, with a fixed `Ready` list (`rev 38 current`, `rev 37`, `rev 36`) and the `--screen revision-diff` fixture diff embedded. No connection call.

On the History tab the side panel leaves out the editor's `N changes` list (it would read as the diff beside it); the Checks stay.

## Roll back where the comparison happens (UX fix H3)

Each older row of the list has `Roll back…`, and the revision diff dialog has `Roll back to rev N…` in its footer (the side that is not current, only when exactly one side is). Both come from `AppShell::roll_back_offer`: the gate of the Deployment's cluster (permission, lock, paused rollout) decides, and a disabled button carries the reason as its tooltip. A click closes the diff dialog, then `begin_roll_back` opens the usual confirm dialog (`roll_back_intent`). The history tab calls `begin_roll_back` directly, so it works while the edit is open; the Deployment change then reaches the editor as a conflict.

A Roll back that goes through closes the Edit YAML view of that Deployment (`roll_back_finished`), so its text, `resourceVersion`, and `current` pill never stay behind the new revision. An editor holding unsaved text stays open: the rollout notice still appears, and Apply reads the change as a conflict.

## Change cause (UX round 3, O2 and N1)

A row shows the ReplicaSet's `kubernetes.io/change-cause` (`ReplicaSetSummary.change_cause`, `RevisionSide.change_cause`) as a muted second line under `rev N · tag`, cut to the row; a row without one has no second line. Hovering the row (the drawer's Revisions rows too) shows the whole cause, then `Created 2026-10-06 17:26 +07` in the system zone (`revision_tooltip`). The cause comes from the Deployment's annotation at the time the ReplicaSet was made, so Set image (0032) and `kubectl set image` with an annotation fill it; a revision made another way has none.
