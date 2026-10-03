# 0039 · Logs items and menus

[Back to index](README.md) · Steps 1–2 · Modules: `resource_actions.rs`, `pod_drawer.rs`, `container_detail.rs`, `log_target.rs`, `app_shell.rs`, `keyboard_navigation.rs`. Decisions 1–7.

## View logs ▸ container submenu (step 1, W4 n2)

`pod_menu` calls `view_logs_item(pod, None, live, row, dock)` today: one item, default container. It becomes a built item like `ShellMenu` (a submenu needs the app):

```rust
/// The View logs item of a pod menu, owned so the caller builds it before borrowing the session.
pub(crate) struct LogsMenu { state: LogsMenuState, target_pod: PodSummary }
pub(crate) enum LogsMenuState {
    Disabled(SharedString),          // logs_launch(pod, None, access) failed (not permitted, no containers)
    One(LogTarget),                  // one container: the item opens it, hint L
    Pick(Vec<LogChoice>),            // several: submenu
}
pub(crate) struct LogChoice { pub(crate) name: String, pub(crate) tag: &'static str /* kind_tag_text */ }
impl LogsMenu {
    pub(crate) fn of(pod: &PodSummary, access: &AccessState) -> Self;
    pub(crate) fn item(self, live_connection: ClusterConnection, row: &RowContext, dock: &WeakEntity<Dock>,
        window: &mut Window, cx: &mut App) -> PopupMenuItem;
}
```

- Entries in `pod.containers` order (init and sidecar first, then main, as the drawer lists them), text `{name} · {tag}`; each opens `LogTarget::of_container(pod, name)` via `dock.open(LogOrigin::new(row, connection), …)`. Logs of a container that is not running are still readable (previous/terminated logs), so no entry is disabled for state.
- `PodMenuItems` gains `view_logs: PopupMenuItem`; both pod menus (row `pod_table.rs` context menu and drawer `pod_menu_button`) build it before borrowing the session, like `open_shell`.
- `view_logs_item` stays for the Issues row menu and the container ⋯ menu (one named container).
- Key L is unchanged on a pod: the default container (`selected_log_target`).

## Container ⋯ menu (step 1, W4b n3)

`container_detail` header (`h_flex` with name, state, kind tag, retry) gets `menu_button()` at `ml_auto`, built by `pod_drawer.rs::containers_tab`, passed in a new field `ContainerDetailInput.menu: Option<AnyElement>` (`None` while the session is not live).

```rust
// resource_actions.rs, next to pod_menu
pub(crate) fn container_menu(menu: PopupMenu, pod: &PodSummary, container: &ContainerSummary,
    live: &LiveCluster, guard: &ClusterGuard<'_>, row: &RowContext, links: &PodMenuLinks<'_>) -> PopupMenu;
/// The gate of the pod's cluster, then the container: not running → NOT_RUNNING_REASON.
pub(crate) fn container_shell_availability(container: &ContainerSummary, guard: &ClusterGuard<'_>) -> ActionAvailability;
```

| Item | Does | Gate |
|---|---|---|
| View logs | `view_logs_item(pod, Some(&container.name), live, row, dock)` | `logs_launch` (read-only) |
| Open shell | `shell.start_shell(ShellOpen { cluster: row.cluster, namespace, pod, short_pod, container }, …)` | `container_shell_availability`: `action_availability(OpenShell, guard)` then running state; existing 0036 guarded flow, no new call |
| separator | | |
| Copy image | clipboard `container.image` | none |

No key hints on these items: L, S act on the pod's default container (decision 3). Attach is not listed (audit 0040).

## CronJob View logs of last job (step 2, W7 CronJobs)

```rust
// log_target.rs
/// The pods of the Job the CronJob controller created for `last_schedule_at`, named
/// `{cron_job}-{unix minutes}`. Pure.
pub(crate) fn last_job_owner(cron_job: &CronJobSummary, pods: &[PodSummary]) -> Result<PodOwner, SharedString>;
```

| Case | Result |
|---|---|
| `last_schedule_at` is `None` | `Err("No job has run yet")` |
| no pod in the namespace with `controller == Job {name}` | `Err("Job {name} has no pods left")` |
| else | `Ok(PodOwner::Controller { namespace, kind: JOB_KIND, name })` |

`ponytail:` manual `Trigger now` jobs are not "last" (they use `generateName`); upgrade path: read the drawer's loaded Jobs (`recent_jobs`) when present.

- Menu: `kind_menu` adds, for `ResourceKind::CronJobs`, `View logs of last job` before the change actions: enabled → `shell.open_workload_logs(&context.cluster, owner, …)`; disabled → `disabled_menu_item(label, reason)`; both carry `.action(RowAction::ViewLogs.key_action())` (hint L). The pods come from `MenuCluster.pods` (the row's own cluster).

## L on kind rows (step 2, `RowAction::ViewLogs`)

| Function | Today | Change |
|---|---|---|
| `subject_action(ViewLogs, subject)` | pods only | also `Kind { kind }` when `kind` is Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, or CronJobs |
| `key_availability(row, subject, live, guard)` | pod lookup | for a kind subject: `availability_before_lock(ViewLogs, access)`, then the row: CronJob → `last_job_owner` (Err → `Disabled { reason }`); others → `row.related_pods` present, else `NotOffered` |
| `selected_log_target` | `row.related_pods` for kinds | CronJob arm through `last_job_owner`; reads the cursor's own slot (`slot_live(&selected.cluster)`), never the primary |
| `workload_logs_item` | no hint | `.action(RowAction::ViewLogs.key_action())` |

`run_available_row_key` already maps `ResourceAction::ViewLogs` to `open_logs_of_selection`; no new arm. The palette lists L for these rows through the same `key_availability`.
