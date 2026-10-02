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
- **Locking** is immediate. **Unlocking** runs `confirm_step(guard.confirm, Change, trigger, display_name)` (write-flow.md dialog, title `Unlock {cluster} for changes?`, no dry-run). Lock and unlock each append an audit line from step 3 (audit-log.md).
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

- RBAC before the lock: unlocking cannot fix a denial, so it is the more useful reason.
- `READ_ONLY_MODE_REASON` and `READ_ONLY_FEATURE_REASON` are replaced by the row-2 text; the 0028 test strings change with it.
- `ActionGate` gains `mutates: bool`; `ResourceAction::Cordon` maps to `AccessCheck::PatchNodes`, `mutates: true` (step 4 sets it shipped). Kind-dependent actions carry their kind (`Scale(ObjectKind)`, `RestartRollout(ObjectKind)`, 0032), so `gate()` picks the per-resource check and `action_availability` stays two-argument (decision 35). `key_availability` (0028) builds the guard from the subject row's cluster.
- SSAR source: the session `AccessReport` (one review per check at session start and scope change). Per-kind Update/Delete checks are lazy: they are not in `AccessCheck::ALL`, and the lookup reads the session's `kind_access` (0031 edit-model.md "Lazy write checks"). No extra SSAR per object; the dry-run is the per-object check.

## Confirm tiers

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConfirmMode { TypeName, Enter, Click, None }
impl ConfirmMode { pub(crate) fn for_environment(environment: Environment) -> Self; } // PROD, STG, DEV, LOCAL order
pub(crate) enum ActionRisk { Change, Destructive }   // declared per ResourceAction
pub(crate) enum Trigger { Pointer, Key }
pub(crate) enum ConfirmStep { Run, Dialog(DialogConfirm) }
pub(crate) enum DialogConfirm { ClickOnly, EnterOrClick, TypeName { expected: String } }
pub(crate) fn confirm_step(mode: ConfirmMode, risk: ActionRisk, trigger: Trigger, expected: &str) -> ConfirmStep;
```

| Mode (default env) | Change | Destructive |
|---|---|---|
| `TypeName` (PROD) | dialog, type the name | dialog, type the name |
| `Enter` (STG) | dialog, Enter or click | dialog, Enter or click |
| `Click` (DEV) | pointer: dialog, one click on Apply (Enter does nothing); key: dialog, Enter or click | dialog, Enter or click |
| `None` (LOCAL) | pointer: run; key: dialog, Enter or click | dialog, Enter or click |

- Wireframe rule "no destructive action runs from one key; destructive actions always open a confirmation" overrides `Click` and `None`. A key never runs a change without a dialog; `None` differs from `Click` only for the pointer.
- **Proposed reading** of the W10 tiers ("dev one click, staging Enter, prod type the name", LOCAL none): the user is asked to confirm it before the commit step (step 4).
- `expected`: the cluster display name (W10, W2 "Typing the cluster name") unless the action names its object (W6 drain types the node name; the feature passes it). Match: exact after trimming surrounding spaces; case-sensitive.
- Cordon / Uncordon is `Change`.

## Settings

- Registry key `registry.clusters[].confirm: Option<ConfirmMode>` (reserved in 0024; `None` = `for_environment`). Add to the 0024 allow-list test.
- `ClusterProfile.confirm: ConfirmMode` resolved like `read_only`.
- 0025 Clusters form, Safety section, below "Open as read-only": `Confirm changes by` select: `Auto ({default label})`, `Typing the cluster name`, `Pressing Enter`, `One click`, `No confirmation`. Reset clears it with the entry.
- Settings › **Safety** page at W2 position 5 (0025 page order): group "Audit log" with the file path (mono) and `Show in folder` (`cx.reveal_path`), plus a static table of the four tiers. Row added to `pages_follow_w2_order`.

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
- Title bar in multi mode: the badge reads `Read-only` while any viewed cluster is locked (tooltip lists them), else `Unlocked`; its click opens a menu with one checked item per viewed cluster (env badge + name) that toggles that cluster. Ctrl Shift R locks every viewed cluster when any is unlocked, else opens that menu (unlocking stays per cluster, through its own tier).

## Badge and key

- `title_bar.rs` `read_only_badge` becomes a ghost button: `Locked` → lock icon + `Read-only`; `Unlocked` → `IconName::LockOpen` + `Unlocked`. Border: 1 px dashed in `environment_color` (W1 `.lock`); without a session the badge is hidden. Click and `ToggleReadOnly` run `AppShell::toggle_write_lock`.
- 0028 `keymap.rs`: bind `secondary-shift-r` → `ToggleReadOnly` in `WINDOW`; drop it from `RESERVED_KEYS`; sheet row "Toggle read-only" (General). Tooltip via `tooltip_with_action`.
