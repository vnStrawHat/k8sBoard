# 0006 · App: event drawer, menu, reveal

[Back to index](README.md) · Step 2 · Modules: `kind_row.rs`, `event_rows.rs`, `kind_drawer.rs`, `resource_actions.rs`, `table_selection.rs`, `app_shell.rs`

## `KindRow` additions (`kind_row.rs`)

```rust
pub(crate) struct KindRow { /* 0005 fields */ pub(crate) event: Option<EventDetail> } // None for every other kind
/// Events only: what the drawer header, subtitle, and menu need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventDetail {
    pub(crate) title: SharedString,          // "BackOff · api-7d9f8c-x2k4q"; the object name alone without a reason
    pub(crate) object: Option<ResourceKey>,  // object_key(..): None when k8sBoard has no screen for the kind
    pub(crate) source: Option<SharedString>, // "kubelet on ip-10-0-1-23", for the subtitle
    pub(crate) message: SharedString,        // the full (trimmed, truncated) message, for Copy message
}
pub(crate) enum DetailRow { /* 0005 variants */
    /// Preformatted text that wraps: mono, `text_xs`, `bg(theme.muted)`, rounded, padded.
    Code(SharedString),
    /// A label and a clickable mono value that reveals `target`.
    Link { label: SharedString, text: SharedString, target: ResourceKey },
}
```

## Sections (built by `event_row`)

| Section | Rows |
|---|---|
| **Details** (W7) | Object: `Link` to `object_key` with text `{namespace}/{object_text}` (no prefix without a namespace), or `Field` Mono when `object_key` is `None`. Then the W7 rows: Reason (`Absent` when empty), Count, First seen (`Age`), Last seen (`Age`), Source (`text_or_absent`). Then Name (Mono, the event name) |
| **Message** | `Code(message)`; `Note("No message")` when empty |

The full message is one `SharedString`, cloned (Arc) into `Code` and `EventDetail.message`.

## Drawer (`kind_drawer.rs`)

- Header name: `row.event.title` when set, else `row.name`.
- Subtitle (W7 meta): the toned type, then muted `· {namespace} · {source}` for events (source skipped when `None`); other kinds keep `· {namespace} · created …`. Events have `created_at: None`.
- `detail_element` takes `&Context<AppShell>` (listeners). `Code` and `Link` render as above; `Link` is `wide_detail_row(label, value)` where the value has `id(("link", id))`, `cursor_pointer`, `text_color(theme.link)`, hover underline, tooltip "Open {text}", and `on_click` → `shell.reveal(target.clone(), cx)`.
- Labels only when `kind.has_labels()`.
- `kind_menu_button` passes `cx.weak_entity()` to `kind_menu`.

## Menu (`resource_actions.rs`)

`kind_menu(menu, kind, row, access, shell: &WeakEntity<AppShell>)`. Groups, with separators between non-empty groups only:

| Group | Items |
|---|---|
| event (only when `row.event` is set) | **Go to object**: enabled when `object` is `Some`, `on_click` → `shell.update(cx, \|shell, cx\| shell.reveal(key, cx))`; else `disabled_menu_item("Go to object", "No screen for this kind yet")`. **Copy message**: writes `message` to the clipboard |
| view | View YAML (disabled, `YAML_DEFERRED_REASON`); Port-forward when `has_port_forward` |
| change | `read_only_actions` (none for Events) |
| copy | Copy name |
| danger | `delete_label`, disabled "Read-only mode" |

Clipboard writes happen only on these explicit clicks; nothing is logged.

## Reveal (`app_shell.rs`, `table_selection.rs`)

```rust
impl ResourceKey { pub(crate) fn screen(&self) -> Screen; } // Pod → Pods, Node → Nodes, Kind { kind } → Kind(kind)
impl AppShell {
    /// Opens the key's screen with its row selected; replaces `reveal_pod`.
    pub(crate) fn reveal(&mut self, key: ResourceKey, cx: &mut Context<Self>);
}
```

1. `show_screen(key.screen())` (closes the drawer, starts or keeps the explorer watch).
2. `change_selection(Some(key))`.
3. `sync_selection(cx)`. A loaded list selects the row (`Move`) or drops the key (`Clear`). A loading kind list keeps the key, and `on_session_changed` resolves it after the first snapshot.

- `apply_selection_sync`'s `Clear` arm calls `change_selection(None, cx)` instead of writing `self.selected` (needed in step 3, harmless now).
- `kind_drawer`'s related-pod rows call `reveal(ResourceKey::of_pod(pod))`.

## Screenshots (step 2)

`events` and `events-drawer` come from `from_plural` with no new code. Update `USAGE` (add `events`). `events-drawer` opens row 0, the newest event.
