# 0030 · Lock, gate, confirm tiers

[Back to index](README.md) · Steps 2a (pure guard, gate, confirm setting), 2b (lock, badge, key, unlock dialog) · Modules: `write_guard.rs` (new, pure) + `write_guard_tests.rs`, `resource_actions.rs`, `cluster_registry.rs` (0024), `cluster_session.rs`, `title_bar.rs`, `keymap.rs` (0028), `settings_window.rs` (0025). Decisions 6–12. Wireframes: W2 Safety, env token rows, keyboard map.

## Read-only lock

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteLock { Locked, Unlocked }
impl WriteLock { pub(crate) fn at_open(profile: &ClusterProfile) -> Self; } // profile.read_only (0025 resolver)
```

- `ClusterSession` holds `lock: WriteLock`, set by `at_open` when the session starts (and on switch); a reconnect keeps it. 0027 has one per session.
- `profile.read_only` = `entry.read_only.unwrap_or(environment == Production)` (0024 decision 27, 0025 decision 17). The W2 switch "Open as read-only" is that stored default; toggling the lock in the title bar is **session-only** and never writes `settings.json`.
- **Locking** is immediate. **Unlocking** runs `confirm_step(guard.confirm, Change, display_name)` (write-flow.md dialog, title `Unlock {cluster} for changes?`, no dry-run). Lock and unlock each append an audit line from step 3 (audit-log.md).
- Enforcement point: `write_flow.rs` re-reads the lock right before every commit (write-flow.md). The gate below only decides what the UI offers.

## The gate (one function)

`resource_actions::action_availability` stays the single source for menus, 0028 keys, and the 0029 palette. New signature and order:

```rust
pub(crate) fn action_availability(action: ResourceAction, guard: &ClusterGuard) -> ActionAvailability; // below
```

| # | Condition | Reason text |
|---|---|---|
| 1 | read-only action (View logs, View YAML, Copy name) | gated by RBAC only, lock ignored |
| 2 | mutating action whose spec has not shipped | `Comes in a later version` |
| 3 | `AccessState::Checking` / `Unknown` | `Checking permissions…` / `Permissions could not be checked` (unchanged) |
| 4 | SSAR denied | `Not permitted: {check}` (unchanged format, e.g. `Not permitted: patch nodes`) |
| 5 | `WriteLock::Locked` | `{cluster} is read-only` (`{cluster}` = display name) |
| 6 | otherwise | Enabled |

- A denied action says what to do (UX walkthrough M14): a disabled menu row reads `No permission · {verb resource}` (the resource alone for a verb pair), its tooltip adds `Open Check permissions to see your rules`, a notice after a denied attempt carries a `Check permissions` button, and a disabled Forward button shows the same short reason under its port.
- A locked cluster has an unlock path in place (UX walkthrough M17): a disabled menu row reads `Read-only · Ctrl+Shift+R` (`Cmd` on macOS) and its tooltip, a disabled bulk button, and the block line of a write dialog add `Unlock {cluster} with Ctrl+Shift+R or the title-bar badge`; the item stays disabled.
- RBAC before the lock: unlocking cannot fix a denial, so it is the more useful reason.
- `READ_ONLY_MODE_REASON` and `READ_ONLY_FEATURE_REASON` are replaced by the row-2 text; the 0028 test strings change with it.
- `ActionGate` gains `mutates: bool`; `ResourceAction::Cordon` maps to `AccessCheck::PatchNodes`, `mutates: true` (step 4 sets it shipped). Kind-dependent actions carry their kind (`Scale(ObjectKind)`, `RestartRollout(ObjectKind)`, 0032), so `gate()` picks the per-resource check and `action_availability` stays two-argument (decision 35). `key_availability` (0028) builds the guard from the subject row's cluster.
- SSAR source: the session `AccessReport` (one review per check at session start and scope change). Per-kind Update/Delete checks are lazy: they are not in `AccessCheck::ALL`, and the lookup reads the session's `kind_access` (0031 edit-model.md "Lazy write checks"). No extra SSAR per object; the dry-run is the per-object check.

## Confirm tiers

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConfirmMode { TypeName, Click }   // "type-name", "click"
impl ConfirmMode { pub(crate) fn for_environment(environment: Environment) -> Self; } // Production → TypeName, else Click
pub(crate) enum ActionRisk { Change, Destructive, Privileged }   // declared per ResourceAction; Privileged is 0037 (node shell)
pub(crate) enum DialogConfirm { Click, TypeName { expected: String } }
/// Every guarded action opens the confirm dialog; this only picks how it is confirmed.
pub(crate) fn confirm_step(mode: ConfirmMode, risk: ActionRisk, expected: &str) -> DialogConfirm;
```

| Mode (default env) | Change | Destructive |
|---|---|---|
| `TypeName` (PROD) | dialog, type the name | dialog, type the name, danger button |
| `Click` (STG, DEV, LOCAL; unknown → STG) | dialog, click the focused confirm button | dialog, click the focused danger button |
| any tier | `Privileged` (node shell, 0037): dialog, type the **node name**, danger button, whatever the cluster tier is | |

- (user, 2026-10-02, decision 9) **Every guarded action opens a dialog**, for every tier, risk, and trigger (pointer, key, palette). The Enter, one-click-without-dialog, and None tiers are removed; `Trigger` and `ConfirmStep::Run` went with them, since they only chose between those tiers.
- Enter inside the dialog activates the focused confirm button; held or repeated Enter is ignored (decision 28, write-flow.md).
- `Change` and `Destructive` confirm the same way (the risk only picks the danger button). `Privileged` (0037, as built) is its own arm of the exhaustive `confirm_step` match: `TypeName { expected }` for every mode, with the node name as `expected`, and the danger button.
- `expected` (UX walkthrough M13, one rule for every typed confirmation): the **name of the object** the action is on (`WriteIntent`: the request target; `ConnectIntent`: its `object`, so Attach, shells, and Forward type the pod, service, or node; a batch of one: that object). Only an action with no single object, a batch of several, types the **cluster display name** (W10, W2 "Typing the cluster name"); so does unlocking a cluster. The dialog's hint line says which (`Type the pod name to confirm`). Match: exact after trimming surrounding spaces; case-sensitive.
- Cordon / Uncordon is `Change`.

## Settings

- Registry key `registry.clusters[].confirm: Option<ConfirmMode>` (reserved in 0024; `None` = `for_environment`). Add to the 0024 allow-list test.
- `ClusterProfile.confirm: ConfirmMode` resolved like `read_only`.
- 0025 Clusters form, Safety section, below "Open as read-only": `Confirm changes by` select: `Auto ({default label})`, `Typing the cluster name`, `Clicking Confirm`. Reset clears it with the entry.
- Settings › **Safety** page at W2 position 5 (0025 page order): group "Audit log" with the file path (mono) and `Show in folder` (`cx.reveal_path`), plus a static table of the two tiers. Row added to `pages_follow_w2_order`.

## Row's own cluster (0027 contract)

0027 can have several live sessions, and its rows carry `ClusterObject { cluster: ClusterRef, key }`. Every guardrail input comes from **the row's cluster, never the primary or "the active session"**:

```rust
/// Everything the gate and the confirm step need, taken from one cluster's session and profile.
pub(crate) struct ClusterGuard<'a> { pub(crate) cluster: &'a ClusterRef, pub(crate) access: &'a AccessState,
    pub(crate) lock: WriteLock, pub(crate) environment: Environment, pub(crate) confirm: ConfirmMode,
    pub(crate) display_name: &'a str, pub(crate) profile: &'a ClusterProfile, pub(crate) summary: &'a ContextSummary,
    pub(crate) generation: u64 /* session connection generation; bumps on reconnect or switch */ }
impl AppShell { pub(crate) fn guard_for(&self, cluster: &ClusterRef, cx: &App) -> Option<ClusterGuard<'_>>; } // None: not viewed
```

- **Everything resolves from `WriteIntent.cluster`** through `guard_for`: the profile, the gate (`action_availability(action, guard)`), the tier (`confirm_step(guard.confirm, ..)`), the typed-name `expected` (`guard.display_name`), the dialog badge (`guard.environment`), and the audit `profile`/`summary`. Test `gate_and_confirm_use_the_rows_cluster`.
- `write_flow` resolves the guard at the dry-run **and** again right before the commit. A ref no longer viewed, or a `generation` that changed (switch or reconnect in between) → `{cluster} is no longer open; nothing was changed`.
- The audit line names that cluster. No guardrail API takes an implicit session.
- Single mode (0030 alone): the only session is the row's cluster, so behavior is the same; 0027 only adds slots.
- Title bar in multi mode: the badge reads `Read-only` only when every viewed cluster is locked; while any is open the open lock wins, as `Unlocked: stg-b` (one open cluster among several) or `2 of 3 unlocked` (decision 12, amended 2026-10-03: the state that can change something must not be hidden by a locked neighbour; the tooltip lists each cluster's state); its click opens a menu with one item per viewed cluster (env badge + name, ticked while read-only) that toggles that cluster. **Ctrl Shift R toggles the lock of one cluster: the cluster of the cursor row (which is the cluster of the open drawer), else the primary when there is no cursor** (as built; it replaces "lock every viewed cluster", so a key press never unlocks or locks a cluster the user is not looking at). Locking is immediate; unlocking asks that cluster's own tier.

## Badge and key

- `title_bar.rs` `read_only_badge` becomes a ghost button: `Locked` → lock icon + `Read-only`; `Unlocked` → `IconName::LockOpen` + `Unlocked`. Border: 1 px dashed in `environment_color` (W1 `.lock`); without a session the badge is hidden. Click and `ToggleReadOnly` run `AppShell::toggle_write_lock`.
- 0028 `keymap.rs`: bind `secondary-shift-r` → `ToggleReadOnly` in `WINDOW`; drop it from `RESERVED_KEYS`; sheet row "Toggle read-only" (General). Tooltip via `tooltip_with_action`.
