# 0032b addendum: Edit labels / annotations (UX round 3, N4)

`SetObjectMetadata { labels, annotations }` of a Pod, Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, or CronJob. Modules: `metadata_edits.rs` (pure builder), `metadata_editor.rs` (dialog), `object_metadata.rs` (cluster: read and patch body); gate `AccessCheck::Patch(kind)`, asked lazily with the other per-kind checks (`kind_access.rs`).

## Behavior

- Entry: the row menu and the drawer menu (`Edit labels / annotations…`), the palette, and the unbound key action `EditMetadata`. The gate (lock, permission) is read again when the dialog opens.
- The dialog reads the object fresh (`object_metadata`, one GET) and shows two lists of key and value rows (Labels, Annotations) with Add and Remove. Review… (Enter in a field) is off while a row is invalid or nothing changed.
- Annotations that embed an applied manifest (`kubectl.kubernetes.io/last-applied-configuration`, `kapp.k14s.io/original*`) are never read into the dialog or sent: a muted line says how many stay hidden. The write path refuses an edit that names one.
- The confirm lists one change line per key: `metadata.annotations.owner: (none) → infra`, `metadata.labels.app: web → (removed)`. A key with a dot or a slash is bracketed (`metadata.annotations[example.com/owner]`). A value is cut at 60 characters; the value of an annotation under a credential-looking key reads `(hidden)`.
- A pod label change adds a warning: services and controllers match pods by label.
- Risk `Change`; the audit line records the action `Edit labels / annotations` and the keys. A label value is audited like a node label; an annotation value never is (`ChangedField.value` is `None`).

## Wire format and validation

PATCH `application/merge-patch+json`, per-key (`null` removes), no `resourceVersion`: `{"metadata":{"labels":{…},"annotations":{…}}}` with only the lists that changed. `WriteRequest::new` refuses an empty edit, a key that is not a qualified name or repeats in its list, a label value that is not a label value, annotations over 256 KiB in total, and a kind outside the seven above.

## Drawer

The Overview of a pod and of the six workload kinds has an `Annotations` section under Labels, folded into one `N annotations` row until opened (`DrawerState.are_annotations_open`). Summaries carry `annotations: Vec<String>` (`key=value`, at most 50, a value cut at 200 characters on one line, an applied manifest left out, a credential-looking key's value `<hidden>`); the editor itself always reads the object again.
