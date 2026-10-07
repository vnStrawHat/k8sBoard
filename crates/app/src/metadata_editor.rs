//! The labels and annotations editor of a pod or a workload (spec 0032b). The editor reads the
//! object fresh from its own cluster, collects rows, and hands the result to the guarded flow
//! (`start_write`), which shows the confirm dialog with the tier, the server dry-run, and the audit
//! line. Nothing here sends a change.
//!
//! A child of `app_shell`, like `node_editor`: every step names the cluster of the object and takes
//! its guard and connection from that cluster's own slot, never from the primary.

use cluster::{ClusterConnection, ClusterError, ObjectKind, ObjectMetadata, ObjectRef};
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window, div, px,
};

use super::AppShell;
use super::node_editor::{
    DIALOG_WIDTH, ROWS_MAX_HEIGHT, ask_before_closing, input_cell, text_input,
};
use super::write_flow::{WriteIntent, notify};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::fresh_enter::{confirms, is_enter};
use crate::keymap::FORWARD_FORM;
use crate::metadata_edits::{
    MetadataList, MetadataRow, metadata_intent, metadata_row_problem, metadata_rows,
};
use crate::node_edits::{KEY_HINT, NodeScope, RowField, is_empty_key_problem};
use crate::resource_actions::{
    ActionAvailability, ResourceAction, RowAction, action_availability, action_label,
    subject_action, unavailable_text,
};
use crate::table_selection::ClusterObject;
use crate::yaml_view::object_ref;

struct RowInputs {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

enum EditorState {
    Loading,
    Failed(SharedString),
    Ready {
        edit: ObjectMetadata,
        labels: Vec<RowInputs>,
        annotations: Vec<RowInputs>,
    },
}

fn row_inputs(row: &MetadataRow, window: &mut Window, cx: &mut App) -> RowInputs {
    RowInputs {
        key: text_input(&row.key, "key", window, cx),
        value: text_input(&row.value, "value", window, cx),
    }
}

fn read_rows(inputs: &[RowInputs], cx: &App) -> Vec<MetadataRow> {
    inputs
        .iter()
        .map(|row| MetadataRow {
            key: row.key.read(cx).value().to_string(),
            value: row.value.read(cx).value().to_string(),
        })
        .collect()
}

/// The body of the editor dialog.
pub(crate) struct MetadataEditor {
    shell: WeakEntity<AppShell>,
    cluster: ClusterRef,
    cluster_name: SharedString,
    kind: ObjectKind,
    object: ObjectRef,
    state: EditorState,
    /// Review… was pressed on a row with no key: the problem line and the row mark show now, not
    /// while the user is still typing.
    has_tried_review: bool,
    /// Takes the focus when a row goes, so Enter after × still reviews instead of reaching the
    /// dialog, which would close it with the edits.
    focus_handle: FocusHandle,
    _load: Option<Task<()>>,
}

/// What an editor is opened on: the object and the cluster it belongs to.
struct EditorTarget {
    cluster: ClusterRef,
    cluster_name: SharedString,
    kind: ObjectKind,
    object: ObjectRef,
}

impl MetadataEditor {
    fn new(
        shell: WeakEntity<AppShell>,
        target: EditorTarget,
        connection: ClusterConnection,
        runtime: ClusterRuntime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let EditorTarget {
            cluster,
            cluster_name,
            kind,
            object,
        } = target;
        let read_of = object.clone();
        let load = cx.spawn_in(window, async move |this, cx| {
            let read = runtime
                .spawn(async move { connection.object_metadata(&read_of).await })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| editor.loaded(read, window, cx));
        });
        Self {
            shell,
            cluster,
            cluster_name,
            kind,
            object,
            state: EditorState::Loading,
            has_tried_review: false,
            focus_handle: cx.focus_handle(),
            _load: Some(load),
        }
    }

    fn loaded(
        &mut self,
        read: Result<Result<ObjectMetadata, ClusterError>, tokio::task::JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state = match read {
            Ok(Ok(edit)) => {
                let inputs = |rows: Vec<MetadataRow>, window: &mut Window, cx: &mut App| {
                    rows.iter()
                        .map(|row| row_inputs(row, window, cx))
                        .collect::<Vec<_>>()
                };
                EditorState::Ready {
                    labels: inputs(metadata_rows(&edit.labels), window, cx),
                    annotations: inputs(metadata_rows(&edit.annotations), window, cx),
                    edit,
                }
            }
            Ok(Err(error)) => EditorState::Failed(
                format!(
                    "Could not read {} {}: {error}",
                    self.object.kind_name().to_ascii_lowercase(),
                    self.object.name()
                )
                .into(),
            ),
            Err(_) => EditorState::Failed("The request task stopped".into()),
        };
        cx.notify();
    }

    /// The rows of both lists as typed now.
    fn rows(&self, cx: &App) -> Option<(Vec<MetadataRow>, Vec<MetadataRow>)> {
        let EditorState::Ready {
            labels,
            annotations,
            ..
        } = &self.state
        else {
            return None;
        };
        Some((read_rows(labels, cx), read_rows(annotations, cx)))
    }

    /// The change the rows describe now, or why there is none. `No changes` keeps Review off.
    fn intent(&self, cx: &App) -> Result<WriteIntent, SharedString> {
        let (EditorState::Ready { edit, .. }, Some((labels, annotations))) =
            (&self.state, self.rows(cx))
        else {
            return Err("Loading…".into());
        };
        let scope = NodeScope {
            cluster: &self.cluster,
            cluster_name: &self.cluster_name,
        };
        metadata_intent(&scope, self.kind, &self.object, edit, &labels, &annotations)
    }

    /// The list and row that carry the validation line, to mark them.
    fn row_problem(&self, cx: &App) -> Option<(MetadataList, usize, RowField)> {
        let (labels, annotations) = self.rows(cx)?;
        [
            (MetadataList::Labels, labels),
            (MetadataList::Annotations, annotations),
        ]
        .into_iter()
        .find_map(|(list, rows)| {
            let problem = metadata_row_problem(list, &rows)?;
            Some((list, problem.index, problem.field))
        })
    }

    /// The first row of `list` with no key, to mark it once Review… was pressed.
    fn first_empty_key_row(&self, list: MetadataList, cx: &App) -> Option<usize> {
        let (labels, annotations) = self.rows(cx)?;
        let rows = match list {
            MetadataList::Labels => labels,
            MetadataList::Annotations => annotations,
        };
        rows.iter().position(|row| row.key.trim().is_empty())
    }

    fn inputs_mut(&mut self, list: MetadataList) -> Option<&mut Vec<RowInputs>> {
        let EditorState::Ready {
            labels,
            annotations,
            ..
        } = &mut self.state
        else {
            return None;
        };
        Some(match list {
            MetadataList::Labels => labels,
            MetadataList::Annotations => annotations,
        })
    }

    /// Puts a new empty row at the top of `list`, where it is in view whatever the list scrolled
    /// to, and the cursor in its key field so typing goes into it.
    fn add_row(&mut self, list: MetadataList, window: &mut Window, cx: &mut Context<Self>) {
        let added = row_inputs(
            &MetadataRow {
                key: String::new(),
                value: String::new(),
            },
            window,
            cx,
        );
        let key = added.key.clone();
        let Some(inputs) = self.inputs_mut(list) else {
            return;
        };
        inputs.insert(0, added);
        key.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Drops a row, and takes the focus: the × that was clicked goes with its row, and an Enter
    /// with the focus nowhere would reach the dialog and close it with the edits.
    fn remove_row(
        &mut self,
        list: MetadataList,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(inputs) = self.inputs_mut(list) else {
            return;
        };
        if index < inputs.len() {
            inputs.remove(index);
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    /// Review…: closes the editor and starts the guarded flow, whose dialog follows. Nothing is
    /// sent from here.
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let intent = match self.intent(cx) {
            Ok(intent) => intent,
            Err(reason) => {
                // A row with no key is named only now that the user asked to review.
                if is_empty_key_problem(&reason) {
                    self.has_tried_review = true;
                    cx.notify();
                }
                return;
            }
        };
        let shell = self.shell.clone();
        window.close_dialog(cx);
        // After the close: the flow opens the confirm dialog, which the close must not pop.
        window.defer(cx, move |window, cx| {
            let _ = shell.update(cx, |shell, cx| shell.start_write(intent, window, cx));
        });
    }

    /// Whether the rows differ from the object as it was read: closing now would lose them.
    pub(super) fn has_unsaved_rows(&self, cx: &App) -> bool {
        let EditorState::Ready { edit, .. } = &self.state else {
            return false;
        };
        self.rows(cx).is_some_and(|(labels, annotations)| {
            labels != metadata_rows(&edit.labels) || annotations != metadata_rows(&edit.annotations)
        })
    }

    /// Cancel: closes at once when no row changed, else asks first.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !ask_before_closing(self.has_unsaved_rows(cx), window, cx) {
            window.close_dialog(cx);
        }
    }

    /// Enter in a row's text field, or with the focus on the editor body after a removal, presses
    /// Review…, a fresh press only. A focused button keeps its own Enter.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !is_enter(event)
            || !(self.is_input_focused(window, cx) || self.focus_handle.is_focused(window))
        {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if confirms(event) {
            self.review(window, cx);
        }
    }

    fn is_input_focused(&self, window: &Window, cx: &App) -> bool {
        let EditorState::Ready {
            labels,
            annotations,
            ..
        } = &self.state
        else {
            return false;
        };
        let is_focused =
            |input: &Entity<InputState>| input.read(cx).focus_handle(cx).is_focused(window);
        labels
            .iter()
            .chain(annotations)
            .any(|row| is_focused(&row.key) || is_focused(&row.value))
    }

    /// One list: its title with the Add button, then a row per key.
    fn render_list(&self, list: MetadataList, cx: &mut Context<Self>) -> AnyElement {
        let EditorState::Ready {
            edit,
            labels,
            annotations,
        } = &self.state
        else {
            return div().into_any_element();
        };
        let (title, inputs, hidden) = match list {
            MetadataList::Labels => ("Labels", labels, 0),
            MetadataList::Annotations => {
                ("Annotations", annotations, edit.hidden_annotations.len())
            }
        };
        let (muted, danger) = (cx.theme().muted_foreground, cx.theme().danger);
        let problem = self.row_problem(cx);
        let empty_key = self
            .has_tried_review
            .then(|| self.first_empty_key_row(list, cx))
            .flatten();
        let is_bad = |index: usize, field: RowField| {
            (field == RowField::Key && empty_key == Some(index))
                || problem.is_some_and(|(problem_list, problem_index, problem_field)| {
                    problem_list == list && problem_index == index && problem_field == field
                })
        };
        let remove_id = match list {
            MetadataList::Labels => "metadata-remove-label",
            MetadataList::Annotations => "metadata-remove-annotation",
        };
        let rows = inputs.iter().enumerate().map(|(index, row)| {
            h_flex()
                .gap_2()
                .items_center()
                .child(input_cell(
                    Input::new(&row.key).small(),
                    is_bad(index, RowField::Key),
                    danger,
                ))
                .child(input_cell(
                    Input::new(&row.value).small(),
                    is_bad(index, RowField::Value),
                    danger,
                ))
                .child(
                    Button::new((remove_id, index))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::X))
                        .tooltip("Remove")
                        .on_click(cx.listener(move |editor, _, window, cx| {
                            editor.remove_row(list, index, window, cx)
                        })),
                )
        });
        let add_id = match list {
            MetadataList::Labels => "metadata-add-label",
            MetadataList::Annotations => "metadata-add-annotation",
        };
        let hidden_note = (hidden > 0).then(|| {
            let noun = if hidden == 1 {
                "annotation holds"
            } else {
                "annotations hold"
            };
            div().text_xs().text_color(muted).child(format!(
                "{hidden} {noun} the applied manifest and stay hidden"
            ))
        });
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        Button::new(add_id)
                            .icon(Icon::new(IconName::Plus))
                            .label("Add")
                            .small()
                            .outline()
                            .on_click(cx.listener(move |editor, _, window, cx| {
                                editor.add_row(list, window, cx);
                            })),
                    ),
            )
            .children(inputs.is_empty().then(|| {
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(format!("No {}", title.to_ascii_lowercase()))
            }))
            .children(rows)
            .children(hidden_note)
            .into_any_element()
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let intent = self.intent(cx);
        let is_ready = matches!(self.state, EditorState::Ready { .. });
        // A row with no key keeps Review on: pressing it is what names the problem.
        let is_off = intent
            .as_ref()
            .err()
            .is_some_and(|reason| !is_empty_key_problem(reason));
        let review = Button::new("metadata-edit-review")
            .label("Review…")
            .small()
            .primary()
            .disabled(is_off)
            .on_click(cx.listener(|editor, _, window, cx| editor.review(window, cx)));
        let review = match &intent {
            Err(reason) if is_ready && is_off => review.tooltip(reason.clone()),
            _ => review,
        };
        h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(
                Button::new("metadata-edit-cancel")
                    .label("Cancel")
                    .small()
                    .outline()
                    .on_click(cx.listener(|editor, _, window, cx| editor.cancel(window, cx))),
            )
            .child(review)
            .into_any_element()
    }
}

impl Render for MetadataEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let intent = self.intent(cx);
        // A row with no key is named only after Review… asked, and no longer once every row has one.
        let is_empty_key = intent
            .as_ref()
            .err()
            .is_some_and(|reason| is_empty_key_problem(reason));
        self.has_tried_review &= is_empty_key;
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let body: AnyElement = match &self.state {
            EditorState::Loading => h_flex()
                .gap_2()
                .items_center()
                .child(Spinner::new())
                .child(div().text_sm().text_color(muted).child(format!(
                    "Loading {}…",
                    self.object.kind_name().to_ascii_lowercase()
                )))
                .into_any_element(),
            EditorState::Failed(text) => div()
                .text_sm()
                .text_color(danger)
                .child(text.clone())
                .into_any_element(),
            EditorState::Ready { .. } => {
                // `No changes` is the resting state, not a problem; a row with no key waits for Review….
                let problem = intent.err().filter(|reason| {
                    reason.as_ref() != "No changes"
                        && (!is_empty_key_problem(reason) || self.has_tried_review)
                });
                let hint = self
                    .row_problem(cx)
                    .map(|_| div().text_xs().text_color(muted).child(KEY_HINT));
                v_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .id("metadata-edit-rows")
                            .gap_3()
                            .max_h(px(ROWS_MAX_HEIGHT))
                            .overflow_y_scroll()
                            .child(self.render_list(MetadataList::Labels, cx))
                            .child(self.render_list(MetadataList::Annotations, cx)),
                    )
                    .children(problem.map(|text| div().text_sm().text_color(danger).child(text)))
                    .children(hint)
                    .into_any_element()
            }
        };
        v_flex()
            .key_context(FORWARD_FORM)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .child(body)
            .child(self.render_footer(cx))
    }
}

/// Opens the editor as the window's modal. An outside click does not close it, and Escape asks
/// first when rows changed.
fn show_metadata_editor(
    editor: Entity<MetadataEditor>,
    title: String,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, _, _| {
        let asked = editor.clone();
        dialog
            .title(title.clone())
            .w(px(DIALOG_WIDTH))
            .child(editor.clone())
            .overlay_closable(false)
            .on_cancel(move |_, window, cx| {
                !ask_before_closing(asked.read(cx).has_unsaved_rows(cx), window, cx)
            })
    });
}

impl AppShell {
    /// Opens Edit labels / annotations on `subject`, the cursor pod or workload, in its own
    /// cluster. The key, the menu item, and the palette entry all end here; the gate is checked
    /// again (a stale menu or a key pressed in a gap cannot bypass it), and the object is read from
    /// that cluster's own connection when the dialog opens.
    pub(crate) fn open_metadata_editor(
        &mut self,
        subject: ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::EditMetadata(ObjectKind::Pod));
        let (Some(ResourceAction::EditMetadata(kind)), Some(object)) = (
            subject_action(RowAction::EditMetadata, &subject.key),
            object_ref(&subject.key),
        ) else {
            notify(
                window,
                cx,
                unavailable_text(label, "this object has no labels to edit here"),
            );
            return;
        };
        let (cluster_name, connection) = {
            let (Some(guard), Some(live)) = (
                self.guard_for(&subject.cluster, cx),
                self.live_of(&subject.cluster, cx),
            ) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            if let ActionAvailability::Disabled { reason } =
                action_availability(ResourceAction::EditMetadata(kind), &guard)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            (
                SharedString::from(guard.display_name().to_owned()),
                live.connection().clone(),
            )
        };
        let target = EditorTarget {
            cluster: subject.cluster,
            cluster_name,
            kind,
            object,
        };
        let title = format!(
            "Edit labels / annotations of {} {}",
            target.object.kind_name().to_ascii_lowercase(),
            target.object.name()
        );
        let runtime = cx.global::<ClusterRuntime>().clone();
        let shell = cx.weak_entity();
        let editor =
            cx.new(|cx| MetadataEditor::new(shell, target, connection, runtime, window, cx));
        #[cfg(test)]
        {
            self.last_metadata_editor = Some(editor.downgrade());
        }
        show_metadata_editor(editor, title, window, cx);
    }
}

/// What the shell tests read from an open editor and how they type into it.
#[cfg(test)]
impl MetadataEditor {
    pub(crate) fn is_loaded(&self) -> bool {
        matches!(self.state, EditorState::Ready { .. })
    }

    pub(crate) fn failure(&self) -> Option<SharedString> {
        match &self.state {
            EditorState::Failed(text) => Some(text.clone()),
            _ => None,
        }
    }

    /// The rows of a list as typed now.
    pub(crate) fn rows_of(&self, list: MetadataList, cx: &App) -> Vec<MetadataRow> {
        self.rows(cx)
            .map(|(labels, annotations)| match list {
                MetadataList::Labels => labels,
                MetadataList::Annotations => annotations,
            })
            .unwrap_or_default()
    }

    /// How many annotations stay hidden (an applied manifest).
    pub(crate) fn hidden_annotation_count(&self) -> usize {
        match &self.state {
            EditorState::Ready { edit, .. } => edit.hidden_annotations.len(),
            _ => 0,
        }
    }

    /// Adds a row with the given text, as the Add button and typing would.
    pub(crate) fn add_row_with(
        &mut self,
        list: MetadataList,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_row(list, window, cx);
        let Some(first) = self.inputs_mut(list).and_then(|inputs| {
            inputs
                .first()
                .map(|row| (row.key.clone(), row.value.clone()))
        }) else {
            return;
        };
        first
            .0
            .update(cx, |input, cx| input.set_value(key.to_owned(), window, cx));
        first.1.update(cx, |input, cx| {
            input.set_value(value.to_owned(), window, cx)
        });
    }

    pub(crate) fn press_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.review(window, cx);
    }

    pub(crate) fn current_intent(&self, cx: &App) -> Result<WriteIntent, SharedString> {
        self.intent(cx)
    }
}

#[cfg(test)]
impl MetadataEditor {
    pub(crate) fn remove_label_for_test(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remove_row(MetadataList::Labels, index, window, cx);
    }
}
