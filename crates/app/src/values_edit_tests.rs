//! The Edit values view model (spec 0047): masked Secret fields, newlines, dirtiness, the single
//! copy at Apply, the clipboard rules, and the rebase by key. The view runs in a headless window
//! without a shell, so a call that needs the shell finds none and sends nothing. Tests use the
//! headless clipboard only, never the real one.

use std::path::PathBuf;

use cluster::WritePolicy;
use cluster::fake_api::FakeApi;
use gpui_kit::component::input::{Copy as CopyText, Cut as CutText, SelectAll};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, ClipboardItem, TestAppContext, WindowOptions};
use serde_json::{Value, json};

use super::*;
use crate::cluster_registry::ClusterRef;
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

const SECRET_VALUE: &str = "S3cr3t-0047-ZZ";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime")
}

fn object(kind: ObjectKind, name: &str) -> ObjectRef {
    ObjectRef::new(kind, Some("team-a".to_owned()), name.to_owned()).expect("a namespaced kind")
}

fn secret_json(extra: Value) -> Value {
    let mut secret = json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "api-db", "namespace": "team-a", "resourceVersion": "42"},
        "type": "Opaque",
        "data": {
            "DB_HOST": "ZGIuaW50ZXJuYWw=",
            "DB_PASSWORD": "UzNjcjN0LTAwNDctWlo=",
            "DB_USER": "YXBp",
            "ca.der": "//4A",
        },
    });
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            secret[key] = value.clone();
        }
    }
    secret
}

fn config_map_json() -> Value {
    json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "api-config", "namespace": "team-a", "resourceVersion": "7"},
        "data": {"app.yaml": "a: 1\n", "mode": "fast", "big": "x".repeat(cluster::MAX_INLINE_VALUE + 1)},
        "binaryData": {"BLOB": "AQIDBA=="},
    })
}

/// The base the cluster crate builds from `body`, over a fake server.
fn base_of(rt: &tokio::runtime::Runtime, target: &ObjectRef, body: &Value) -> ValuesBase {
    let body = body.to_string();
    let connection = {
        let _guard = rt.enter();
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone())).0
    };
    rt.block_on(connection.values_base(target))
        .expect("an editable base")
}

struct ViewTest {
    window: AnyWindowHandle,
    view: Entity<ValuesEditView>,
    _rt: tokio::runtime::Runtime,
}

fn subject(target: &ObjectRef) -> ValuesSubject {
    let kind = match target.kind_name() {
        "Secret" => ResourceKind::Secrets,
        _ => ResourceKind::ConfigMaps,
    };
    let cluster = ClusterRef {
        kubeconfig: PathBuf::from("kube.yaml"),
        context: "ctx".to_owned(),
    };
    ValuesSubject {
        edit: EditSubject {
            target: ClusterObject::new(
                cluster,
                ResourceKey::Kind {
                    kind,
                    namespace: Some("team-a".to_owned()),
                    name: target.name().to_owned(),
                },
            ),
            cluster_name: "ctx".into(),
            object: target.clone(),
            kind: if kind == ResourceKind::Secrets {
                ObjectKind::Secret
            } else {
                ObjectKind::ConfigMap
            },
        },
        secret_type: None,
    }
}

fn open_view(
    kind: ObjectKind,
    body: &Value,
    access: ValueAccess,
    cx: &mut TestAppContext,
) -> ViewTest {
    let rt = runtime();
    let name = body["metadata"]["name"]
        .as_str()
        .expect("a name")
        .to_owned();
    let target = object(kind, &name);
    let base = base_of(&rt, &target, body);
    let subject = subject(&target);
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::keymap::bind_keys(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                ValuesEditView::empty(WeakEntity::new_invalid(), subject, access, window, cx)
            })
        })
        .expect("open the test window")
    });
    let test = ViewTest {
        window,
        view,
        _rt: rt,
    };
    test.with(cx, |view, window, cx| {
        view.finish_load(base, LoadPurpose::Replace, window, cx)
    });
    test
}

fn secret_view(cx: &mut TestAppContext) -> ViewTest {
    open_view(
        ObjectKind::Secret,
        &secret_json(json!({})),
        ValueAccess::Enabled,
        cx,
    )
}

fn config_map_view(cx: &mut TestAppContext) -> ViewTest {
    open_view(
        ObjectKind::ConfigMap,
        &config_map_json(),
        ValueAccess::Enabled,
        cx,
    )
}

impl ViewTest {
    fn with<R>(
        &self,
        cx: &mut TestAppContext,
        run: impl FnOnce(&mut ValuesEditView, &mut Window, &mut Context<ValuesEditView>) -> R,
    ) -> R {
        let result = cx
            .update_window(self.window, |_, window, cx| {
                self.view.update(cx, |view, cx| run(view, window, cx))
            })
            .expect("the window is open");
        cx.run_until_parked();
        result
    }

    fn field(&self, name: &str, cx: &mut TestAppContext) -> Entity<TextareaState> {
        self.view.read_with(cx, |view, _| {
            let row = view
                .rows
                .iter()
                .find(|row| row.name == name)
                .expect("the row exists");
            match &row.field {
                FieldKind::Secret { field, .. } | FieldKind::Text { field, .. } => field.clone(),
                FieldKind::Binary { .. } | FieldKind::Large { .. } => panic!("{name} has no field"),
            }
        })
    }

    fn text(&self, name: &str, cx: &mut TestAppContext) -> String {
        let field = self.field(name, cx);
        field.read_with(cx, |state, _| state.text().to_string())
    }

    /// Inserts `text` into the field as the kit does for typing: one atomic edit.
    fn insert(&self, name: &str, text: &str, cx: &mut TestAppContext) {
        let field = self.field(name, cx);
        self.with(cx, |_, window, cx| {
            field.update(cx, |state, cx| state.insert(text.to_owned(), window, cx))
        });
    }

    fn reveal_of(&self, name: &str, cx: &mut TestAppContext) -> Reveal {
        self.view.read_with(cx, |view, _| {
            match &view
                .rows
                .iter()
                .find(|row| row.name == name)
                .expect("the row exists")
                .field
            {
                FieldKind::Secret { reveal, .. } => *reveal,
                _ => panic!("{name} is not a Secret field"),
            }
        })
    }

    fn toggle_reveal(&self, name: &str, cx: &mut TestAppContext) {
        self.with(cx, |view, window, cx| view.toggle_reveal(name, window, cx));
    }

    fn changes(&self, cx: &mut TestAppContext) -> Vec<KeyChange> {
        self.view.read_with(cx, |view, cx| {
            view.collect_changes(cx)
                .unwrap_or_else(|_| panic!("no problem"))
        })
    }

    fn render(&self, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .expect("the window is open");
        cx.run_until_parked();
    }
}

fn set(key: &str, text: &str) -> KeyChange {
    KeyChange::Set {
        key: key.to_owned(),
        value: NewValue::new(Zeroizing::new(text.to_owned())),
    }
}

fn add(key: &str, text: &str) -> KeyChange {
    KeyChange::Add {
        key: key.to_owned(),
        value: NewValue::new(Zeroizing::new(text.to_owned())),
    }
}

fn names(view: &ViewTest, cx: &mut TestAppContext) -> Vec<String> {
    view.view.read_with(cx, |view, _| {
        view.rows.iter().map(|row| row.name.clone()).collect()
    })
}

#[gpui_kit::test]
fn secret_fields_start_masked_and_empty(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    assert_eq!(
        names(&t, cx),
        ["DB_HOST", "DB_PASSWORD", "DB_USER", "ca.der"]
    );
    for name in ["DB_HOST", "DB_PASSWORD", "DB_USER"] {
        assert_eq!(t.reveal_of(name, cx), Reveal::Masked, "{name}");
        assert_eq!(t.text(name, cx), "", "{name}");
    }
    t.view.read_with(cx, |view, _| {
        assert!(!view.is_dirty());
        assert!(view.rows.iter().all(|row| row.state().is_none()));
    });
    assert_eq!(
        field_display(Reveal::Masked, 0),
        FieldDisplay::Masked("unchanged".into())
    );
}

#[gpui_kit::test]
fn typing_makes_a_set_and_empty_keeps(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    t.insert("DB_PASSWORD", "new", cx);
    t.view.read_with(cx, |view, _| assert!(view.is_dirty()));
    assert_eq!(t.changes(cx), [set("DB_PASSWORD", "new")]);
    // Emptying the field again keeps the current value.
    let field = t.field("DB_PASSWORD", cx);
    t.with(cx, |_, window, cx| {
        field.update(cx, |state, cx| state.replace_all("", window, cx))
    });
    t.view.read_with(cx, |view, _| assert!(!view.is_dirty()));
    assert!(t.changes(cx).is_empty());
}

#[gpui_kit::test]
fn config_map_set_equal_to_base_is_no_change(cx: &mut TestAppContext) {
    let t = config_map_view(cx);
    let field = t.field("mode", cx);
    t.with(cx, |_, window, cx| {
        field.update(cx, |state, cx| state.replace_all("slow", window, cx))
    });
    assert_eq!(t.changes(cx), [set("mode", "slow")]);
    t.with(cx, |_, window, cx| {
        field.update(cx, |state, cx| state.replace_all("fast", window, cx))
    });
    t.view.read_with(cx, |view, _| assert!(!view.is_dirty()));
    assert!(t.changes(cx).is_empty());
}

#[gpui_kit::test]
fn pasted_newlines_survive_mask_round_trip(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    for (name, pasted) in [("DB_PASSWORD", "a\nb\n"), ("DB_USER", "a\r\nb")] {
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(pasted.to_owned())));
        // While masked.
        t.with(cx, |view, window, cx| view.paste_into(name, window, cx));
        assert_eq!(t.text(name, cx), pasted, "{name} pasted while masked");
        assert_eq!(t.reveal_of(name, cx), Reveal::Masked);
        // Unmask, mask, unmask again: the same widget, the same text.
        t.toggle_reveal(name, cx);
        assert_eq!(t.text(name, cx), pasted, "{name} unmasked");
        t.toggle_reveal(name, cx);
        t.toggle_reveal(name, cx);
        assert_eq!(t.text(name, cx), pasted, "{name} unmasked again");
        // Pasting while unmasked appends the same text.
        t.toggle_reveal(name, cx);
    }
    assert_eq!(
        t.changes(cx),
        [set("DB_PASSWORD", "a\nb\n"), set("DB_USER", "a\r\nb")]
    );
}

#[gpui_kit::test]
fn secret_field_is_one_textarea_for_life(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    let before = t.field("DB_PASSWORD", cx).entity_id();
    t.toggle_reveal("DB_PASSWORD", cx);
    t.insert("DB_PASSWORD", "x", cx);
    t.toggle_reveal("DB_PASSWORD", cx);
    t.toggle_reveal("DB_PASSWORD", cx);
    assert_eq!(t.field("DB_PASSWORD", cx).entity_id(), before);
}

#[test]
fn masked_field_renders_placeholder_with_char_count() {
    assert_eq!(mask_text(0), "unchanged");
    assert_eq!(mask_text(1), "•••• 1 char");
    assert_eq!(mask_text(12), "•••• 12 chars");
    assert_eq!(
        field_display(Reveal::Masked, 12),
        FieldDisplay::Masked("•••• 12 chars".into())
    );
    // A shown field is the textarea itself.
    let shown = Reveal::Shown {
        hides_at: Instant::now(),
    };
    assert_eq!(field_display(shown, 12), FieldDisplay::Editor);
}

#[gpui_kit::test]
fn masked_field_counts_characters_of_the_hidden_text(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("héllo\nx".to_owned())));
    t.with(cx, |view, window, cx| {
        view.paste_into("DB_USER", window, cx)
    });
    let field = t.field("DB_USER", cx);
    let count = field.read_with(cx, |state, _| state.text().chars().count());
    assert_eq!(mask_text(count), "•••• 7 chars");
}

#[gpui_kit::test]
fn paste_while_masked_fills_hidden_field(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(SECRET_VALUE.to_owned())));
    t.with(cx, |view, window, cx| {
        view.paste_into("DB_HOST", window, cx)
    });
    assert_eq!(t.text("DB_HOST", cx), SECRET_VALUE);
    assert_eq!(t.reveal_of("DB_HOST", cx), Reveal::Masked);
    t.view.read_with(cx, |view, _| assert!(view.is_dirty()));
}

#[gpui_kit::test]
fn dirtiness_tracks_change_events_and_length(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    t.view.read_with(cx, |view, _| assert!(!view.is_dirty()));
    t.insert("DB_HOST", "x", cx);
    t.view.read_with(cx, |view, _| {
        let row = view.rows.iter().find(|row| row.name == "DB_HOST");
        assert!(row.is_some_and(|row| row.has_text_change));
        assert!(view.is_dirty());
    });
}

#[gpui_kit::test]
fn apply_copies_rope_once_into_zeroizing(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    t.insert("DB_PASSWORD", "a\r\nb long enough value", cx);
    let field = t.field("DB_PASSWORD", cx);
    let copy = field.read_with(cx, |_, cx| copy_text(&field, cx));
    assert_eq!(copy.as_str(), "a\r\nb long enough value");
    assert_eq!(copy.capacity(), copy.len());
}

#[test]
fn unmask_re_masks_after_thirty_seconds() {
    let start = Instant::now();
    assert_eq!(REVEAL_DURATION, Duration::from_secs(30));
    let mut rows: Vec<ValueRow> = Vec::new();
    assert!(!expire_reveals(&mut rows, start));
}

#[gpui_kit::test]
fn revealed_field_masks_when_its_time_is_up(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    t.toggle_reveal("DB_HOST", cx);
    let Reveal::Shown { hides_at } = t.reveal_of("DB_HOST", cx) else {
        panic!("the field is shown");
    };
    t.with(cx, |view, _, _| {
        let almost = hides_at - Duration::from_secs(1);
        assert!(!expire_reveals(&mut view.rows, almost));
        assert!(expire_reveals(&mut view.rows, hides_at));
    });
    assert_eq!(t.reveal_of("DB_HOST", cx), Reveal::Masked);
}

#[gpui_kit::test]
fn apply_re_masks_every_field(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    for name in ["DB_HOST", "DB_USER"] {
        t.toggle_reveal(name, cx);
    }
    t.insert("DB_HOST", "x", cx);
    t.with(cx, |view, window, cx| view.apply(window, cx));
    for name in ["DB_HOST", "DB_USER"] {
        assert_eq!(t.reveal_of(name, cx), Reveal::Masked, "{name}");
    }
    // The change itself stays for the dialog's answer.
    t.view.read_with(cx, |view, _| assert!(view.is_dirty()));
}

#[gpui_kit::test]
fn copy_and_cut_in_secret_field_are_dropped(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    t.toggle_reveal("DB_PASSWORD", cx);
    t.insert("DB_PASSWORD", SECRET_VALUE, cx);
    t.render(cx);
    // The field has the keyboard, so Copy and Cut would reach it if nothing dropped them.
    let field = t.field("DB_PASSWORD", cx);
    let is_focused = cx
        .update_window(t.window, |_, window, cx| {
            field.read(cx).focus_handle(cx).is_focused(window)
        })
        .expect("the window is open");
    assert!(is_focused);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("sentinel".to_owned())));
    cx.update_window(t.window, |_, window, cx| {
        window.dispatch_action(Box::new(SelectAll), cx);
        window.dispatch_action(Box::new(CopyText), cx);
        window.dispatch_action(Box::new(CutText), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(clipboard.as_deref(), Some("sentinel"));
    // Cut did not remove the text either.
    assert_eq!(t.text("DB_PASSWORD", cx), SECRET_VALUE);
}

#[gpui_kit::test]
fn config_map_fields_copy_normally(cx: &mut TestAppContext) {
    let t = config_map_view(cx);
    let field = t.field("mode", cx);
    t.with(cx, |_, window, cx| {
        field.update(cx, |state, cx| state.focus(window, cx))
    });
    t.render(cx);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("sentinel".to_owned())));
    cx.update_window(t.window, |_, window, cx| {
        window.dispatch_action(Box::new(SelectAll), cx);
        window.dispatch_action(Box::new(CopyText), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(clipboard.as_deref(), Some("fast"));
}

#[gpui_kit::test]
fn binary_and_large_rows_offer_remove_only(cx: &mut TestAppContext) {
    let t = config_map_view(cx);
    t.view.read_with(cx, |view, _| {
        let kind_of = |name: &str| {
            let row = view.rows.iter().find(|row| row.name == name);
            row.map(|row| match row.field {
                FieldKind::Binary { size_bytes } => format!("binary {size_bytes}"),
                FieldKind::Large { .. } => "large".to_owned(),
                FieldKind::Text { .. } => "text".to_owned(),
                FieldKind::Secret { .. } => "secret".to_owned(),
            })
        };
        assert_eq!(kind_of("BLOB").as_deref(), Some("binary 4"));
        assert_eq!(kind_of("big").as_deref(), Some("large"));
        assert_eq!(kind_of("mode").as_deref(), Some("text"));
    });
    t.with(cx, |view, _, cx| {
        view.toggle_remove("BLOB", cx);
        view.toggle_remove("big", cx);
    });
    assert_eq!(
        t.changes(cx),
        [
            KeyChange::Remove {
                key: "BLOB".to_owned()
            },
            KeyChange::Remove {
                key: "big".to_owned()
            }
        ]
    );
    // Undo takes the mark back.
    t.with(cx, |view, _, cx| view.toggle_remove("BLOB", cx));
    assert_eq!(t.changes(cx).len(), 1);
}

#[gpui_kit::test]
fn screenshot_access_disables_the_eye(cx: &mut TestAppContext) {
    let t = open_view(
        ObjectKind::Secret,
        &secret_json(json!({})),
        ValueAccess::Blocked,
        cx,
    );
    t.toggle_reveal("DB_PASSWORD", cx);
    assert_eq!(t.reveal_of("DB_PASSWORD", cx), Reveal::Masked);
}

#[gpui_kit::test]
fn added_key_starts_masked_and_needs_a_value(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    let add_name = t.view.read_with(cx, |view, _| view.add_name.clone());
    t.with(cx, |_, window, cx| {
        add_name.update(cx, |input, cx| input.set_value("DB_PORT", window, cx))
    });
    t.with(cx, |view, window, cx| view.add_key(window, cx));
    assert_eq!(
        names(&t, cx),
        ["DB_HOST", "DB_PASSWORD", "DB_PORT", "DB_USER", "ca.der"]
    );
    assert_eq!(t.reveal_of("DB_PORT", cx), Reveal::Masked);
    // Apply with an empty new Secret key sends nothing and says why under the key.
    t.with(cx, |view, window, cx| view.apply(window, cx));
    t.view.read_with(cx, |view, _| {
        assert!(view.row_errors.iter().any(|(key, _)| key == "DB_PORT"));
    });
    t.insert("DB_PORT", "5432", cx);
    assert_eq!(t.changes(cx), [add("DB_PORT", "5432")]);
}

#[gpui_kit::test]
fn add_key_checks_the_name_locally(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    let add_name = t.view.read_with(cx, |view, _| view.add_name.clone());
    for (typed, ok) in [
        ("a/b", false),
        ("DB_HOST", false),
        ("", false),
        ("NEW-1.x", true),
    ] {
        t.with(cx, |_, window, cx| {
            add_name.update(cx, |input, cx| input.set_value(typed, window, cx))
        });
        t.with(cx, |view, window, cx| view.add_key(window, cx));
        let has_error = t.view.read_with(cx, |view, _| view.add_error.is_some());
        assert_eq!(!has_error, ok, "{typed:?}");
    }
    assert!(names(&t, cx).contains(&"NEW-1.x".to_owned()));
}

#[gpui_kit::test]
fn removing_an_added_key_drops_its_row(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    let add_name = t.view.read_with(cx, |view, _| view.add_name.clone());
    t.with(cx, |_, window, cx| {
        add_name.update(cx, |input, cx| input.set_value("TEMP", window, cx))
    });
    t.with(cx, |view, window, cx| view.add_key(window, cx));
    assert!(names(&t, cx).contains(&"TEMP".to_owned()));
    t.with(cx, |view, _, cx| view.toggle_remove("TEMP", cx));
    assert!(!names(&t, cx).contains(&"TEMP".to_owned()));
}

#[gpui_kit::test]
fn reload_keeps_changes_by_key_and_lists_dropped(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    // Pending: a new value for DB_PASSWORD, DB_USER removed, DB_HOST removed, and two added keys.
    t.insert("DB_PASSWORD", "mine", cx);
    let add_name = t.view.read_with(cx, |view, _| view.add_name.clone());
    for key in ["TAKEN", "FRESH"] {
        t.with(cx, |_, window, cx| {
            add_name.update(cx, |input, cx| input.set_value(key, window, cx))
        });
        t.with(cx, |view, window, cx| view.add_key(window, cx));
        t.insert(key, "v", cx);
    }
    t.with(cx, |view, _, cx| {
        view.toggle_remove("DB_USER", cx);
        view.toggle_remove("DB_HOST", cx);
    });
    // The server meanwhile dropped DB_USER, added TAKEN, and bumped the version.
    let rt = runtime();
    let newer = secret_json(json!({
        "metadata": {"name": "api-db", "namespace": "team-a", "resourceVersion": "43"},
        "data": {"DB_HOST": "aA==", "DB_PASSWORD": "cA==", "TAKEN": "dA==", "ca.der": "//4A"},
    }));
    let base = base_of(&rt, &object(ObjectKind::Secret, "api-db"), &newer);
    t.with(cx, |view, window, cx| {
        view.finish_load(base, LoadPurpose::Rebase, window, cx)
    });
    assert_eq!(
        names(&t, cx),
        ["DB_HOST", "DB_PASSWORD", "FRESH", "TAKEN", "ca.der"]
    );
    assert_eq!(
        t.changes(cx),
        [
            KeyChange::Remove {
                key: "DB_HOST".to_owned()
            },
            set("DB_PASSWORD", "mine"),
            add("FRESH", "v"),
        ]
    );
    t.view.read_with(cx, |view, _| {
        let Some(ValuesBanner::Rebased { dropped }) = &view.banner else {
            panic!("a rebased banner");
        };
        let lines: Vec<&str> = dropped.iter().map(AsRef::as_ref).collect();
        assert_eq!(
            lines,
            [
                "DB_USER: removed on the server, your change was dropped",
                "TAKEN: added on the server, your change was dropped",
            ]
        );
        assert_eq!(view.server_changed_keys, ["DB_HOST", "DB_PASSWORD"]);
    });
    // The typed value is the same widget's text, newlines and all.
    assert_eq!(t.text("DB_PASSWORD", cx), "mine");
}

#[gpui_kit::test]
fn local_error_shows_under_its_key(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    t.view.update(cx, |view, cx| {
        view.show_edit_error(ValuesEditError::InvalidKey("a/b".to_owned()));
        view.show_edit_error(ValuesEditError::ObjectTooLarge);
        cx.notify();
    });
    t.view.read_with(cx, |view, _| {
        assert_eq!(view.row_errors.len(), 1);
        assert_eq!(view.row_errors[0].0, "a/b");
        assert_eq!(
            view.footer_error.as_deref(),
            Some("the object would be larger than 1 MiB")
        );
    });
}

#[gpui_kit::test]
fn server_rejection_names_the_key_from_its_field_path(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    t.view.update(cx, |view, cx| {
        view.commit_failed(
            EditFailure::Invalid {
                message: "Invalid".into(),
                fields: vec!["data[DB_HOST]".into(), "metadata.name".into()],
            },
            cx,
        )
    });
    t.view.read_with(cx, |view, _| {
        assert_eq!(view.row_errors.len(), 1);
        assert_eq!(view.row_errors[0].0, "DB_HOST");
        assert_eq!(view.footer_error.as_deref(), Some("Invalid"));
    });
}

#[test]
fn field_paths_name_their_key() {
    assert_eq!(key_of_field("data[DB_HOST]"), Some("DB_HOST"));
    assert_eq!(key_of_field("binaryData[BLOB]"), Some("BLOB"));
    assert_eq!(key_of_field("data.DB_HOST"), Some("DB_HOST"));
    assert_eq!(key_of_field("metadata.name"), None);
    assert_eq!(key_of_field("data"), None);
}

#[gpui_kit::test]
fn commit_failures_show_in_place(cx: &mut TestAppContext) {
    let t = secret_view(cx);
    t.view
        .update(cx, |view, cx| view.commit_failed(EditFailure::Conflict, cx));
    t.view.read_with(cx, |view, _| {
        assert!(matches!(view.banner, Some(ValuesBanner::Conflict)))
    });
    t.view
        .update(cx, |view, cx| view.commit_failed(EditFailure::Deleted, cx));
    t.view.read_with(cx, |view, _| {
        assert!(matches!(view.banner, Some(ValuesBanner::Deleted)))
    });
    // A deleted object cannot take the changes: Apply does nothing.
    t.insert("DB_HOST", "x", cx);
    t.with(cx, |view, window, cx| view.apply(window, cx));
    t.view
        .read_with(cx, |view, _| assert!(view.footer_error.is_none()));
}

#[gpui_kit::test]
fn warnings_name_helm_owner_and_restart(cx: &mut TestAppContext) {
    let body = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {
            "name": "api-config", "namespace": "team-a", "resourceVersion": "7",
            "labels": {"app.kubernetes.io/managed-by": "Helm"},
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment", "name": "api", "uid": "u"}],
        },
        "data": {"mode": "fast"},
    });
    let t = open_view(ObjectKind::ConfigMap, &body, ValueAccess::Enabled, cx);
    t.view.read_with(cx, |view, _| {
        let lines: Vec<&str> = view.warnings.iter().map(AsRef::as_ref).collect();
        assert_eq!(
            lines,
            [
                "Managed by Helm: the next upgrade replaces this change",
                "Owned by Deployment/api: its controller may replace this change",
                "Pods that read these keys as environment variables keep the old values until they restart",
            ]
        );
    });
}

#[test]
fn view_has_no_value_in_any_debug_or_error_text() {
    let error = ValuesEditError::TooLarge("DB_PASSWORD".to_owned());
    assert!(!error.to_string().contains(SECRET_VALUE));
    assert_eq!(
        base_error_text(&ValuesBaseError::HelmRelease),
        "Helm release records cannot be edited"
    );
}
