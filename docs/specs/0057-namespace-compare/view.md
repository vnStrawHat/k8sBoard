# 0057 view — app side

## Entry points

- **Namespaces row**: menu item `Compare with…` (hint `V`), row action `RowAction::CompareNamespaces` -> `ResourceAction::CompareNamespaces`, gate `ReadOnly { check: None }`, offered on the Namespaces kind only. Key `V` is free in the keymap and is added to the shortcut sheet (`SelectedResource`, "Compare with another namespace (Namespaces)"), to `ROW_ACTIONS` and `PAIR_LABEL_ACTIONS` of `palette_search.rs`.
- **Palette**: the row action above with a namespace under the cursor, and the command `Compare namespaces` (General group, action `OpenNamespaceCompare`, chord `Ctrl+Shift+D`), which takes the scope's single namespace, else the cluster's default namespace, as the left side.
- A cluster without a live session opens nothing (like the revision diff).

## The dialog (`namespace_compare_view.rs`)

`NamespaceCompareView` in a dialog, width like the revision diff, height 70 % of the window, a scroll area inside. Title `Compare namespaces`. Three phases:

1. **Pick**: `{left} ↔` plus a filter `Input` and the loaded namespaces except the left one (case-insensitive `contains`, as the namespace picker); Enter picks the first match (the action `PickNamespace`, bound in `NamespaceCompare > Input`, because the kit dialog would otherwise take the same Enter as its confirm and close), a click picks any. The filter takes the focus on the first frames of the phase (a few tries, since the menu the dialog was opened from can give the focus back). No namespaces loaded: a note.
2. **Loading**: spinner; the task is dropped with the dialog. The call runs on the cluster runtime; the line model is a cheap pass over the result and runs on the main thread (a diff is computed only for an object whose `Open diff` is on).
3. **Ready**: toolbar text `lab-shop ↔ lab-shop-stg · 3 differ · 1 only in lab-shop · 0 only in lab-shop-stg · 7 same`; buttons `Show env values` (only when env literals are hidden; reloads) and `Change namespace` (back to Pick). Body, per kind with something to show: a kind header, then the groups `Only in {left}`, `Only in {right}` (names) and `Differs` (one row per object: name, `Open diff` button, then its field change lines `path  old → new`, an absent side as `—`). `Open diff` toggles the line diff of `left_text` against `right_text` under the object (`yaml_diff::diff_rows`, drawn with `diff_row_element`). An unreadable kind is its header and one muted line (`Could not be read: {error}`). Nothing to show at all: `The two namespaces are the same (N objects).`

A failed task shows its message (danger color). Esc closes (kit dialog). Tab walks the buttons. While the lines show they hold the focus, and Page Up, Page Down, Home and End scroll them with the drawer rules (`drawer::scrolled_offset`); the mouse wheel scrolls too.

## Row model

`compare_lines(comparison, expanded) -> Vec<CompareLine>` is pure and tested: `Kind`, `Group`, `Name`, `Object`, `Change`, `Diff(DiffRow)`, `Note`. The view renders the lines in one scrolling column (no virtual list: a few hundred lines; a longer comparison would need one).

## P33: links

`drawer::link_text` prefixes the shown text with `namespace/` when the target is namespaced and the open session's scope is All or has two or more namespaces (`scope_shows_namespace`, pure). The scope is read from the `ActiveConnection` global's session, so no call site changes. A text that already starts with the namespace is left alone.
