# 0017 · App: the Releases kind

[Back to index](README.md) · Step 2 (History buttons and menu views in step 3) · Modules: `resource_kind.rs`, `kind_row.rs`, `helm_rows.rs` (new) + `helm_rows_tests.rs`, `live_sections.rs`, `kind_diagnosis.rs`, `related_objects.rs`, `cluster_session.rs`, `drawer.rs`, `object_events.rs`, `yaml_view.rs`, `resource_actions.rs`. Anything not listed works unchanged for a new `ResourceKind` (0013 [app-model.md](../0013-policy-kinds/app-model.md)).

## `KindSpec`

| Variant | label | object | singular / plural | badge | ns | access | read-only actions | delete label |
|---|---|---|---|---|---|---|---|---|
| `HelmReleases` | Releases | Secret | release / releases | Hm | yes | `ListSecrets` | Roll back… | Uninstall release… |

`NameColumn::Flexible`, `has_labels: false` (Helm's labels are internal), no port-forward, no Monitor. Appended to `ALL` **after Secrets** (decision 22). `watch_rows`: `rows(update, helm_release_row)` over `watch_helm_releases(scope)`. The sidebar item Helm › Releases opens it through `from_label`.

| Special case | Change |
|---|---|
| counts (C11, 0012 step 5) | new `ResourceKind::has_count(self) -> bool`, false for `HelmReleases`; `refresh_kind_counts` skips such kinds (decision 23) |
| `object_events.rs` `event_subject` | `Kind { kind: HelmReleases, .. }` → `None` |
| `yaml_view.rs` `object_ref` | `Kind { kind: HelmReleases, .. }` → `None` |
| `drawer.rs` `drawer_tabs` | arm before the generic ones: step 2 `&[Overview]`; step 3 `&[Overview, Values, Manifest, Notes]` |

## Row model (`kind_row.rs`)

```rust
pub(crate) enum KindObject { /* … */ HelmRelease(HelmReleaseSummary) }
pub(crate) enum LiveContent { /* … */ HelmRelease, HelmHistory /* step 2 */, HelmValuesChange /* step 3 */ }
```

## Columns and cells (`helm_rows.rs`)

| Column | Width | Cell |
|---|---|---|
| Chart | 260 | `Mono("{chart.name}-{chart.version}")`; `chart: None` → `Absent` |
| App version | 110 | `Text(app_version)`; none → `Absent` |
| Revision | 80 r | `Quantity { text: revision, value: revision, tone: None }` (numeric sort) |
| Status | 130 | `Toned(helm_status_label(&status))` |
| Updated | 100 r | `Age { at: updated_at, tone: None }` |

```rust
pub(crate) fn helm_release_row(summary: &HelmReleaseSummary) -> KindRow; // sections (C7): step 2 [Live(HelmRelease), Live(HelmHistory)]; step 3 inserts Live(HelmValuesChange) between them
pub(crate) fn helm_status_label(status: &HelmStatus) -> StatusLabel;
```

| Status | Tone |
|---|---|
| Deployed | Ok |
| Failed | Bad |
| PendingInstall, PendingUpgrade, PendingRollback, Uninstalling, Unknown | Warn |
| Uninstalled, Superseded | Done |

Text is `HelmStatus::label()`. The row status (drawer subtitle) is the same label.

## Drawer Overview (built at render from `KindObject::HelmRelease`)

| # | Section | Rows |
|---|---|---|
| — | status box | `kind_diagnosis` arm below |
| 1 | **Release** (`Live(HelmRelease)`) | Chart (Mono), App version, Revision, Status (Toned), Updated (Age), Still deployed `rev {d}` (only with `deployed_revision`), Description (`Stacked`, only when set). `chart: None` → first row `Note("The release payload could not be read, so chart facts are missing.")` |
| 2 (step 3) | **Values changed in rev {n}** (`Live(HelmValuesChange)`) | the `HelmReleaseView` Overview part ([release-view.md](release-view.md)): latest revision vs the highest earlier one, user values, masked path list, Reveal (30s). No earlier revision → `Note("This is the first stored revision.")`; History loading → `Note("Loading…")` |
| 3 | **History** (`Live(HelmHistory)`) | newest first, at most 50: `rev {n}` (Mono) · status (Toned) · `{age} ago` (muted). Step 3 adds buttons ([release-view.md](release-view.md)) |

History states: related list `Loading` → `Note("Loading…")`; `Failed` → `Note(error_text)`; empty → `Note("No revisions found.")`; more than 50 → last row `Note("{n} older revisions not shown.")`.

## Status box (`kind_diagnosis.rs`)

| Condition (first match) | Tone · title | Text |
|---|---|---|
| Failed, description starts with `Rollback "` | Bad · ROLLBACK FAILED | description |
| Failed, description starts with `Upgrade "` | Bad · UPGRADE FAILED | description, then ` Revision {d} is still deployed.` with `deployed_revision` |
| Failed, any other description (or none) | Bad · INSTALL FAILED | description (decision 28) |
| PendingInstall / PendingUpgrade / PendingRollback | Warn · PENDING INSTALL / UPGRADE / ROLLBACK | `Helm started this operation {age} ago and has not finished. A pending release blocks the next helm upgrade.` (age at paint time from `updated_at`) |
| Uninstalling | Warn · UNINSTALLING | `Helm started uninstalling this release {age} ago.` |
| Unknown(s) | Warn · UNKNOWN STATUS | `Helm reports status "{s}".` |
| else | none | — |

A missing description reads `Helm gave no reason.`

## History related watch (`related_objects.rs`, `cluster_session.rs`)

```rust
pub(crate) enum RelatedSubject { /* … */ HelmHistory { namespace: String, release: String } } // from KindObject::HelmRelease
pub(crate) enum RelatedList { /* … */ HelmHistory(LiveList<HelmRevision>) }
```

Stream `watch_helm_history(namespace, release)` mapped to `RelatedUpdate::HelmHistory`; no access gate (the kind's `ListSecrets`). Lifecycle, debounce, and screenshot settle are 0012's. Watches: explorer N + related 1, inside `3N + 4`.

## Menus (`resource_actions.rs`)

Order: **View values**, **View manifest** (step 3; both call `AppShell::open_drawer_tab(key, tab)`), **Roll back…** (disabled, tooltip "Read-only mode"), Copy name, then **Uninstall release…** (danger, disabled "Read-only mode"). Step 2 shows only the disabled items and Copy name.

`open_yaml(key)` becomes `open_drawer_tab(key, tab: DrawerTab)` (same body, `self.drawer.tab = tab`); View YAML callers pass `DrawerTab::Yaml`.

## W7 parts not rendered

List-level "Rollback" button and history "Roll back" (decision 24); the drawer `meta` line (0013 convention); chart version per history row (labels only, decision 5). "Values changed in rev N" is a path list, not W7's line diff (decision 21, accepted deviation).
