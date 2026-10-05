# 0053 · Environments page

[Back to index](README.md) · Step 3 · Modules: `environment_form.rs` (new, pure; tests `environment_form_tests.rs`), `environments_page.rs` (new view; tests `environments_page_tests.rs`), `settings_window.rs`, `launch_options.rs`. The split mirrors `cluster_form.rs` / `clusters_page.rs`.

## Layout (no wireframe; W2 style: `SettingPage` header, `form_row`-like rows, kit controls)

```
Environments                                       (page description below)
Group clusters and choose how changes to them are confirmed.

Built-in
  [PROD]  Production    Type the cluster name
  [STG]   Staging       Click Confirm
  [DEV]   Development   Click Confirm
  [LOCAL] Local         Click Confirm

Custom
  [QA]  [QA__________]  (o)(o)(o)(o)(o)(*)(o)  Behaves like [Staging v]     [Delete]
        Used by 2 clusters
  [DR]  [DR__________]  ...                    Behaves like [Production v]  [Delete]
        Not used
  (empty state: "No custom environments yet.")

  [New environment name______]  [Add]
  <error text in danger colour>
```

- `SettingPage::new("Environments").resettable(false).description(..)` with one `SettingItem::render` holding the `EnvironmentsPage` entity (the Metrics page pattern).
- Built-in rows: `environment_badge`, the name, and `tier_cell(for_tier(tier), ActionRisk::Change)`-style text (`Type the cluster name` / `Click Confirm`). Read-only.
- Custom row: badge (live), name `Input` (fixed width ~160 px, `MAX` 16 chars enforced by validation, not by the input), seven 18 px round swatches (the removed Clusters swatch code, moved here: current one ringed `border_2` `theme.foreground`, tooltip = colour name), `Behaves like` dropdown (`Button…dropdown_menu`, four tiers, checked = current), and a `Delete` danger button. Under it, muted: `Used by N cluster(s)` or `Not used`. Rows keep creation order (no reorder; decision 11).
- A row that `usable_environments` skips (hand-edited reserved or repeated name) is still listed, with its validation error under the name input and no usage line, so the user can rename or delete it.
- Error text under a name input or the add input in `theme.danger` (`error_text` pattern of the Clusters page).

## Pure functions (`environment_form.rs`)

```rust
/// `own` = the list index of the row being renamed (excluded from the uniqueness check); reserved
/// words come from `environment::is_reserved` (decision 6).
pub(crate) fn validate_environment_name(text: &str, own: Option<usize>, custom: &[CustomEnvironment])
    -> Result<String, FieldError>;                                         // trimmed name
pub(crate) fn add_environment(registry: &mut ClusterRegistry, name: String); // appended; Purple, Production tier
pub(crate) fn rename_environment(registry: &mut ClusterRegistry, at: usize, to: String); // + exact entry refs
pub(crate) fn edit_environment(registry: &mut ClusterRegistry, at: usize, edit: impl FnOnce(&mut CustomEnvironment));
pub(crate) fn delete_environment(registry: &mut ClusterRegistry, at: usize); // usable row: refs → BuiltIn(tier)
pub(crate) fn clusters_using(registry: &ClusterRegistry, name: &str) -> usize; // exact refs, every entry
pub(crate) fn is_weaker(from: EnvironmentTier, to: EnvironmentTier) -> bool;  // to < from (Ord = risk)
pub(crate) fn delete_dialog_text(environment: &CustomEnvironment, using: usize) -> (String, String);
pub(crate) fn weaken_dialog_text(environment: &CustomEnvironment, to: EnvironmentTier, using: usize) -> (String, String);
```

Rows are addressed by list index; references by exact name (model.md "Reference matching"). An index out of range is a no-op. Renaming or deleting a skipped (unusable) row never rewrites references: none ever resolved to it. `FieldError` is reused from `cluster_form.rs`.

## Validation messages (decision 6)

| Case | Message |
|---|---|
| empty after trim | `Enter a name.` |
| more than 16 chars | `Use at most 16 characters.` |
| control char | `Remove line breaks and tabs.` |
| `is_reserved` (built-in names and badges, built-in group titles, `auto`) | `'{name}' is used by a built-in environment.` |
| another custom has it (case-insensitive, `own` excluded) | `Another environment is already named '{name}'.` |

Renaming `qa` to `QA` (same name, other case) is allowed.

## Actions

| Action | Effect (one `AppSettings::update` each) |
|---|---|
| Name input `Change` | valid → `rename_environment`, clear the row error; invalid → keep the stored name, show the error |
| Add (button or Enter in the add input) | valid → `add_environment`, clear the input, rebuild row inputs; invalid → error under the add input |
| Swatch click | `edit_environment(.., color = c)` |
| Behaves like pick | `is_weaker(current, t)` and `clusters_using > 0` → alert (delete pattern), OK `Change`, danger; else, or on OK, `edit_environment(.., tier = t)` |
| Delete | `window.open_alert_dialog` (the `confirm_remove` pattern), danger OK `Delete`, cancel shown; OK → `delete_environment`, rebuild row inputs |

Delete dialog (`delete_dialog_text`):

| Using | Title | Body |
|---|---|---|
| 0 | `Delete environment QA?` | `No cluster uses it.` |
| 1 | same | `1 cluster uses it and moves to Staging, the built-in environment with the same confirm rules.` |
| n | same | `{n} clusters use it and move to Staging, …` (same sentence, plural) |
Weaker-tier dialog (`weaken_dialog_text`, only when `using > 0`): title `Change QA to Staging rules?`; body `1 cluster using QA will follow the Staging rules instead of the Production rules.` (plural: `{n} clusters`). OK `Change` (danger), Cancel keeps the tier.

## View state (`environments_page.rs`)

```rust
pub(crate) struct EnvironmentsPage {
    rows: Vec<CustomRow>,            // one per registry.environments item, same order
    new_name: Entity<InputState>,
    new_error: Option<FieldError>,
    _subscriptions: Vec<Subscription>,
}
struct CustomRow { input: Entity<InputState>, error: Option<FieldError> } // row i edits registry.environments[i]
```

Rows are built on open and rebuilt only after this page's add or delete (decision 15). The page re-renders on `observe_global::<AppSettings>` for swatches, tiers, and counts.

## Settings window

- `SettingsPage::Environments`, title `Environments`, icon `IconName::Tag`; `PAGES` grows to 10 with it after `Clusters`.
- `SettingsWindow` holds `environments: Entity<EnvironmentsPage>` next to `clusters` and `metrics`.
- `launch_options.rs`: `settings-environments` → `Settings(SettingsPage::Environments, SettingsSize::Standard)`, added to USAGE.
