# 0057 — Compare two namespaces

Status: **draft 2026-10-07**, from the round-3 walkthrough (U48, proposal 14, finding P33); the user approved the proposal. Crates: `crates/cluster` (data), `crates/app` (dialog, entry points, links). Read-only: no write, no new permission beyond `list` on the compared kinds.

## Goal

"Staging works, prod does not. What differs?" One action diffs `lab-shop` against `lab-shop-stg` by kind: what exists in only one of them, and for objects present in both, which fields differ (replicas, image, env, resources, ConfigMap keys, Service ports, CronJob schedule). Today the workaround is a two-namespace table per kind with no diff (about ten steps per kind).

## Non-goals

- Comparing across clusters (one cluster at a time, spec 0046), or three namespaces.
- Writing: no "copy to the other namespace", no sync.
- Kinds beyond the eleven below, custom resources, Helm releases, pods and ReplicaSets (they follow their owner).
- Live updates of an open comparison: it is a snapshot with a Refresh button.

## Files

| File | Content |
|---|---|
| [data.md](data.md) | kinds, one-shot lists, masking and Secret handling, matching, field changes, types |
| [view.md](view.md) | entry points, the dialog and its three phases, rows, keys, P33 links |

## Decisions

1. **One-shot lists, not the loaded watches.** The watches hold summaries (no full spec), and the scope may not include both namespaces; one LIST per kind per namespace (22 requests, concurrent, each under the request timeout) gives complete objects for every case. A kind that cannot be listed (forbidden) is reported on its own line and never fails the rest.
2. **Equality is on the cleaned object**: masked like the YAML tab, then `status`, the server's metadata and `<hidden>` values dropped (the `clean_yaml` rules), then the namespace and a short list of values the cluster assigns per namespace dropped. Equal means "Same" and is only counted.
3. **Secrets never surface a value.** Each value becomes a token that is equal exactly when the values are equal (a salted hash made once per comparison, never stored); only key names and "differs" are shown.
4. **Env literals stay hidden** (as in the revision diff) until the toolbar toggle reloads with them shown; with them hidden, a differing literal is not seen, and the header says how many are hidden.
5. **The second namespace is picked inside the dialog** (filter field plus the loaded namespaces) instead of a separate popover: one component, Esc closes it, and the palette command shares it.
6. **Field changes reuse `edit_preview`** (`field_paths`, `field_changes`): the same paths (`spec.template.spec.containers[web].image`) the Edit YAML preview prints.
7. **P33**: object links in drawers show `namespace/text` when the scope covers more than one namespace (All, or several), fixed once in `drawer::link_text`.

## Acceptance

- [x] A1 Namespaces row menu, key `V`, and the palette offer Compare with…; the palette also has Compare namespaces.
- [x] A2 The dialog lists, per kind, Only in A, Only in B and Differs with field change lines; equal objects are counted in the header.
- [x] A3 Open diff on a differing object shows the line diff of the two cleaned manifests, Secret values as tokens.
- [x] A4 Nothing blocks the main thread; a failed or forbidden kind is a line, not a failure.
- [x] A5 With two namespaces in scope a drawer link reads `namespace/name`.

## As built (2026-10-07)

- Data: `crates/cluster/src/namespace_compare.rs`; `clean_value` was split out of `clean_yaml` (`object_edit.rs`) and `field_changes` made `pub(crate)` (`edit_preview.rs`).
- View: `namespace_compare_rows.rs` (pure lines), `namespace_compare_view.rs` (dialog), `namespace_compare_flow.rs` (the shell entry points). Keys: `V` on a Namespaces row, `Ctrl+Shift+D` for the command; both on the sheet.
- The palette pairs `Compare with…` with every namespace (like View YAML), and lists `Compare namespaces` as an Actions command.
- P33 lives in `drawer::link_text`; the scope is read through the `ActiveConnection` global's session.
- Not done: ticking two rows plus a selection bar action (optional in the brief); a virtual list for very long comparisons (the lines are one scroll column).
