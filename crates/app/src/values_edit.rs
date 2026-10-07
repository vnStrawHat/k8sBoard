//! The Edit values view (spec 0047, wireframe W7): one row per key of a ConfigMap or Secret, with
//! add, change, and remove. It replaces the table and the drawer in the workspace while it is open,
//! in the slot it shares with Edit YAML.
//!
//! Secret values are write-only here: the editor never fetches or shows a current value. Each Secret
//! value field is ONE `TextareaState` for its whole life, so a pasted `\n` or `\r\n` survives; the
//! editor's own mask only swaps the rendered element for a `•••• N chars` placeholder. The kit mask
//! and `Input::mask_toggle()` are never used, and the view never calls `value()` on a Secret field.
//!
//! Nothing here logs, writes to disk, or sends a request itself: Apply hands a `WriteIntent` to the
//! write flow (dry-run, confirm tier, commit, audit), always on the cluster of the edited object.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cluster::{
    ClusterConnection, HELM_MANAGED_WARNING, KeyChange, KeyContent, NewValue, ObjectKind,
    ObjectRef, ValueKey, ValuesBase, ValuesBaseError, ValuesEditError, WriteOperation,
    WriteRequest, is_valid_key_name,
};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, KeyDownEvent, SharedString,
    Subscription, Task, WeakEntity, Window,
};
use zeroize::Zeroizing;

use crate::app_shell::AppShell;
use crate::app_shell::write_flow::WriteIntent;
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::resource_actions::ResourceAction;
use crate::secret_values::{REVEAL_DURATION, ValueAccess};
use crate::table_selection::ClusterObject;
use crate::write_guard::ActionRisk;
use crate::yaml_edit::{EditFailure, EditSubject};

const TICK: Duration = Duration::from_secs(1);
/// Always shown under the title: a change of values reaches a running pod only when it reads them
/// again.
const RESTART_WARNING: &str =
    "Pods that read these keys as environment variables keep the old values until they restart";

/// What the editor opens on: the edit subject and the type chip of a Secret.
pub(crate) struct ValuesSubject {
    pub(crate) edit: EditSubject,
    /// The Secret `type` from the row, for the chip; `None` for a ConfigMap.
    pub(crate) secret_type: Option<SharedString>,
}

/// Where the first read of the object stands.
enum LoadState {
    Loading {
        _task: Task<()>,
    },
    Ready,
    /// The object cannot be edited (a refusal) or could not be read; only Close is offered.
    Failed(SharedString),
}

/// A condition of the object itself, shown over the rows.
pub(crate) enum ValuesBanner {
    /// A 409, or a newer `resourceVersion` than the one the edit started from.
    Conflict,
    /// The changes were moved onto the newer object; these had no place on it.
    Rebased { dropped: Vec<SharedString> },
    /// A 404: the changes are kept but cannot be applied.
    Deleted,
    /// A commit whose request may have left the client before it failed.
    OutcomeUnknown,
}

/// What a load of the object is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadPurpose {
    /// The first read and Discard: the rows are built from the object.
    Replace,
    /// Reload and keep my changes: the pending changes move onto the new object by key.
    Rebase,
}

/// Whether a Secret field is shown. Only the editor's own eye button unmasks one field, and its
/// timer, Apply, and close mask it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reveal {
    Masked,
    Shown { hides_at: Instant },
}

pub(crate) enum FieldKind {
    /// Secret text: write-only, empty keeps the current value.
    Secret {
        field: Entity<TextareaState>,
        reveal: Reveal,
    },
    /// ConfigMap text, edited as typed. `original` is the server's text.
    Text {
        field: Entity<TextareaState>,
        original: String,
    },
    Binary {
        size_bytes: usize,
    },
    /// ConfigMap text over the inline cap: only Remove.
    Large {
        size_bytes: usize,
    },
}

pub(crate) enum RowOrigin {
    Server { is_removed: bool },
    Added,
}

pub(crate) struct ValueRow {
    pub(crate) name: String,
    pub(crate) origin: RowOrigin,
    pub(crate) field: FieldKind,
    /// The field holds a change: text typed in a Secret field, or ConfigMap text that differs from
    /// the server's. Set from `InputEvent::Change`, never from a per-render read of the text.
    pub(crate) has_text_change: bool,
    /// Characters of a Secret field's text, for the mask placeholder. Set with `has_text_change`, so
    /// drawing a frame never walks the rope.
    pub(crate) char_count: usize,
    /// A Secret field whose text ends with a line break, usually a paste. Set with `char_count`.
    pub(crate) ends_with_line_break: bool,
    _subscription: Option<Subscription>,
}

/// How a row reads next to its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    Added,
    Changed,
    Removed,
}

impl ValueRow {
    fn is_removed(&self) -> bool {
        matches!(self.origin, RowOrigin::Server { is_removed: true })
    }

    pub(crate) fn state(&self) -> Option<RowState> {
        match self.origin {
            RowOrigin::Added => Some(RowState::Added),
            RowOrigin::Server { is_removed: true } => Some(RowState::Removed),
            RowOrigin::Server { .. } if self.has_text_change => Some(RowState::Changed),
            RowOrigin::Server { .. } => None,
        }
    }
}

/// What a Secret field shows in place of the textarea while masked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FieldDisplay {
    /// `•••• 12 chars`, or `unchanged` for an empty field.
    Masked(SharedString),
    /// The textarea itself.
    Editor,
}

/// The text of the mask. Only the count of characters shows; an empty field keeps the value.
pub(crate) fn mask_text(char_count: usize) -> SharedString {
    match char_count {
        0 => "unchanged".into(),
        1 => "•••• 1 char".into(),
        count => format!("•••• {count} chars").into(),
    }
}

pub(crate) fn field_display(reveal: Reveal, char_count: usize) -> FieldDisplay {
    match reveal {
        Reveal::Masked => FieldDisplay::Masked(mask_text(char_count)),
        Reveal::Shown { .. } => FieldDisplay::Editor,
    }
}

/// Masks every field whose time is up; true when any changed.
pub(crate) fn expire_reveals(rows: &mut [ValueRow], now: Instant) -> bool {
    let mut changed = false;
    for row in rows {
        if let FieldKind::Secret { reveal, .. } = &mut row.field
            && matches!(*reveal, Reveal::Shown { hides_at } if hides_at <= now)
        {
            *reveal = Reveal::Masked;
            changed = true;
        }
    }
    changed
}

/// Copies the text once, from the rope's chunks into a buffer of exactly its length (so the buffer
/// never grows and leaves an unwiped earlier copy), wiped on drop. `value()` is never called: it
/// would make an `Arc<str>` copy the kit owns.
pub(crate) fn copy_text(field: &Entity<TextareaState>, cx: &App) -> Zeroizing<String> {
    let rope = field.read(cx).text();
    let mut copy = Zeroizing::new(String::with_capacity(rope.len()));
    for chunk in rope.chunks() {
        copy.push_str(chunk);
    }
    copy
}

/// Inserts the pasted text and gives the wrapper back untouched: the caller drops it, which wipes
/// the clipboard copy. The kit gets its own copy through a borrow; moving the `String` out of the
/// wrapper would leave nothing to wipe.
fn insert_clipboard_text(
    field: &Entity<TextareaState>,
    text: Zeroizing<String>,
    window: &mut Window,
    cx: &mut App,
) -> Zeroizing<String> {
    field.update(cx, |state, cx| {
        state.insert(SharedString::from(text.as_str()), window, cx)
    });
    text
}

/// A row problem found before any request.
struct RowProblem {
    key: String,
    text: SharedString,
}

/// A change waiting to be applied, by key: what a rebase moves onto the newer object.
enum Pending {
    Add,
    Set,
    Remove,
}

/// Counts the editors opened in this run, so a commit can tell which one it was started from.
static NEXT_OPEN_ID: AtomicU64 = AtomicU64::new(1);

/// The open values edit, in `AppShell.edit`.
pub(crate) struct ValuesEditView {
    /// Told to the shell when Apply starts a write, so a commit that finishes after this editor was
    /// closed and another opened does not act on the new one.
    open_id: u64,
    shell: WeakEntity<AppShell>,
    /// The cursor's cluster and key; the guard, the connection, the tier, and the audit line are
    /// resolved from this slot for every request.
    target: ClusterObject,
    cluster_name: SharedString,
    object: ObjectRef,
    kind: ObjectKind,
    secret_type: Option<SharedString>,
    access: ValueAccess,
    load: LoadState,
    /// `None` while loading and in a fixture.
    base: Option<ValuesBase>,
    resource_version: Option<SharedString>,
    rows: Vec<ValueRow>,
    add_name: Entity<InputState>,
    add_error: Option<SharedString>,
    banner: Option<ValuesBanner>,
    /// An error under the key it belongs to; cleared when the row is edited.
    row_errors: Vec<(String, SharedString)>,
    footer_error: Option<SharedString>,
    /// Non-blocking lines: Helm managed, owned, and the restart note.
    warnings: Vec<SharedString>,
    /// Keys whose kept change meets a server that changed since the editor opened (decision 12).
    server_changed_keys: Vec<String>,
    /// A reload in flight; dropping the task cancels it.
    reload: Option<Task<()>>,
    /// A held Ctrl S was seen in this key event (see `apply_from_key`).
    is_apply_key_held: bool,
    is_ticking: bool,
    _ticker: Option<Task<()>>,
    focus_handle: FocusHandle,
    /// `--screen values-edit`: a picture drawn from fixed data that sends nothing.
    #[cfg(feature = "screenshot")]
    is_fixture: bool,
    _subscription: Subscription,
}

impl ValuesEditView {
    /// Opens the editor on `subject.edit.object` and reads it over `connection`, the one of the
    /// object's own cluster, which the caller resolved: the shell is being updated while this runs,
    /// so the view cannot ask it. The caller has checked the gate.
    pub(crate) fn new(
        shell: WeakEntity<AppShell>,
        subject: ValuesSubject,
        access: ValueAccess,
        connection: Option<ClusterConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::empty(shell, subject, access, window, cx);
        view.begin_load(connection, LoadPurpose::Replace, window, cx);
        view
    }

    /// An editor with nothing loaded: no base, no rows, no request.
    fn empty(
        shell: WeakEntity<AppShell>,
        subject: ValuesSubject,
        access: ValueAccess,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ValuesSubject { edit, secret_type } = subject;
        let EditSubject {
            target,
            cluster_name,
            object,
            kind,
        } = edit;
        let add_name = cx.new(|cx| InputState::new(window, cx).placeholder("NEW_KEY"));
        let subscription = cx.subscribe_in(
            &add_name,
            window,
            |view, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => view.add_key(window, cx),
                InputEvent::Change => {
                    view.add_error = None;
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        );
        Self {
            open_id: NEXT_OPEN_ID.fetch_add(1, Ordering::Relaxed),
            shell,
            target,
            cluster_name,
            object,
            kind,
            secret_type,
            access,
            load: LoadState::Ready,
            base: None,
            resource_version: None,
            rows: Vec::new(),
            add_name,
            add_error: None,
            banner: None,
            row_errors: Vec::new(),
            footer_error: None,
            warnings: Vec::new(),
            server_changed_keys: Vec::new(),
            reload: None,
            is_apply_key_held: false,
            is_ticking: false,
            _ticker: None,
            focus_handle: cx.focus_handle(),
            #[cfg(feature = "screenshot")]
            is_fixture: false,
            _subscription: subscription,
        }
    }

    pub(crate) fn open_id(&self) -> u64 {
        self.open_id
    }

    pub(crate) fn cluster(&self) -> &ClusterRef {
        &self.target.cluster
    }

    pub(crate) fn object(&self) -> &ObjectRef {
        &self.object
    }

    /// Whether any key was added, removed, or given a new value.
    pub(crate) fn is_dirty(&self) -> bool {
        self.rows.iter().any(|row| row.state().is_some())
    }

    /// `Secret/payments/api-db`: what the discard prompt and the leaving dialog call the edit.
    pub(crate) fn subject_text(&self) -> String {
        match self.object.namespace() {
            Some(namespace) => format!(
                "{}/{namespace}/{}",
                self.object.kind_name(),
                self.object.name()
            ),
            None => format!("{}/{}", self.object.kind_name(), self.object.name()),
        }
    }

    fn is_secret(&self) -> bool {
        self.kind == ObjectKind::Secret
    }

    fn is_running(&self) -> bool {
        self.reload.is_some()
    }

    fn change_count(&self) -> usize {
        self.rows.iter().filter(|row| row.state().is_some()).count()
    }

    // ---- loading ----

    /// Reads the object again over the connection of its own cluster, which the shell knows now.
    /// For the commands of an open edit: they run from the view's own events, so the shell can be
    /// asked.
    fn reload_with(&mut self, purpose: LoadPurpose, window: &mut Window, cx: &mut Context<Self>) {
        let connection = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).edit_connection(&self.target.cluster, cx));
        self.begin_load(connection, purpose, window, cx);
    }

    fn begin_load(
        &mut self,
        connection: Option<ClusterConnection>,
        purpose: LoadPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = connection else {
            self.fail_load(format!("{} is not open", self.cluster_name).into(), cx);
            return;
        };
        let object = self.object.clone();
        let runtime = cx.global::<ClusterRuntime>().clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let read = runtime
                .spawn(async move { connection.values_base(&object).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| match read {
                Ok(Ok(base)) => view.finish_load(base, purpose, window, cx),
                Ok(Err(error)) => view.fail_load(base_error_text(&error).into(), cx),
                Err(_) => view.fail_load("The request stopped before it finished".into(), cx),
            });
        });
        if self.base.is_none() {
            self.load = LoadState::Loading { _task: task };
        } else {
            self.reload = Some(task);
        }
        cx.notify();
    }

    fn fail_load(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.reload = None;
        if self.base.is_none() {
            self.load = LoadState::Failed(format!("Cannot edit: {message}").into());
        } else {
            self.footer_error = Some(message);
        }
        cx.notify();
    }

    fn finish_load(
        &mut self,
        base: ValuesBase,
        purpose: LoadPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load = LoadState::Ready;
        self.reload = None;
        self.footer_error = None;
        self.row_errors.clear();
        self.resource_version = Some(base.resource_version().to_owned().into());
        self.warnings = warnings_of(&base);
        match (purpose, self.base.is_some()) {
            (LoadPurpose::Rebase, true) => self.rebase_onto(&base, window, cx),
            _ => {
                self.rows = Self::build_rows(base.keys(), base.is_secret(), window, cx);
                self.banner = None;
                self.server_changed_keys.clear();
                self.focus_handle.focus(window, cx);
            }
        }
        self.base = Some(base);
        cx.notify();
    }

    /// One row per key of the object, sorted as the base sorts them.
    fn build_rows(
        keys: &[ValueKey],
        is_secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<ValueRow> {
        keys.iter()
            .map(|key| {
                let (field, subscription) = match &key.content {
                    KeyContent::Hidden => {
                        let (field, subscription) =
                            Self::new_field(&key.name, true, None, window, cx);
                        (
                            FieldKind::Secret {
                                field,
                                reveal: Reveal::Masked,
                            },
                            Some(subscription),
                        )
                    }
                    KeyContent::Text(text) => {
                        let (field, subscription) =
                            Self::new_field(&key.name, is_secret, Some(text), window, cx);
                        (
                            FieldKind::Text {
                                field,
                                original: text.clone(),
                            },
                            Some(subscription),
                        )
                    }
                    KeyContent::Binary => (
                        FieldKind::Binary {
                            size_bytes: key.size_bytes,
                        },
                        None,
                    ),
                    KeyContent::TooLarge => (
                        FieldKind::Large {
                            size_bytes: key.size_bytes,
                        },
                        None,
                    ),
                };
                ValueRow {
                    name: key.name.clone(),
                    origin: RowOrigin::Server { is_removed: false },
                    field,
                    has_text_change: false,
                    char_count: 0,
                    ends_with_line_break: false,
                    _subscription: subscription,
                }
            })
            .collect()
    }

    /// A value field: one `TextareaState` for its whole life. `text` is the ConfigMap text to start
    /// from; a Secret field starts empty.
    fn new_field(
        name: &str,
        is_secret: bool,
        text: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<TextareaState>, Subscription) {
        let field = cx.new(|cx| {
            let state = TextareaState::new(window, cx).auto_grow(1, if is_secret { 8 } else { 12 });
            if is_secret {
                state.placeholder("Type or paste a new value")
            } else {
                state
            }
        });
        if let Some(text) = text {
            // `set_value` emits no change event, so the row starts clean.
            field.update(cx, |state, cx| state.set_value(text.to_owned(), window, cx));
        }
        let name = name.to_owned();
        let subscription =
            cx.subscribe_in(&field, window, move |view, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.refresh_row(&name, cx);
                }
            });
        (field, subscription)
    }

    /// Reads the dirtiness of one row after a change event: the length of a Secret field's rope, or
    /// a ConfigMap text against the server's. A Secret field is never read as a string here.
    fn refresh_row(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter_mut().find(|row| row.name == name) else {
            return;
        };
        if let FieldKind::Secret { field, .. } = &row.field {
            let rope = field.read(cx).text();
            row.char_count = rope.chars().count();
            // The last non-empty chunk ends the text; nothing is copied out of the rope.
            row.ends_with_line_break = rope
                .chunks()
                .filter(|chunk| !chunk.is_empty())
                .last()
                .is_some_and(|chunk| chunk.ends_with('\n'));
        }
        row.has_text_change = match &row.field {
            FieldKind::Secret { .. } => row.char_count > 0,
            FieldKind::Text { field, original } => {
                field.read(cx).value().as_ref() != original.as_str()
            }
            FieldKind::Binary { .. } | FieldKind::Large { .. } => false,
        };
        self.row_errors.retain(|(key, _)| key != name);
        self.footer_error = None;
        cx.notify();
    }

    // ---- rebase (decision 12) ----

    /// The changes by key, without their values.
    fn pending(&self) -> Vec<(String, Pending)> {
        self.rows
            .iter()
            .filter_map(|row| {
                let pending = match row.state()? {
                    RowState::Added => Pending::Add,
                    RowState::Changed => Pending::Set,
                    RowState::Removed => Pending::Remove,
                };
                Some((row.name.clone(), pending))
            })
            .collect()
    }

    /// Moves the pending changes onto the newer object by key. A `Set` or `Remove` whose key is gone
    /// and an `Add` whose key now exists are dropped and listed; each kept change to a key the
    /// server has adds a confirm warning, because values cannot be compared.
    fn rebase_onto(&mut self, base: &ValuesBase, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.pending();
        let mut old = std::mem::take(&mut self.rows);
        let mut rows = Self::build_rows(base.keys(), base.is_secret(), window, cx);
        let mut dropped: Vec<SharedString> = Vec::new();
        let mut server_changed_keys = Vec::new();
        for (key, change) in pending {
            let Some(position) = old.iter().position(|row| row.name == key) else {
                continue;
            };
            let old_row = old.swap_remove(position);
            let fresh = rows.iter().position(|row| row.name == key);
            match (change, fresh) {
                (Pending::Add, Some(_)) => {
                    dropped.push(
                        format!("{key}: added on the server, your change was dropped").into(),
                    );
                }
                (Pending::Add, None) => rows.push(old_row),
                (Pending::Remove, Some(index)) => {
                    rows[index].origin = RowOrigin::Server { is_removed: true };
                    server_changed_keys.push(key);
                }
                (Pending::Set, Some(index)) => {
                    if take_field(old_row, &mut rows[index]) {
                        server_changed_keys.push(key);
                    } else {
                        dropped.push(
                            format!(
                                "{key}: can no longer be edited as text, your change was dropped"
                            )
                            .into(),
                        );
                    }
                }
                (Pending::Remove | Pending::Set, None) => {
                    dropped.push(
                        format!("{key}: removed on the server, your change was dropped").into(),
                    );
                }
            }
        }
        rows.sort_by(|left, right| left.name.cmp(&right.name));
        self.rows = rows;
        let names: Vec<String> = self.rows.iter().map(|row| row.name.clone()).collect();
        for name in names {
            self.refresh_row(&name, cx);
        }
        self.server_changed_keys = server_changed_keys;
        self.banner = Some(ValuesBanner::Rebased { dropped });
    }

    // ---- commands ----

    /// Marks an existing key for removal, or takes the mark back; an added key is just dropped.
    pub(crate) fn toggle_remove(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(index) = self.rows.iter().position(|row| row.name == name) else {
            return;
        };
        match &mut self.rows[index].origin {
            RowOrigin::Added => {
                self.rows.remove(index);
            }
            RowOrigin::Server { is_removed } => *is_removed = !*is_removed,
        }
        self.row_errors.retain(|(key, _)| key != name);
        self.footer_error = None;
        cx.notify();
    }

    /// The eye: unmasks one field for `REVEAL_DURATION`, or masks it again. Off in screenshot runs.
    pub(crate) fn toggle_reveal(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.access == ValueAccess::Blocked {
            return;
        }
        let Some(row) = self.rows.iter_mut().find(|row| row.name == name) else {
            return;
        };
        let FieldKind::Secret { field, reveal } = &mut row.field else {
            return;
        };
        match *reveal {
            Reveal::Masked => {
                *reveal = Reveal::Shown {
                    hides_at: Instant::now() + REVEAL_DURATION,
                };
                let field = field.clone();
                field.update(cx, |state, cx| state.focus(window, cx));
                self.start_ticker(window, cx);
            }
            Reveal::Shown { .. } => {
                *reveal = Reveal::Masked;
                self.release_focus_of_masked(window, cx);
            }
        }
        cx.notify();
    }

    /// Masks every Secret field: Apply and close do.
    fn mask_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for row in &mut self.rows {
            if let FieldKind::Secret { reveal, .. } = &mut row.field {
                *reveal = Reveal::Masked;
            }
        }
        self.release_focus_of_masked(window, cx);
    }

    /// A masked field is not drawn, so its textarea cannot keep the keyboard: focus would sit on an
    /// element outside the tree, and Ctrl S and the other keys of the view would stop working. The
    /// view's own handle takes it.
    fn release_focus_of_masked(&self, window: &mut Window, cx: &mut Context<Self>) {
        let has_focus = self.rows.iter().any(|row| match &row.field {
            FieldKind::Secret {
                field,
                reveal: Reveal::Masked,
            } => field.read(cx).focus_handle(cx).is_focused(window),
            _ => false,
        });
        if has_focus {
            self.focus_handle.focus(window, cx);
        }
    }

    /// The Paste button of a masked field: inserts the clipboard text into the hidden textarea
    /// without showing it. The user's click is the explicit act; the text is read once and handed
    /// over.
    pub(crate) fn paste_into(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.name == name) else {
            return;
        };
        let FieldKind::Secret { field, .. } = &row.field else {
            return;
        };
        let field = field.clone();
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // Dropped at the end of this call, which wipes it.
        let _text = insert_clipboard_text(&field, Zeroizing::new(text), window, cx);
    }

    /// The Trim button of the line-break warning: drops the trailing `\r` and `\n` of a Secret
    /// field. The text is copied once into a wiped buffer, like Apply does.
    pub(crate) fn trim_line_break(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.iter().find(|row| row.name == name) else {
            return;
        };
        let FieldKind::Secret { field, .. } = &row.field else {
            return;
        };
        let field = field.clone();
        let text = copy_text(&field, cx);
        let trimmed = Zeroizing::new(text.trim_end_matches(['\r', '\n']).to_owned());
        field.update(cx, |state, cx| {
            state.set_value(SharedString::from(trimmed.as_str()), window, cx)
        });
        // `set_value` emits no change event.
        self.refresh_row(name, cx);
    }

    /// Adds a row for the name in the Add field. The name is checked locally; the value is typed in
    /// the row (a Secret field starts masked like every other).
    pub(crate) fn add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.base.is_none() || self.is_running() {
            return;
        }
        let name = self.add_name.read(cx).value().trim().to_owned();
        if !is_valid_key_name(&name) {
            self.add_error = Some("Use letters, digits, '-', '_' and '.' (at most 253)".into());
            cx.notify();
            return;
        }
        if self.rows.iter().any(|row| row.name == name) {
            self.add_error = Some(format!("{name} is already a key").into());
            cx.notify();
            return;
        }
        let is_secret = self.is_secret();
        let (field, subscription) = Self::new_field(&name, is_secret, None, window, cx);
        let field = if is_secret {
            FieldKind::Secret {
                field,
                reveal: Reveal::Masked,
            }
        } else {
            FieldKind::Text {
                field,
                original: String::new(),
            }
        };
        self.rows.push(ValueRow {
            name,
            origin: RowOrigin::Added,
            field,
            has_text_change: true,
            char_count: 0,
            ends_with_line_break: false,
            _subscription: Some(subscription),
        });
        self.rows.sort_by(|left, right| left.name.cmp(&right.name));
        self.add_error = None;
        self.add_name
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    /// Reload and keep my changes: the pending changes move onto the newest object by key.
    pub(crate) fn keep_my_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.base.is_none() || self.is_running() {
            return;
        }
        self.reload_with(LoadPurpose::Rebase, window, cx);
    }

    /// Discard my changes: the newest object replaces the rows.
    pub(crate) fn discard_my_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        self.reload_with(LoadPurpose::Replace, window, cx);
    }

    pub(crate) fn dismiss_banner(&mut self, cx: &mut Context<Self>) {
        self.banner = None;
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        // The shell is told whether the view is dirty because it cannot read this view while the
        // view is being updated.
        let is_dirty = self.is_dirty();
        let _ = self
            .shell
            .update(cx, |shell, cx| shell.cancel_edit(is_dirty, cx));
    }

    /// Runs once a second while any field is shown.
    fn start_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_ticking {
            return;
        }
        self.is_ticking = true;
        self._ticker = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keeps_going =
                    this.update_in(cx, |view, window, cx| view.tick(Instant::now(), window, cx));
                if !matches!(keeps_going, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Masks what is due; false when nothing is left to count down.
    fn tick(&mut self, now: Instant, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if expire_reveals(&mut self.rows, now) {
            self.release_focus_of_masked(window, cx);
            cx.notify();
        }
        let keeps_going = self.rows.iter().any(|row| {
            matches!(
                row.field,
                FieldKind::Secret {
                    reveal: Reveal::Shown { .. },
                    ..
                }
            )
        });
        if !keeps_going {
            self.is_ticking = false;
        }
        keeps_going
    }

    // ---- apply ----

    /// Ctrl S and Apply…: copies each changed value once, checks the edit locally, and hands the
    /// request to the write flow, whose dialog runs the dry-run and the confirm tier.
    pub(crate) fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return;
        }
        // Apply hides every field again, whatever the answer: a local error leaves none shown.
        self.mask_all(window, cx);
        let is_deleted = matches!(self.banner, Some(ValuesBanner::Deleted));
        if self.base.is_none() || self.is_running() || !self.is_dirty() || is_deleted {
            return;
        }
        self.footer_error = None;
        self.row_errors.clear();
        let changes = match self.collect_changes(cx) {
            Ok(changes) => changes,
            Err(problem) => {
                self.row_errors.push((problem.key, problem.text));
                cx.notify();
                return;
            }
        };
        let Some(base) = &self.base else {
            return;
        };
        let edit = match base.edit(changes) {
            Ok(edit) => edit,
            Err(error) => {
                self.show_edit_error(error);
                cx.notify();
                return;
            }
        };
        let request = WriteRequest::new(
            self.object.clone(),
            WriteOperation::SetDataValues(Box::new(edit)),
        );
        let Some(request) = request else {
            self.footer_error = Some("These values cannot be edited here".into());
            cx.notify();
            return;
        };
        let mut warnings = self.warnings.clone();
        warnings.extend(self.server_changed_keys.iter().map(|key| {
            SharedString::from(format!(
                "{key}: the object changed on the server since you opened it"
            ))
        }));
        let intent = WriteIntent {
            cluster: self.target.cluster.clone(),
            cluster_name: self.cluster_name.clone(),
            action: ResourceAction::EditValues(self.kind),
            label: format!(
                "Edit values of {} {}",
                self.object.kind_name(),
                self.object.name()
            )
            .into(),
            button: "Apply changes".into(),
            request,
            risk: ActionRisk::Change,
            warnings,
            change_lines: Vec::new(),
            audit_fields: Vec::new(),
        };
        let open_id = self.open_id;
        let _ = self.shell.update(cx, |shell, cx| {
            shell.note_values_commit(open_id);
            shell.start_write(intent, window, cx)
        });
        cx.notify();
    }

    /// Ctrl S as the key layer delivers it. The action runs before the key-down listeners, so the
    /// decision waits until the end of the event: a repeat of a held key must never open the
    /// confirm dialog.
    pub(crate) fn apply_from_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.defer_in(window, |view, window, cx| {
            if !std::mem::take(&mut view.is_apply_key_held) {
                view.apply(window, cx);
            }
        });
    }

    /// Notes that the Ctrl S being delivered is a repeat of a held key.
    pub(crate) fn note_key_down(&mut self, event: &KeyDownEvent) {
        if event.is_held && event.keystroke.key == "s" && event.keystroke.modifiers.modified() {
            self.is_apply_key_held = true;
        }
    }

    /// The changes of the rows. Each changed value is copied once; a new Secret key with an empty
    /// field has nothing to keep, so it is a problem under its key.
    fn collect_changes(&self, cx: &App) -> Result<Vec<KeyChange>, RowProblem> {
        let mut changes = Vec::new();
        for row in &self.rows {
            let key = row.name.clone();
            if row.is_removed() {
                changes.push(KeyChange::Remove { key });
                continue;
            }
            let field = match &row.field {
                FieldKind::Secret { field, .. } | FieldKind::Text { field, .. } => field,
                FieldKind::Binary { .. } | FieldKind::Large { .. } => continue,
            };
            let is_secret = matches!(row.field, FieldKind::Secret { .. });
            match row.origin {
                RowOrigin::Server { .. } if row.has_text_change => changes.push(KeyChange::Set {
                    key,
                    value: NewValue::new(copy_text(field, cx)),
                }),
                RowOrigin::Server { .. } => {}
                RowOrigin::Added => {
                    let value = copy_text(field, cx);
                    if is_secret && value.is_empty() {
                        return Err(RowProblem {
                            key,
                            text: "Enter a value: an empty field keeps nothing for a new key"
                                .into(),
                        });
                    }
                    changes.push(KeyChange::Add {
                        key,
                        value: NewValue::new(value),
                    });
                }
            }
        }
        Ok(changes)
    }

    /// Shows a refused edit under its key, or in the footer when it names none.
    fn show_edit_error(&mut self, error: ValuesEditError) {
        let text: SharedString = error.to_string().into();
        match error {
            ValuesEditError::InvalidKey(key)
            | ValuesEditError::DuplicateKey(key)
            | ValuesEditError::UnknownKey(key)
            | ValuesEditError::BinaryValue(key)
            | ValuesEditError::TooLarge(key) => self.row_errors.push((key, text)),
            ValuesEditError::NoChange | ValuesEditError::ObjectTooLarge => {
                self.footer_error = Some(text);
            }
        }
    }

    /// A commit of this edit failed; the dialog is gone.
    pub(crate) fn commit_failed(&mut self, failure: EditFailure, cx: &mut Context<Self>) {
        match failure {
            EditFailure::Conflict => self.banner = Some(ValuesBanner::Conflict),
            EditFailure::Deleted => self.banner = Some(ValuesBanner::Deleted),
            EditFailure::OutcomeUnknown => self.banner = Some(ValuesBanner::OutcomeUnknown),
            EditFailure::Invalid { message, fields } => {
                for field in &fields {
                    if let Some(key) = key_of_field(field) {
                        self.row_errors
                            .push((key.to_owned(), "The server rejected this value".into()));
                    }
                }
                self.footer_error = Some(message);
            }
            EditFailure::Refused(text) | EditFailure::Other(text) => {
                self.footer_error = Some(text);
            }
        }
        cx.notify();
    }
}

/// Moves the typed field of `old` into `fresh` when both read the same way (text in text, a Secret
/// field in a Secret field); false when the key no longer takes typed text.
fn take_field(old: ValueRow, fresh: &mut ValueRow) -> bool {
    match (old.field, &mut fresh.field) {
        (
            FieldKind::Secret { field, .. },
            FieldKind::Secret {
                field: slot,
                reveal,
            },
        ) => {
            *slot = field;
            *reveal = Reveal::Masked;
        }
        (FieldKind::Text { field, .. }, FieldKind::Text { field: slot, .. }) => *slot = field,
        _ => return false,
    }
    fresh._subscription = old._subscription;
    true
}

/// The key a server field path names: `data[DB_PORT]` or `data.DB_PORT`.
fn key_of_field(field: &str) -> Option<&str> {
    let rest = field
        .strip_prefix("data")
        .or_else(|| field.strip_prefix("binaryData"))?;
    if let Some(inner) = rest.strip_prefix('[') {
        return inner.split(']').next().filter(|key| !key.is_empty());
    }
    rest.strip_prefix('.').filter(|key| !key.is_empty())
}

/// The non-blocking lines of decision 14, from the object's flags.
fn warnings_of(base: &ValuesBase) -> Vec<SharedString> {
    let mut lines: Vec<SharedString> = Vec::new();
    if base.notes().is_helm_managed {
        lines.push(HELM_MANAGED_WARNING.into());
    }
    if let Some(owner) = &base.notes().owner {
        lines.push(format!("Owned by {owner}: its controller may replace this change").into());
    }
    lines.push(RESTART_WARNING.into());
    lines
}

/// Why no editor opened, in the words of the view.
fn base_error_text(error: &ValuesBaseError) -> String {
    match error {
        ValuesBaseError::Cluster(error) => error_text(error),
        other => other.to_string(),
    }
}

impl Focusable for ValuesEditView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(feature = "screenshot")]
impl ValuesEditView {
    /// `--screen values-edit`: the editor drawn from fixed data. It has no base, so Apply is off,
    /// and it never touches a connection. Every value is fixture text and every field is masked.
    pub(crate) fn fixture(
        shell: WeakEntity<AppShell>,
        subject: ValuesSubject,
        access: ValueAccess,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        use cluster::DataField;
        let key = |name: &str, size_bytes: usize, content: KeyContent| ValueKey {
            name: name.to_owned(),
            field: DataField::Data,
            size_bytes,
            content,
        };
        let keys = [
            key("DB_HOST", 21, KeyContent::Hidden),
            key("DB_PASSWORD", 24, KeyContent::Hidden),
            key("DB_USER", 8, KeyContent::Hidden),
            key("ca.der", 1187, KeyContent::Binary),
        ];
        let mut view = Self::empty(shell, subject, access, window, cx);
        view.rows = Self::build_rows(&keys, true, window, cx);
        // A pending Remove of DB_USER and an Add of DB_PORT with a typed value (masked).
        if let Some(row) = view.rows.iter_mut().find(|row| row.name == "DB_USER") {
            row.origin = RowOrigin::Server { is_removed: true };
        }
        let (field, subscription) = Self::new_field("DB_PORT", true, None, window, cx);
        field.update(cx, |state, cx| state.insert("5432\r\n", window, cx));
        view.rows.push(ValueRow {
            name: "DB_PORT".to_owned(),
            origin: RowOrigin::Added,
            field: FieldKind::Secret {
                field,
                reveal: Reveal::Masked,
            },
            has_text_change: true,
            char_count: 6,
            ends_with_line_break: true,
            _subscription: Some(subscription),
        });
        view.rows.sort_by(|left, right| left.name.cmp(&right.name));
        view.is_fixture = true;
        view.resource_version = Some("88412093".into());
        view.warnings = vec![HELM_MANAGED_WARNING.into(), RESTART_WARNING.into()];
        view
    }
}

#[cfg(test)]
impl ValuesEditView {
    /// Whether the object was read and the rows are shown.
    pub(crate) fn is_loaded(&self) -> bool {
        self.base.is_some()
    }

    pub(crate) fn load_error(&self) -> Option<&SharedString> {
        match &self.load {
            LoadState::Failed(message) => Some(message),
            LoadState::Loading { .. } | LoadState::Ready => None,
        }
    }

    pub(crate) fn banner(&self) -> Option<&ValuesBanner> {
        self.banner.as_ref()
    }

    /// The textarea of key `key`, if it has one.
    pub(crate) fn field_of(&self, key: &str) -> Option<Entity<TextareaState>> {
        let row = self.rows.iter().find(|row| row.name == key)?;
        match &row.field {
            FieldKind::Secret { field, .. } | FieldKind::Text { field, .. } => Some(field.clone()),
            FieldKind::Binary { .. } | FieldKind::Large { .. } => None,
        }
    }
}

#[path = "values_edit_panels.rs"]
pub(crate) mod values_edit_panels;

#[cfg(test)]
#[path = "values_edit_tests.rs"]
mod values_edit_tests;
