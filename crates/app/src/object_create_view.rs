//! The New object view (spec 0042, wireframe W7 `New`, W10 for the editor shape): a YAML template
//! of a kind in the kit code editor, a server dry-run, and the confirm of the write flow. It lives
//! in the edit slot (`OpenEdit::Create`) and replaces the table like the Edit YAML view does.
//!
//! The text is user input and can hold credentials. This module never logs, never writes to disk,
//! and sends nothing itself: the dry-run and the commit go through `checked_write` and the dialog
//! of the write flow, on the cluster the view was opened on.

use std::rc::Rc;
use std::time::Duration;

use cluster::{
    DraftFix, DraftWarning, ObjectDraft, ObjectKind, WriteOperation, WriteOutcome, WriteRequest,
    format_yaml,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};

use crate::app_shell::AppShell;
use crate::app_shell::write_flow::{CheckedWriteError, WriteIntent, checked_write};
use crate::cluster_registry::ClusterRef;
use crate::keymap::{ApplyEdit, YAML_EDIT};
use crate::resource_actions::ResourceAction;
use crate::write_guard::ActionRisk;
use crate::yaml_edit::{EditFailure, edit_failure_of};

const SIDE_PANEL_WIDTH: f32 = 280.;
const COULD_NOT_CREATE: &str = "this object cannot be created here";
const OUTCOME_UNKNOWN: &str =
    "No answer in time; the object may have been created. Check the list before trying again.";

/// What the server's dry-run says about the text the editor held when it was asked.
pub(crate) enum CreateCheck {
    NotChecked,
    /// Dropping the task cancels the check; Ctrl S does nothing meanwhile.
    Running {
        _task: Task<()>,
    },
    Passed(Box<PassedCreate>),
    Failed(CreateFailure),
}

/// A dry-run that passed for one text.
pub(crate) struct PassedCreate {
    /// The editor text the check was for; a different text makes the check stale.
    pub(crate) for_text: SharedString,
    /// The request the second press hands to the confirm dialog. `None` in a fixture.
    pub(crate) request: Option<WriteRequest>,
    /// The draft warnings, then one line per field the server dropped.
    pub(crate) warnings: Vec<SharedString>,
    pub(crate) elapsed: Duration,
}

pub(crate) enum CreateFailure {
    /// The text is not a creatable object; nothing was sent. `fix` is the one-click repair of the
    /// text, when the error has one.
    Local {
        message: SharedString,
        fix: Option<DraftFix>,
    },
    /// A 422, a 409 name clash, or a missing namespace: the side panel lists the fields as the
    /// server spelled them.
    Invalid {
        message: SharedString,
        fields: Vec<SharedString>,
    },
    Server(SharedString),
}

impl CreateFailure {
    fn local(message: impl Into<SharedString>, fix: Option<DraftFix>) -> Self {
        Self::Local {
            message: message.into(),
            fix,
        }
    }
}

/// How a press of Ctrl S arrived: a repeat of a held key never opens the confirm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyPress {
    Fresh,
    Held,
}

/// The open New view, in `AppShell.edit`.
pub(crate) struct ObjectCreateView {
    shell: WeakEntity<AppShell>,
    /// The guard, the connection, the tier, and the audit line come from this cluster's own slot.
    cluster: ClusterRef,
    cluster_name: SharedString,
    kind: ObjectKind,
    /// The text the view opened with; the view is dirty when the editor differs from it.
    template: SharedString,
    editor: Entity<EditorState>,
    is_dirty: bool,
    check: CreateCheck,
    /// A held Ctrl S was seen in this key event (see `apply_from_key`).
    is_apply_key_held: bool,
    focus_handle: FocusHandle,
    /// `--screen new-config-map`: a picture drawn from fixed data that sends nothing.
    #[cfg(feature = "screenshot")]
    is_fixture: bool,
    _subscription: Subscription,
}

impl ObjectCreateView {
    pub(crate) fn new(
        shell: WeakEntity<AppShell>,
        cluster: ClusterRef,
        cluster_name: SharedString,
        kind: ObjectKind,
        template: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let template = SharedString::from(template);
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("yaml")
                // Wrapping measures a whole line to break it; a single line of a few MiB (a paste)
                // hangs the window under load. The 2 MiB cap is checked before any request.
                .soft_wrap(false)
                .line_number(true);
            state.set_value(template.to_string(), window, cx);
            state
        });
        let subscription =
            cx.subscribe_in(&editor, window, |view, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.refresh_dirty(cx);
                    cx.notify();
                }
            });
        Self {
            shell,
            cluster,
            cluster_name,
            kind,
            template,
            editor,
            is_dirty: false,
            check: CreateCheck::NotChecked,
            is_apply_key_held: false,
            focus_handle: cx.focus_handle(),
            #[cfg(feature = "screenshot")]
            is_fixture: false,
            _subscription: subscription,
        }
    }

    /// Puts the caret in the editor so the user types at once. The caller does it, not `new`: a
    /// fixture never focuses, and a view that holds the focus at exit trips the leak check of the
    /// screenshot build.
    pub(crate) fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
    }

    pub(crate) fn cluster(&self) -> &ClusterRef {
        &self.cluster
    }

    pub(crate) fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// `new ConfigMap`: what the leaving dialog and the discard prompt call the edit.
    pub(crate) fn subject_text(&self) -> String {
        format!("new {}", self.kind.name())
    }

    fn text(&self, cx: &gpui_kit::App) -> SharedString {
        self.editor.read(cx).value()
    }

    fn refresh_dirty(&mut self, cx: &gpui_kit::App) {
        self.is_dirty = self.text(cx) != self.template;
    }

    fn is_running(&self) -> bool {
        matches!(self.check, CreateCheck::Running { .. })
    }

    // ---- commands ----

    /// Format: the 0007 serializer on the text, as one undoable replacement.
    pub(crate) fn format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        match format_yaml(&text) {
            Ok(formatted) => {
                self.editor
                    .update(cx, |editor, cx| editor.replace_all(formatted, window, cx));
                self.refresh_dirty(cx);
            }
            Err(error) => {
                self.check = CreateCheck::Failed(CreateFailure::local(error.to_string(), None));
            }
        }
        cx.notify();
    }

    /// The one-click repair of a failed text (`DraftFix`): the text is replaced as one undoable
    /// edit, and the check starts over.
    pub(crate) fn apply_fix(&mut self, fix: DraftFix, window: &mut Window, cx: &mut Context<Self>) {
        match fix.apply(&self.text(cx)) {
            Ok(fixed) => {
                self.editor
                    .update(cx, |editor, cx| editor.replace_all(fixed, window, cx));
                self.refresh_dirty(cx);
                self.check = CreateCheck::NotChecked;
            }
            Err(error) => {
                self.check = CreateCheck::Failed(CreateFailure::local(error.to_string(), None));
            }
        }
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        // The shell is told whether the text is dirty because it cannot read this view while the
        // view is being updated.
        let is_dirty = self.is_dirty;
        let _ = self
            .shell
            .update(cx, |shell, cx| shell.cancel_edit(is_dirty, cx));
    }

    /// Ctrl S and `Create…`: the first press checks the text with the server, the next one for the
    /// same text opens the confirm dialog. Nothing while a check runs.
    pub(crate) fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_press(KeyPress::Fresh, window, cx);
    }

    /// Ctrl S as the key layer delivers it. The action runs before the key-down listeners, so the
    /// decision waits until the end of the event: a repeat of a held key is seen by then and must
    /// never open the confirm dialog.
    fn apply_from_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.defer_in(window, |view, window, cx| {
            let press = if std::mem::take(&mut view.is_apply_key_held) {
                KeyPress::Held
            } else {
                KeyPress::Fresh
            };
            view.apply_press(press, window, cx);
        });
    }

    /// Notes that the Ctrl S being delivered is a repeat of a held key.
    fn note_key_down(&mut self, event: &KeyDownEvent) {
        if event.is_held && event.keystroke.key == "s" && event.keystroke.modifiers.modified() {
            self.is_apply_key_held = true;
        }
    }

    fn apply_press(&mut self, press: KeyPress, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return;
        }
        if self.is_running() {
            return;
        }
        let text = self.text(cx);
        if let CreateCheck::Passed(passed) = &self.check
            && passed.for_text == text
        {
            if press == KeyPress::Fresh {
                self.confirm(window, cx);
            }
            return;
        }
        self.run_check(text, cx);
    }

    /// The second press: the confirm dialog of the write flow, with its own dry-run.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let CreateCheck::Passed(passed) = &self.check else {
            return;
        };
        let Some(request) = passed.request.clone() else {
            return;
        };
        let risk = confirm_risk(&request);
        let warnings = passed.warnings.clone();
        let mut intent = self.intent(request, warnings);
        intent.risk = risk;
        let _ = self
            .shell
            .update(cx, |shell, cx| shell.start_write(intent, window, cx));
    }

    fn intent(&self, request: WriteRequest, warnings: Vec<SharedString>) -> WriteIntent {
        create_intent(
            &self.cluster,
            &self.cluster_name,
            self.kind,
            request,
            warnings,
        )
    }

    /// Checks `text` locally, then asks the server to dry-run the create.
    fn run_check(&mut self, text: SharedString, cx: &mut Context<Self>) {
        let draft = match ObjectDraft::new(self.kind, &text) {
            Ok(draft) => draft,
            Err(error) => {
                let fix = error.fix();
                self.check = CreateCheck::Failed(CreateFailure::local(error.to_string(), fix));
                cx.notify();
                return;
            }
        };
        let warnings = draft.warnings().to_vec();
        let Some(request) = WriteRequest::new(
            draft.target().clone(),
            WriteOperation::CreateObject(Box::new(draft)),
        ) else {
            self.check = CreateCheck::Failed(CreateFailure::local(COULD_NOT_CREATE, None));
            cx.notify();
            return;
        };
        let intent = Rc::new(self.intent(request.clone(), Vec::new()));
        let step = self
            .shell
            .update(cx, |shell, cx| shell.begin_preview(Rc::clone(&intent), cx));
        let step = match step {
            Ok(Ok(step)) => step,
            Ok(Err(reason)) => {
                self.check = CreateCheck::Failed(CreateFailure::Server(reason));
                cx.notify();
                return;
            }
            Err(_) => return,
        };
        let shell = self.shell.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = checked_write(&shell, step, cx).await;
            let _ = this.update(cx, |view, cx| {
                view.finish_check(text, request, warnings, result, cx);
            });
        });
        self.check = CreateCheck::Running { _task: task };
        cx.notify();
    }

    fn finish_check(
        &mut self,
        text: SharedString,
        request: WriteRequest,
        warnings: Vec<DraftWarning>,
        result: Result<WriteOutcome, CheckedWriteError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(outcome) => {
                let warnings = warning_lines(&warnings, &outcome.dropped_fields);
                self.check = CreateCheck::Passed(Box::new(PassedCreate {
                    for_text: text,
                    request: Some(request),
                    warnings,
                    elapsed: outcome.elapsed,
                }));
            }
            Err(error) => self.check = CreateCheck::Failed(failure_of(edit_failure_of(&error))),
        }
        cx.notify();
    }

    /// A commit of this create failed; the dialog is gone. The text stays, and the next press runs a
    /// new dry-run.
    pub(crate) fn commit_failed(&mut self, failure: EditFailure, cx: &mut Context<Self>) {
        self.check = CreateCheck::Failed(failure_of(failure));
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn check(&self) -> &CreateCheck {
        &self.check
    }

    #[cfg(test)]
    pub(crate) fn set_text_for_test(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |editor, cx| {
            editor.set_value(text.to_owned(), window, cx)
        });
        self.refresh_dirty(cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn text_for_test(&self, cx: &gpui_kit::App) -> SharedString {
        self.text(cx)
    }
}

impl Focusable for ObjectCreateView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The intent of creating the object, for the dry-run and the confirm dialog. The audit line
/// records `Create`; the confirm button says `Create`.
pub(crate) fn create_intent(
    cluster: &ClusterRef,
    cluster_name: &SharedString,
    kind: ObjectKind,
    request: WriteRequest,
    warnings: Vec<SharedString>,
) -> WriteIntent {
    let target = request.target();
    let label = match target.namespace() {
        Some(namespace) => format!(
            "Create {kind} {namespace}/{}",
            target.name(),
            kind = kind.name()
        ),
        None => format!("Create {} {}", kind.name(), target.name()),
    };
    WriteIntent {
        cluster: cluster.clone(),
        cluster_name: cluster_name.clone(),
        action: ResourceAction::CreateObject(kind),
        label: label.into(),
        button: "Create".into(),
        request,
        risk: ActionRisk::Change,
        warnings,
    }
}

/// A draft with a risky grant types its name on every environment (decision 15); any other draft
/// keeps `Change` and the cluster's own tier.
pub(crate) fn confirm_risk(request: &WriteRequest) -> ActionRisk {
    let is_risky = matches!(
        request.operation(),
        WriteOperation::CreateObject(draft)
            if draft.warnings().iter().any(DraftWarning::needs_typed_name)
    );
    if is_risky {
        ActionRisk::Privileged
    } else {
        ActionRisk::Change
    }
}

/// The text of one draft warning in the side panel and the confirm dialog (decision 7).
fn warning_text(warning: &DraftWarning) -> SharedString {
    match warning {
        DraftWarning::PowerfulRole { role } => {
            format!("Grants the {role} ClusterRole, which gives broad rights").into()
        }
        DraftWarning::BroadSubject { kind, name } => {
            format!("{kind} {name} is a built-in API server identity; it may cover every user")
                .into()
        }
        DraftWarning::PrivilegedPodSecurity => {
            "Pods in this namespace may run privileged (pod-security enforce: privileged)".into()
        }
        DraftWarning::SystemNamespaceAccount { name } => {
            format!("ServiceAccount kube-system/{name} is a control plane account").into()
        }
        DraftWarning::SystemRole { role } => {
            format!("{role} is a built-in ClusterRole of the API server").into()
        }
    }
}

/// How many warning lines the side panel and the confirm show; the rest is one `and {n} more`
/// line, so a draft of hundreds of subjects or dropped fields cannot flood either.
const MAX_WARNING_LINES: usize = 10;

/// The draft warnings, then one line per field the server dropped, cut at ten lines.
pub(crate) fn warning_lines(warnings: &[DraftWarning], dropped: &[String]) -> Vec<SharedString> {
    let mut lines: Vec<SharedString> = warnings
        .iter()
        .map(warning_text)
        .chain(dropped.iter().map(|path| dropped_text(path)))
        .collect();
    let more = lines.len().saturating_sub(MAX_WARNING_LINES);
    if more > 0 {
        lines.truncate(MAX_WARNING_LINES);
        lines.push(format!("\u{2026} and {more} more").into());
    }
    lines
}

/// A field of the draft that the server dropped (decision 14): a misspelled field name.
fn dropped_text(path: &str) -> SharedString {
    format!("{path} is not a known field; the server dropped it").into()
}

/// How a failed write is told in the view, whichever request failed.
fn failure_of(failure: EditFailure) -> CreateFailure {
    match failure {
        EditFailure::Invalid { message, fields } => CreateFailure::Invalid { message, fields },
        EditFailure::OutcomeUnknown => CreateFailure::Server(OUTCOME_UNKNOWN.into()),
        EditFailure::Conflict => CreateFailure::Server(
            "The server reported a conflict; check the list, then try again".into(),
        ),
        EditFailure::Deleted => CreateFailure::Server("The object no longer exists".into()),
        EditFailure::Refused(text) | EditFailure::Other(text) => CreateFailure::Server(text),
    }
}

/// The footer line under the editor, from what the check and the text say now.
pub(crate) fn footer_text(check: &CreateCheck, current: &str) -> String {
    match check {
        CreateCheck::NotChecked => "Not checked yet".to_owned(),
        CreateCheck::Running { .. } => "Server dry-run…".to_owned(),
        CreateCheck::Passed(passed) if passed.for_text != current => {
            "Changed since the last check".to_owned()
        }
        CreateCheck::Passed(passed) => {
            format!("Dry-run OK · {} ms", passed.elapsed.as_millis())
        }
        CreateCheck::Failed(
            CreateFailure::Local { message: text, .. }
            | CreateFailure::Invalid { message: text, .. }
            | CreateFailure::Server(text),
        ) => text.to_string(),
    }
}

impl ObjectCreateView {
    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .flex_shrink_0()
            .gap_3()
            .px_4()
            .py_2()
            .items_baseline()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(self.kind.name()),
            )
            .child(
                div()
                    .text_lg()
                    .font_semibold()
                    .child(format!("New {}", self.kind.name())),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(self.cluster_name.clone()),
            )
            .child(div().ml_auto())
            .child(
                Button::new("create-format")
                    .label("Format")
                    .small()
                    .outline()
                    .tooltip("Sort the keys and tidy the text")
                    .on_click(cx.listener(|view, _, window, cx| view.format(window, cx))),
            )
            .into_any_element()
    }

    fn render_side(&self, text: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mono = theme.mono_font_family.clone();
        let heading = |text: &'static str| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let (status, tone) = dry_run_line(&self.check, text, cx);
        let mut side = v_flex()
            .id("create-side")
            .w(px(SIDE_PANEL_WIDTH))
            .flex_shrink_0()
            .h_full()
            .gap_2()
            .p_3()
            .border_l_1()
            .border_color(theme.border)
            .overflow_y_scroll()
            .child(heading("Checks"))
            .child(div().text_xs().text_color(tone).child(status));
        if let CreateCheck::Passed(passed) = &self.check {
            for warning in &passed.warnings {
                side = side.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(warning.clone()),
                );
            }
        }
        // The message is the status line above; the button repairs the text it describes.
        if let CreateCheck::Failed(CreateFailure::Local { fix: Some(fix), .. }) = &self.check {
            let fix = *fix;
            side = side.child(
                Button::new("create-fix")
                    .label(fix_label(fix))
                    .small()
                    .outline()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.apply_fix(fix, window, cx);
                    })),
            );
        }
        if let CreateCheck::Failed(CreateFailure::Invalid { message, fields }) = &self.check {
            side = side
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.danger)
                        .child("The object is invalid"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(message.clone()),
                );
            for field in fields {
                side = side.child(
                    div()
                        .text_xs()
                        .font_family(mono.clone())
                        .text_color(theme.danger)
                        .child(field.clone()),
                );
            }
        }
        side.into_any_element()
    }

    fn render_footer(&self, text: &str, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (_, tone) = dry_run_line(&self.check, text, cx);
        let create = Button::new("create-apply")
            .label("Create…")
            .small()
            .primary()
            .disabled(self.is_running())
            .tooltip("Check the object with the server, then create it")
            .on_click(cx.listener(|view, _, window, cx| view.apply(window, cx)));
        let key = Kbd::binding_for_action(&ApplyEdit, Some(YAML_EDIT), window);
        h_flex()
            .flex_shrink_0()
            .gap_3()
            .px_4()
            .py_2()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("create-status")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(tone)
                    .child(footer_text(&self.check, text)),
            )
            .child(
                Button::new("create-cancel")
                    .label("Cancel")
                    .small()
                    .outline()
                    .on_click(cx.listener(|view, _, _, cx| view.cancel(cx))),
            )
            .child(h_flex().gap_1().items_center().child(create).children(key))
            .into_any_element()
    }
}

impl Render for ObjectCreateView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.text(cx);
        v_flex()
            .key_context(YAML_EDIT)
            .track_focus(&self.focus_handle)
            .capture_key_down(
                cx.listener(|view, event: &KeyDownEvent, _, _| view.note_key_down(event)),
            )
            .on_action(cx.listener(|view, _: &ApplyEdit, window, cx| {
                view.apply_from_key(window, cx);
                // A handled action ends the key event before the key-down listeners, which tell a
                // held key from a fresh one: let the event go on.
                cx.propagate();
            }))
            .size_full()
            .min_h_0()
            .child(self.render_header(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div().flex_1().min_w_0().h_full().child(
                            Editor::new(&self.editor)
                                .bordered(false)
                                .text_xs()
                                .size_full(),
                        ),
                    )
                    .child(self.render_side(&text, cx)),
            )
            .child(self.render_footer(&text, window, cx))
    }
}

/// The dry-run line of the Checks section and its tone.
fn dry_run_line(
    check: &CreateCheck,
    current: &str,
    cx: &gpui_kit::App,
) -> (String, gpui_kit::Hsla) {
    let theme = cx.theme();
    match check {
        CreateCheck::Passed(passed) if passed.for_text == current => {
            ("Server dry-run passed".to_owned(), theme.success)
        }
        CreateCheck::Passed(_) => (
            "Changed since the last check".to_owned(),
            theme.muted_foreground,
        ),
        CreateCheck::Failed(CreateFailure::Invalid { .. }) => {
            ("Server dry-run refused the object".to_owned(), theme.danger)
        }
        CreateCheck::Failed(_) => (footer_text(check, current), theme.danger),
        CreateCheck::Running { .. } | CreateCheck::NotChecked => {
            (footer_text(check, current), theme.muted_foreground)
        }
    }
}

/// The button that repairs the text for a failed check.
fn fix_label(fix: DraftFix) -> &'static str {
    match fix {
        DraftFix::RemoveServerFields => "Remove server fields",
        DraftFix::KeepFirstDocument => "Keep the first document",
    }
}
#[cfg(feature = "screenshot")]
impl ObjectCreateView {
    /// `--screen new-config-map`: the ConfigMap template of `payments` with a passed dry-run, drawn
    /// from fixed data. It has no request, so Create is off, and it never touches a connection.
    pub(crate) fn fixture(
        shell: WeakEntity<AppShell>,
        cluster: ClusterRef,
        cluster_name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kind = ObjectKind::ConfigMap;
        let template = crate::object_templates::template_text(kind, "payments").unwrap_or_default();
        let mut view = Self::new(
            shell,
            cluster,
            cluster_name,
            kind,
            template.clone(),
            window,
            cx,
        );
        view.is_fixture = true;
        view.check = CreateCheck::Passed(Box::new(PassedCreate {
            for_text: template.into(),
            request: None,
            warnings: Vec::new(),
            elapsed: Duration::from_millis(212),
        }));
        view
    }
}

#[cfg(test)]
#[path = "object_create_view_tests.rs"]
mod object_create_view_tests;
