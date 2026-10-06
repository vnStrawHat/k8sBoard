//! The taint and label editors of a node (spec 0034 step 2) and the bulk Cordon and Uncordon of
//! the Nodes selection bar. The editor reads the node fresh from its own cluster, collects rows,
//! and hands the result to the guarded flow (`start_write`), which shows the confirm dialog with
//! the tier, the server dry-run, and the audit line. Nothing here sends a change.
//!
//! A child of `app_shell`, like `node_shell_open`: every step names the cluster of the node and
//! takes its guard, connection, and tier from that cluster's own slot, never from the primary.

use cluster::{ClusterConnection, ClusterError, LabelChange, NodeEdit, NodeTaint};
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IndexPath, Sizable as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};

use super::AppShell;
use super::Screen;
use super::batch_write::{BATCH_RUNNING_REASON, MAX_BATCH_ITEMS};
use super::write_flow::{WriteIntent, notify};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::fresh_enter::{confirms, is_enter};
use crate::keymap::FORWARD_FORM;
use crate::node_edits::{
    CordonMode, KEY_HINT, LabelRow, NO_EXECUTE_WARNING, NodeScope, RowField, RowProblem, TaintRow,
    TickedNode, conflict_notice, cordon_batch, label_batch, label_intent, label_row_problem,
    label_rows, node_names_text, rows_after_conflict, taint_intent, taint_row_problem, taint_rows,
};
use crate::resource_actions::{
    ActionAvailability, NOT_SHIPPED_REASON, ResourceAction, action_availability, action_label,
    unavailable_text,
};
use crate::row_selection::{BulkButton, BulkState, bulk_actions};
use crate::table_selection::{ClusterObject, ResourceKey};

const DIALOG_WIDTH: f32 = 560.;
const ROWS_MAX_HEIGHT: f32 = 320.;
const EFFECT_CHOICES: [&str; 3] = ["NoSchedule", "PreferNoSchedule", "NoExecute"];
const MANAGED_BY_KUBERNETES: &str = "Managed by Kubernetes";
/// The operations of a bulk label row, in select order.
const BULK_OPERATIONS: [&str; 2] = ["Set", "Remove"];
const BULK_REMOVE: usize = 1;
const SET_BY_KUBELET: &str = "Set by the kubelet";
/// The taints a taint editor read when it sent its change to review.
pub(crate) struct TaintBase {
    pub(crate) cluster: ClusterRef,
    pub(crate) node: String,
    pub(crate) taints: Vec<NodeTaint>,
}

/// What a taint editor reopened after a conflict carries over: the user's rows, and the taints
/// the editor first read, when they are known.
pub(crate) struct KeptEdit {
    pub(crate) rows: Vec<TaintRow>,
    pub(crate) base: Option<Vec<NodeTaint>>,
}

/// Which list the editor changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeEditKind {
    Taints,
    Labels,
}

impl NodeEditKind {
    pub(crate) fn action(self) -> ResourceAction {
        match self {
            Self::Taints => ResourceAction::EditTaints,
            Self::Labels => ResourceAction::EditLabels,
        }
    }

    fn title(self, node: &str) -> String {
        match self {
            Self::Taints => format!("Edit taints of node {node}"),
            Self::Labels => format!("Edit labels of node {node}"),
        }
    }
}

struct TaintInputs {
    key: Entity<InputState>,
    value: Entity<InputState>,
    effect: Entity<SelectState<Vec<String>>>,
    time_added: Option<jiff::Timestamp>,
    is_read_only: bool,
}

struct LabelInputs {
    key: Entity<InputState>,
    value: Entity<InputState>,
    is_read_only: bool,
}

enum Rows {
    Taints(Vec<TaintInputs>),
    Labels(Vec<LabelInputs>),
}

enum EditorState {
    Loading,
    Failed(SharedString),
    Ready { edit: NodeEdit, rows: Rows },
}

/// The body of the editor dialog.
pub(crate) struct NodeEditor {
    shell: WeakEntity<AppShell>,
    cluster: ClusterRef,
    cluster_name: SharedString,
    node: String,
    kind: NodeEditKind,
    notice: Option<SharedString>,
    /// The rows and base of the editor a conflict closed; used once, when the node is read.
    kept: Option<KeptEdit>,
    state: EditorState,
    _load: Option<Task<()>>,
}

fn text_input(
    value: &str,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder(placeholder);
        input.set_value(value.to_owned(), window, cx);
        input
    })
}

fn effect_select(
    effect: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SelectState<Vec<String>>> {
    let choices: Vec<String> = EFFECT_CHOICES
        .iter()
        .map(|text| (*text).to_owned())
        .collect();
    let selected = EFFECT_CHOICES
        .iter()
        .position(|choice| *choice == effect)
        .unwrap_or(0);
    cx.new(|cx| {
        SelectState::new(
            choices,
            Some(IndexPath::default().row(selected)),
            window,
            cx,
        )
    })
}

fn taint_inputs(row: &TaintRow, window: &mut Window, cx: &mut App) -> TaintInputs {
    TaintInputs {
        key: text_input(&row.key, "key", window, cx),
        value: text_input(&row.value, "value (optional)", window, cx),
        effect: effect_select(&row.effect, window, cx),
        time_added: row.time_added,
        is_read_only: row.is_read_only(),
    }
}

fn label_inputs(row: &LabelRow, window: &mut Window, cx: &mut App) -> LabelInputs {
    LabelInputs {
        key: text_input(&row.key, "key", window, cx),
        value: text_input(&row.value, "value", window, cx),
        is_read_only: row.is_read_only(),
    }
}

/// A row line with the reason it is read-only under it, so the inputs of every row keep one width.
fn with_note(line: gpui_kit::Div, note: Option<&'static str>, muted: gpui_kit::Hsla) -> AnyElement {
    v_flex()
        .gap_0p5()
        .child(line)
        .children(note.map(|text| div().text_xs().text_color(muted).child(text)))
        .into_any_element()
}

/// An input cell; the danger border marks the input the validation line names. The border is
/// always there (transparent when fine) so a row keeps its height.
fn input_cell(input: Input, is_bad: bool, danger: gpui_kit::Hsla) -> gpui_kit::Div {
    div()
        .flex_1()
        .rounded_md()
        .border_1()
        .border_color(if is_bad { danger } else { danger.opacity(0.) })
        .child(input)
}

/// The key of a locked taint: a cell cut with an ellipsis, with the full key in a tooltip.
fn locked_key(index: usize, key: SharedString, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let full = key.clone();
    div()
        .id(("node-edit-locked-key", index))
        .flex_1()
        .min_w_0()
        .truncate()
        .px_2()
        .py_1()
        .text_sm()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .text_color(theme.muted_foreground)
        .child(key)
        .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
        .into_any_element()
}

/// The editor state for a node as it was read: one row per taint or label.
fn ready(
    kind: NodeEditKind,
    edit: NodeEdit,
    kept_taints: Option<Vec<TaintRow>>,
    window: &mut Window,
    cx: &mut App,
) -> EditorState {
    let rows = match kind {
        NodeEditKind::Taints => Rows::Taints(
            kept_taints
                .unwrap_or_else(|| taint_rows(&edit))
                .iter()
                .map(|row| taint_inputs(row, window, cx))
                .collect(),
        ),
        NodeEditKind::Labels => Rows::Labels(
            label_rows(&edit)
                .iter()
                .map(|row| label_inputs(row, window, cx))
                .collect(),
        ),
    };
    EditorState::Ready { edit, rows }
}

impl NodeEditor {
    fn new(
        shell: WeakEntity<AppShell>,
        target: EditorTarget,
        kept: Option<KeptEdit>,
        connection: ClusterConnection,
        runtime: ClusterRuntime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let EditorTarget {
            cluster,
            cluster_name,
            node,
            kind,
        } = target;
        let name = node.clone();
        let load = cx.spawn_in(window, async move |this, cx| {
            let read = runtime
                .spawn(async move { connection.node_for_edit(&name).await })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| editor.loaded(read, window, cx));
        });
        Self {
            shell,
            cluster,
            cluster_name,
            node,
            kind,
            notice: None,
            kept,
            state: EditorState::Loading,
            _load: Some(load),
        }
    }

    fn loaded(
        &mut self,
        read: Result<Result<NodeEdit, ClusterError>, tokio::task::JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state = match read {
            Ok(Ok(edit)) => {
                let kept = self.kept.take();
                if let Some(kept) = &kept {
                    self.notice = Some(conflict_notice(kept.base.as_deref(), &edit).into());
                }
                let rows =
                    kept.map(|kept| rows_after_conflict(&edit, kept.base.as_deref(), &kept.rows));
                ready(self.kind, edit, rows, window, cx)
            }
            Ok(Err(error)) => {
                EditorState::Failed(format!("Could not read node {}: {error}", self.node).into())
            }
            Err(_) => EditorState::Failed("The request task stopped".into()),
        };
        cx.notify();
    }

    fn read_taints(inputs: &[TaintInputs], cx: &App) -> Vec<TaintRow> {
        inputs
            .iter()
            .map(|row| TaintRow {
                key: row.key.read(cx).value().to_string(),
                value: row.value.read(cx).value().to_string(),
                effect: row
                    .effect
                    .read(cx)
                    .selected_index(cx)
                    .and_then(|index| EFFECT_CHOICES.get(index.row))
                    .map_or_else(String::new, |effect| (*effect).to_owned()),
                time_added: row.time_added,
            })
            .collect()
    }

    fn read_labels(inputs: &[LabelInputs], cx: &App) -> Vec<LabelRow> {
        inputs
            .iter()
            .map(|row| LabelRow {
                key: row.key.read(cx).value().to_string(),
                value: row.value.read(cx).value().to_string(),
            })
            .collect()
    }

    /// The change the rows describe now, or why there is none. `No changes` keeps Review off.
    fn intent(&self, cx: &App) -> Result<WriteIntent, SharedString> {
        let EditorState::Ready { edit, rows } = &self.state else {
            return Err("Loading node…".into());
        };
        let scope = NodeScope {
            cluster: &self.cluster,
            cluster_name: &self.cluster_name,
        };
        match rows {
            Rows::Taints(inputs) => {
                taint_intent(&scope, &self.node, edit, &Self::read_taints(inputs, cx))
            }
            Rows::Labels(inputs) => {
                label_intent(&scope, &self.node, edit, &Self::read_labels(inputs, cx))
            }
        }
    }

    /// The row and input that carry the validation line, to mark them.
    fn row_problem(&self, cx: &App) -> Option<RowProblem> {
        let EditorState::Ready { rows, .. } = &self.state else {
            return None;
        };
        match rows {
            Rows::Taints(inputs) => taint_row_problem(&Self::read_taints(inputs, cx)),
            Rows::Labels(inputs) => label_row_problem(&Self::read_labels(inputs, cx)),
        }
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let EditorState::Ready { rows, .. } = &mut self.state else {
            return;
        };
        match rows {
            Rows::Taints(inputs) => {
                let row = TaintRow {
                    key: String::new(),
                    value: String::new(),
                    effect: EFFECT_CHOICES[0].to_owned(),
                    time_added: None,
                };
                inputs.push(taint_inputs(&row, window, cx));
            }
            Rows::Labels(inputs) => {
                let row = LabelRow {
                    key: String::new(),
                    value: String::new(),
                };
                inputs.push(label_inputs(&row, window, cx));
            }
        }
        cx.notify();
    }

    fn remove_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let EditorState::Ready { rows, .. } = &mut self.state else {
            return;
        };
        match rows {
            Rows::Taints(inputs) if inputs.get(index).is_some_and(|row| !row.is_read_only) => {
                inputs.remove(index);
            }
            Rows::Labels(inputs) if inputs.get(index).is_some_and(|row| !row.is_read_only) => {
                inputs.remove(index);
            }
            Rows::Taints(_) | Rows::Labels(_) => return,
        }
        cx.notify();
    }

    /// Review…: closes the editor and starts the guarded flow, whose dialog follows. Nothing is
    /// sent from here.
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(intent) = self.intent(cx) else {
            return;
        };
        let base = match &self.state {
            EditorState::Ready { edit, .. } if self.kind == NodeEditKind::Taints => {
                Some(TaintBase {
                    cluster: self.cluster.clone(),
                    node: self.node.clone(),
                    taints: edit.taints.clone(),
                })
            }
            _ => None,
        };
        let shell = self.shell.clone();
        window.close_dialog(cx);
        // After the close: the flow opens the confirm dialog, which the close must not pop.
        window.defer(cx, move |window, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.taint_base = base;
                shell.start_write(intent, window, cx);
            });
        });
    }

    /// Enter in a row's text field presses Review…, a fresh press only. A focused button or
    /// select keeps its own Enter.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !is_enter(event) || !self.is_input_focused(window, cx) {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if confirms(event) {
            self.review(window, cx);
        }
    }

    fn is_input_focused(&self, window: &Window, cx: &App) -> bool {
        let EditorState::Ready { rows, .. } = &self.state else {
            return false;
        };
        let is_focused =
            |input: &Entity<InputState>| input.read(cx).focus_handle(cx).is_focused(window);
        match rows {
            Rows::Taints(rows) => rows
                .iter()
                .any(|row| is_focused(&row.key) || is_focused(&row.value)),
            Rows::Labels(rows) => rows
                .iter()
                .any(|row| is_focused(&row.key) || is_focused(&row.value)),
        }
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let EditorState::Ready { rows, .. } = &self.state else {
            return div().into_any_element();
        };
        let (muted, danger) = (cx.theme().muted_foreground, cx.theme().danger);
        let problem = self.row_problem(cx);
        let is_bad = |index: usize, field: RowField| {
            problem
                .as_ref()
                .is_some_and(|problem| problem.index == index && problem.field == field)
        };
        let remove = |index: usize, is_read_only: bool, cx: &mut Context<Self>| {
            if is_read_only {
                return div().w_6().into_any_element();
            }
            Button::new(("node-edit-remove", index))
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::X))
                .tooltip("Remove")
                .on_click(cx.listener(move |editor, _, _, cx| editor.remove_row(index, cx)))
                .into_any_element()
        };
        let list =
            match rows {
                Rows::Taints(inputs) => inputs
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let key_cell = if row.is_read_only {
                            locked_key(index, row.key.read(cx).value().clone(), cx)
                        } else {
                            input_cell(
                                Input::new(&row.key).small(),
                                is_bad(index, RowField::Key),
                                danger,
                            )
                            .into_any_element()
                        };
                        let has_no_execute = !row.is_read_only
                            && row
                                .effect
                                .read(cx)
                                .selected_index(cx)
                                .and_then(|index| EFFECT_CHOICES.get(index.row))
                                .is_some_and(|effect| *effect == "NoExecute");
                        let (note, note_color) = if row.is_read_only {
                            (Some(MANAGED_BY_KUBERNETES), muted)
                        } else if has_no_execute {
                            (Some(NO_EXECUTE_WARNING), cx.theme().warning)
                        } else {
                            (None, muted)
                        };
                        with_note(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(key_cell)
                                .child(input_cell(
                                    Input::new(&row.value).small().disabled(row.is_read_only),
                                    is_bad(index, RowField::Value),
                                    danger,
                                ))
                                .child(div().w(px(150.)).child(
                                    Select::new(&row.effect).small().disabled(row.is_read_only),
                                ))
                                .child(remove(index, row.is_read_only, cx)),
                            note,
                            note_color,
                        )
                    })
                    .collect::<Vec<_>>(),
                Rows::Labels(inputs) => inputs
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        with_note(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(input_cell(
                                    Input::new(&row.key).small().disabled(row.is_read_only),
                                    is_bad(index, RowField::Key),
                                    danger,
                                ))
                                .child(input_cell(
                                    Input::new(&row.value).small().disabled(row.is_read_only),
                                    is_bad(index, RowField::Value),
                                    danger,
                                ))
                                .child(remove(index, row.is_read_only, cx)),
                            row.is_read_only.then_some(SET_BY_KUBELET),
                            muted,
                        )
                    })
                    .collect::<Vec<_>>(),
            };
        v_flex()
            .id("node-edit-rows")
            .gap_1()
            .max_h(px(ROWS_MAX_HEIGHT))
            .overflow_y_scroll()
            .children(list)
            .into_any_element()
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let intent = self.intent(cx);
        let is_ready = matches!(self.state, EditorState::Ready { .. });
        let review = Button::new("node-edit-review")
            .label("Review…")
            .small()
            .primary()
            .disabled(intent.is_err())
            .on_click(cx.listener(|editor, _, window, cx| editor.review(window, cx)));
        let review = match &intent {
            Err(reason) if is_ready => review.tooltip(reason.clone()),
            _ => review,
        };
        h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(
                Button::new("node-edit-cancel")
                    .label("Cancel")
                    .small()
                    .outline()
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(review)
            .into_any_element()
    }
}

impl Render for NodeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, warning, danger) = (theme.muted_foreground, theme.warning, theme.danger);
        let body: AnyElement = match &self.state {
            EditorState::Loading => h_flex()
                .gap_2()
                .items_center()
                .child(Spinner::new())
                .child(div().text_sm().text_color(muted).child("Loading node…"))
                .into_any_element(),
            EditorState::Failed(text) => div()
                .text_sm()
                .text_color(danger)
                .child(text.clone())
                .into_any_element(),
            EditorState::Ready { .. } => {
                let add = Button::new("node-edit-add")
                    .icon(Icon::new(IconName::Plus))
                    .label("Add")
                    .small()
                    .outline()
                    .on_click(cx.listener(|editor, _, window, cx| editor.add_row(window, cx)));
                let problem = match self.intent(cx) {
                    Err(reason) if reason.as_ref() != "No changes" => Some(reason),
                    _ => None,
                };
                let hint = self
                    .row_problem(cx)
                    .map(|_| div().text_xs().text_color(muted).child(KEY_HINT));
                v_flex()
                    .gap_2()
                    .child(self.render_rows(cx))
                    .child(h_flex().child(add))
                    .children(problem.map(|text| div().text_sm().text_color(danger).child(text)))
                    .children(hint)
                    .into_any_element()
            }
        };
        v_flex()
            .key_context(FORWARD_FORM)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .children(
                self.notice
                    .clone()
                    .map(|text| div().text_sm().text_color(warning).child(text)),
            )
            .child(body)
            .child(self.render_footer(cx))
    }
}

/// What an editor is opened on: the node and the cluster it belongs to.
struct EditorTarget {
    cluster: ClusterRef,
    cluster_name: SharedString,
    node: String,
    kind: NodeEditKind,
}

impl AppShell {
    /// Reopens the taint editor a conflict closed, with the user's rows and what changed on the
    /// node since the editor first read it.
    pub(crate) fn reopen_taint_editor(
        &mut self,
        cluster: &ClusterRef,
        node: &str,
        rows: Vec<TaintRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let base = self
            .taint_base
            .take()
            .filter(|base| base.cluster == *cluster && base.node == node)
            .map(|base| base.taints);
        let kept = KeptEdit { rows, base };
        self.open_node_editor(NodeEditKind::Taints, cluster, node, Some(kept), window, cx);
    }

    /// Opens the taint or label editor of `node` of `cluster`, the row's or cursor's own cluster.
    /// The gate is checked here again (a stale menu or a key pressed in a gap cannot bypass it),
    /// and the node is read from that cluster's own connection when the dialog opens. `kept` is
    /// what a taint editor that a conflict closed carries into its reopening.
    pub(crate) fn open_node_editor(
        &mut self,
        kind: NodeEditKind,
        cluster: &ClusterRef,
        node: &str,
        kept: Option<KeptEdit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(kind.action());
        let (target, connection) = {
            let (Some(guard), Some(live)) =
                (self.guard_for(cluster, cx), self.live_of(cluster, cx))
            else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            if let ActionAvailability::Disabled { reason } =
                action_availability(kind.action(), &guard)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            if let Some(reason) = self.drain_conflict(cluster, kind.action(), cx) {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            if !live
                .nodes
                .items()
                .iter()
                .any(|summary| summary.name == node)
            {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the node is no longer listed"),
                );
                return;
            }
            let target = EditorTarget {
                cluster: cluster.clone(),
                cluster_name: guard.display_name().to_owned().into(),
                node: node.to_owned(),
                kind,
            };
            (target, live.connection().clone())
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let (shell, title) = (cx.weak_entity(), kind.title(node));
        let editor =
            cx.new(|cx| NodeEditor::new(shell, target, kept, connection, runtime, window, cx));
        #[cfg(test)]
        {
            self.last_node_editor = Some(editor.downgrade());
        }
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(title.clone())
                .w(px(DIALOG_WIDTH))
                .child(editor.clone())
        });
    }

    /// The Edit labels button of the Nodes header: the 0034 editor for one ticked node, the bulk
    /// editor for 2 to 50 (spec 0040).
    pub(super) fn node_header_buttons(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let button = || {
            Button::new("node-edit-labels")
                .label("Edit labels")
                .small()
                .outline()
        };
        let target = match self.edit_labels_target(cx) {
            Ok(target) => target,
            Err(reason) => return vec![button().disabled(true).tooltip(reason).into_any_element()],
        };
        let button = match target {
            LabelTarget::One { cluster, node } => button()
                .tooltip("Edit the labels of the ticked node")
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.open_node_editor(NodeEditKind::Labels, &cluster, &node, None, window, cx);
                })),
            LabelTarget::Several { cluster, nodes } => {
                let names: Vec<String> = nodes.into_iter().map(|node| node.name).collect();
                button()
                    .tooltip("Edit the labels of the ticked nodes")
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.open_bulk_label_editor(&cluster, &names, window, cx);
                    }))
            }
        };
        vec![button.into_any_element()]
    }

    /// What the header's Edit labels acts on: the one ticked node, or the 2 to 50 ticked nodes of
    /// one cluster, each in that cluster; or why the button is off.
    pub(super) fn edit_labels_target(&self, cx: &App) -> Result<LabelTarget, SharedString> {
        const TICK_FIRST: &str = "Tick nodes first";
        let ticked = self.checked_objects(cx);
        let target = match ticked.as_slice() {
            [] => return Err(TICK_FIRST.into()),
            [only] => {
                let ResourceKey::Node { name } = &only.key else {
                    return Err(TICK_FIRST.into());
                };
                LabelTarget::One {
                    cluster: only.cluster.clone(),
                    node: name.clone(),
                }
            }
            _ => {
                let (cluster, nodes) = self.ticked_nodes(&ticked, cx)?;
                LabelTarget::Several { cluster, nodes }
            }
        };
        let cluster = match &target {
            LabelTarget::One { cluster, .. } | LabelTarget::Several { cluster, .. } => cluster,
        };
        let guard = self.guard_for(cluster, cx).ok_or("Not connected")?;
        if let ActionAvailability::Disabled { reason } =
            action_availability(ResourceAction::EditLabels, &guard)
        {
            return Err(reason);
        }
        // A bulk is a batch: it cannot start while another one commits on the cluster.
        if matches!(target, LabelTarget::Several { .. }) && self.running_batches.contains(cluster) {
            return Err(BATCH_RUNNING_REASON.into());
        }
        Ok(target)
    }

    /// The ticked nodes as their own cluster reports them: `Err` is why the bulk buttons are off.
    pub(super) fn ticked_nodes(
        &self,
        ticked: &[ClusterObject],
        cx: &App,
    ) -> Result<(ClusterRef, Vec<TickedNode>), SharedString> {
        let first = ticked.first().ok_or("Select rows first")?;
        if ticked.len() > MAX_BATCH_ITEMS {
            return Err(format!("Select at most {MAX_BATCH_ITEMS} rows").into());
        }
        if ticked.iter().any(|object| object.cluster != first.cluster) {
            return Err("Select rows of one cluster".into());
        }
        let live = self.live_of(&first.cluster, cx).ok_or("Not connected")?;
        let nodes: Vec<TickedNode> = ticked
            .iter()
            .filter_map(|object| {
                let ResourceKey::Node { name } = &object.key else {
                    return None;
                };
                let summary = live.nodes.items().iter().find(|node| node.name == *name)?;
                Some(TickedNode {
                    name: name.clone(),
                    scheduling: summary.status.scheduling,
                    labels: summary.labels.clone(),
                })
            })
            .collect();
        if nodes.is_empty() {
            return Err("The selected nodes are no longer listed".into());
        }
        Ok((first.cluster.clone(), nodes))
    }

    /// The bulk batch of Cordon or Uncordon over the ticked nodes now.
    fn node_cordon_batch(
        &self,
        mode: CordonMode,
        cx: &App,
    ) -> Result<crate::app_shell::batch_write::BatchIntent, SharedString> {
        let ticked = self.checked_objects(cx);
        let (cluster, nodes) = self.ticked_nodes(&ticked, cx)?;
        let guard = self.guard_for(&cluster, cx).ok_or("Not connected")?;
        let scope = NodeScope {
            cluster: &cluster,
            cluster_name: guard.display_name(),
        };
        cordon_batch(&scope, mode, &nodes)
    }

    /// The bulk buttons of the Nodes screen, each decided for the ticked nodes now: the gate of
    /// the nodes' cluster first, then what the action would do with them.
    pub(super) fn node_bulk_buttons(&self, cx: &App) -> Vec<BulkButton> {
        let ticked = self.checked_objects(cx);
        bulk_actions(Screen::Nodes)
            .iter()
            .map(|item| BulkButton {
                label: item.label.into(),
                state: match item.action {
                    Some(action) => self.node_bulk_state(action, &ticked, cx),
                    None => BulkState::Off(NOT_SHIPPED_REASON.into()),
                },
                is_danger: false,
            })
            .collect()
    }

    fn node_bulk_state(
        &self,
        action: ResourceAction,
        ticked: &[ClusterObject],
        cx: &App,
    ) -> BulkState {
        let (cluster, _) = match self.ticked_nodes(ticked, cx) {
            Ok(found) => found,
            Err(reason) => return BulkState::Off(reason),
        };
        let Some(guard) = self.guard_for(&cluster, cx) else {
            return BulkState::Off("Not connected".into());
        };
        // Drain still opens, as a read-only preview, when the gate says no.
        let mut preview = None;
        if let ActionAvailability::Disabled { reason } = action_availability(action, &guard) {
            if action != ResourceAction::Drain {
                return BulkState::Off(reason);
            }
            preview = Some(reason);
        }
        if let Some(reason) = self.drain_conflict(&cluster, action, cx) {
            return BulkState::Off(reason.into());
        }
        // A drain would interleave its commits with the running batch of the cluster.
        if action == ResourceAction::Drain && self.running_batches.contains(&cluster) {
            return BulkState::Off(BATCH_RUNNING_REASON.into());
        }
        let mode = match action {
            ResourceAction::Cordon => CordonMode::Cordon,
            ResourceAction::Uncordon => CordonMode::Uncordon,
            ResourceAction::EditLabels => {
                return self
                    .edit_labels_target(cx)
                    .map_or_else(BulkState::Off, |_| BulkState::Ready(action));
            }
            // Drain has its own dialog; the gate and the one drain per cluster decide its button.
            _ => {
                if self.has_running_drain(&cluster, cx) {
                    return BulkState::Off(
                        format!("A drain is already running on {}", guard.display_name()).into(),
                    );
                }
                return preview.map_or(BulkState::Ready(action), |reason| {
                    BulkState::Preview(action, reason)
                });
            }
        };
        if self.running_batches.contains(&cluster) {
            return BulkState::Off(BATCH_RUNNING_REASON.into());
        }
        match self.node_cordon_batch(mode, cx) {
            Ok(_) => BulkState::Ready(action),
            Err(reason) => BulkState::Off(reason),
        }
    }

    /// A bulk button of the Nodes selection bar: Cordon and Uncordon build their batch from the
    /// nodes ticked now and open the list dialog; Drain opens its own dialog.
    pub(super) fn run_node_bulk(
        &mut self,
        action: ResourceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = match action {
            ResourceAction::Cordon => CordonMode::Cordon,
            ResourceAction::Uncordon => CordonMode::Uncordon,
            ResourceAction::EditLabels => {
                self.open_label_editor_of_ticked(window, cx);
                return;
            }
            _ => {
                self.start_drain_of_ticked(window, cx);
                return;
            }
        };
        match self.node_cordon_batch(mode, cx) {
            Ok(intent) => self.start_batch(intent, window, cx),
            Err(reason) => notify(window, cx, unavailable_text(action_label(action), &reason)),
        }
    }

    /// Edit labels of the Nodes selection bar: the editor of the one ticked node, or the bulk
    /// editor for 2 to 50, as the header button opens them.
    fn open_label_editor_of_ticked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.edit_labels_target(cx) {
            Ok(LabelTarget::One { cluster, node }) => {
                self.open_node_editor(NodeEditKind::Labels, &cluster, &node, None, window, cx);
            }
            Ok(LabelTarget::Several { cluster, nodes }) => {
                let names: Vec<String> = nodes.into_iter().map(|node| node.name).collect();
                self.open_bulk_label_editor(&cluster, &names, window, cx);
            }
            Err(reason) => notify(
                window,
                cx,
                unavailable_text(action_label(ResourceAction::EditLabels), &reason),
            ),
        }
    }
}

/// What the header's Edit labels acts on (spec 0040).
pub(super) enum LabelTarget {
    One {
        cluster: ClusterRef,
        node: String,
    },
    Several {
        cluster: ClusterRef,
        nodes: Vec<TickedNode>,
    },
}

/// One row of the bulk label editor: a key, Set or Remove, and the value of a Set.
struct BulkRow {
    key: Entity<InputState>,
    operation: Entity<SelectState<Vec<String>>>,
    value: Entity<InputState>,
    /// Dropped with the row, so a removed row stops calling back.
    _subscriptions: Vec<Subscription>,
}

impl BulkRow {
    fn is_remove(&self, cx: &App) -> bool {
        self.operation
            .read(cx)
            .selected_index(cx)
            .is_some_and(|index| index.row == BULK_REMOVE)
    }
}

/// The body of the bulk label editor: changes only (Set key=value, Remove key), not the labels of
/// the nodes, which differ per node. It reads no node; Review… builds the batch from the nodes
/// ticked at that moment.
pub(crate) struct BulkLabelEditor {
    shell: WeakEntity<AppShell>,
    cluster: ClusterRef,
    /// The ticked nodes by name, as `node_names_text` writes them.
    targets: SharedString,
    rows: Vec<BulkRow>,
    /// Why the batch of the changes now would not go; worked out when a row changes, not on every
    /// draw (it reads every ticked node).
    problem: Option<SharedString>,
}

impl BulkLabelEditor {
    fn new(
        shell: WeakEntity<AppShell>,
        cluster: ClusterRef,
        targets: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self {
            shell,
            cluster,
            targets,
            rows: Vec::new(),
            problem: None,
        };
        editor.add_row(window, cx);
        editor
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = text_input("", "key", window, cx);
        let value = text_input("", "value", window, cx);
        let choices: Vec<String> = BULK_OPERATIONS
            .iter()
            .map(|text| (*text).to_owned())
            .collect();
        let operation =
            cx.new(|cx| SelectState::new(choices, Some(IndexPath::default().row(0)), window, cx));
        // The problem line and the Remove layout follow what is typed and picked.
        let mut subscriptions: Vec<Subscription> = [&key, &value]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |editor, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        editor.refresh_problem(cx);
                    }
                })
            })
            .collect();
        subscriptions.push(cx.subscribe_in(
            &operation,
            window,
            |editor, _, _: &SelectEvent<Vec<String>>, _, cx| editor.refresh_problem(cx),
        ));
        self.rows.push(BulkRow {
            key,
            operation,
            value,
            _subscriptions: subscriptions,
        });
        self.refresh_problem(cx);
    }

    fn remove_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.rows.len() {
            self.rows.remove(index);
            self.refresh_problem(cx);
        }
    }

    /// Works the problem line out again, and draws.
    fn refresh_problem(&mut self, cx: &mut Context<Self>) {
        self.problem = self.compute_problem(cx);
        cx.notify();
    }

    /// The changes the non-empty rows describe now. A row with nothing in it is not a change.
    fn changes(&self, cx: &App) -> Vec<LabelChange> {
        self.rows
            .iter()
            .filter_map(|row| {
                let key = row.key.read(cx).value().to_string();
                let value = row.value.read(cx).value().to_string();
                let is_remove = row.is_remove(cx);
                if key.trim().is_empty() && (is_remove || value.trim().is_empty()) {
                    return None;
                }
                Some(LabelChange {
                    key,
                    value: (!is_remove).then_some(value),
                })
            })
            .collect()
    }

    /// Why the batch of the changes now would not go, over the nodes ticked now. `None` while there
    /// is no change to judge or no ticked node to judge it over (the fixture has none).
    fn compute_problem(&self, cx: &App) -> Option<SharedString> {
        let changes = self.changes(cx);
        if changes.is_empty() {
            return None;
        }
        let shell = self.shell.upgrade()?;
        let shell = shell.read(cx);
        let ticked = shell.checked_objects(cx);
        let (cluster, nodes) = shell.ticked_nodes(&ticked, cx).ok()?;
        let guard = shell.guard_for(&cluster, cx)?;
        let scope = NodeScope {
            cluster: &cluster,
            cluster_name: guard.display_name(),
        };
        label_batch(&scope, &nodes, &changes).err()
    }

    /// Review…: closes the editor and starts the guarded batch over the nodes ticked now, whose
    /// dialog follows. Nothing is sent from here.
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let changes = self.changes(cx);
        if changes.is_empty() {
            return;
        }
        let (shell, cluster) = (self.shell.clone(), self.cluster.clone());
        window.close_dialog(cx);
        // After the close: the flow opens the confirm dialog, which the close must not pop.
        window.defer(cx, move |window, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.start_bulk_labels(&cluster, &changes, window, cx);
            });
        });
    }

    /// Enter in a row's text field presses Review…, a fresh press only.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let is_input_focused = self.rows.iter().any(|row| {
            [&row.key, &row.value]
                .into_iter()
                .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
        });
        if !is_enter(event) || !is_input_focused {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if confirms(event) {
            self.review(window, cx);
        }
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let list = self.rows.iter().enumerate().map(|(index, row)| {
            let value_cell = if row.is_remove(cx) {
                div().flex_1().into_any_element()
            } else {
                div()
                    .flex_1()
                    .child(Input::new(&row.value).small())
                    .into_any_element()
            };
            h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().child(Input::new(&row.key).small()))
                .child(div().w(px(110.)).child(Select::new(&row.operation).small()))
                .child(value_cell)
                .child(
                    Button::new(("bulk-label-remove", index))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::X))
                        .tooltip("Remove")
                        .on_click(
                            cx.listener(move |editor, _, _, cx| editor.remove_row(index, cx)),
                        ),
                )
        });
        v_flex()
            .id("bulk-label-rows")
            .gap_1()
            .max_h(px(ROWS_MAX_HEIGHT))
            .overflow_y_scroll()
            .children(list)
            .into_any_element()
    }
}

impl Render for BulkLabelEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let has_changes = !self.changes(cx).is_empty();
        let review = Button::new("bulk-label-review")
            .label("Review…")
            .small()
            .primary()
            .disabled(!has_changes)
            .on_click(cx.listener(|editor, _, window, cx| editor.review(window, cx)));
        let review = if has_changes {
            review
        } else {
            review.tooltip("No changes")
        };
        let add = Button::new("bulk-label-add")
            .icon(Icon::new(IconName::Plus))
            .label("Add")
            .small()
            .outline()
            .on_click(cx.listener(|editor, _, window, cx| editor.add_row(window, cx)));
        v_flex()
            .key_context(FORWARD_FORM)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .child(div().text_sm().child(self.targets.clone()))
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("Changes apply to every ticked node; other labels stay."),
            )
            .child(self.render_rows(cx))
            .child(h_flex().child(add))
            .children(
                self.problem
                    .clone()
                    .map(|text| div().text_sm().text_color(danger).child(text)),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("bulk-label-cancel")
                            .label("Cancel")
                            .small()
                            .outline()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(review),
            )
    }
}

impl AppShell {
    /// Opens the bulk label editor for the ticked nodes `names` of `cluster`. The gate is checked
    /// again here (a stale button or a key pressed in a gap cannot bypass it); the nodes are read
    /// when Review… is pressed.
    pub(super) fn open_bulk_label_editor(
        &mut self,
        cluster: &ClusterRef,
        names: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::EditLabels);
        let Some(guard) = self.guard_for(cluster, cx) else {
            notify(
                window,
                cx,
                unavailable_text(label, "the cluster is not open"),
            );
            return;
        };
        if let ActionAvailability::Disabled { reason } =
            action_availability(ResourceAction::EditLabels, &guard)
        {
            notify(window, cx, unavailable_text(label, &reason));
            return;
        }
        if self.running_batches.contains(cluster) {
            notify(window, cx, unavailable_text(label, BATCH_RUNNING_REASON));
            return;
        }
        let (shell, cluster) = (cx.weak_entity(), cluster.clone());
        let targets = node_names_text(names).into();
        let editor = cx.new(|cx| BulkLabelEditor::new(shell, cluster, targets, window, cx));
        self.show_bulk_label_editor(editor, names.len(), window, cx);
    }

    fn show_bulk_label_editor(
        &mut self,
        editor: Entity<BulkLabelEditor>,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        {
            self.last_bulk_label_editor = Some(editor.downgrade());
        }
        let title = format!("Edit labels of {count} nodes");
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(title.clone())
                .w(px(DIALOG_WIDTH))
                .child(editor.clone())
        });
    }

    /// Review… of the bulk editor: the batch over the nodes ticked now, in the editor's own
    /// cluster, or the reason there is none. The batch dialog follows; nothing is sent from here.
    pub(super) fn start_bulk_labels(
        &mut self,
        cluster: &ClusterRef,
        changes: &[LabelChange],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.bulk_label_batch(cluster, changes, cx) {
            Ok(intent) => self.start_batch(intent, window, cx),
            Err(reason) => notify(
                window,
                cx,
                unavailable_text(action_label(ResourceAction::EditLabels), &reason),
            ),
        }
    }

    fn bulk_label_batch(
        &self,
        cluster: &ClusterRef,
        changes: &[LabelChange],
        cx: &App,
    ) -> Result<crate::app_shell::batch_write::BatchIntent, SharedString> {
        let ticked = self.checked_objects(cx);
        let (found, nodes) = self.ticked_nodes(&ticked, cx)?;
        // The selection may have moved to another cluster since the editor opened.
        if found != *cluster {
            return Err("the ticked nodes are of another cluster".into());
        }
        let guard = self.guard_for(&found, cx).ok_or("Not connected")?;
        let scope = NodeScope {
            cluster: &found,
            cluster_name: guard.display_name(),
        };
        label_batch(&scope, &nodes, changes)
    }
}

/// `--screen node-taints-editor` and `node-labels-editor`: the editors over a fixed node, with no
/// cluster behind them. Review… opens the confirm dialog like the real one, but the fixture's
/// cluster is not open, so nothing can be sent.
#[cfg(feature = "screenshot")]
impl AppShell {
    pub(super) fn open_node_editor_fixture(
        &mut self,
        kind: NodeEditKind,
        extra_taints: &[(&str, &str, &str)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use std::collections::BTreeMap;

        use cluster::NodeTaint;

        const NODE: &str = "wk-04";
        let taint = |key: &str, value: Option<&str>, effect: &str, added: Option<&str>| NodeTaint {
            key: key.to_owned(),
            value: value.map(str::to_owned),
            effect: effect.to_owned(),
            time_added: added.and_then(|text| text.parse().ok()),
        };
        let labels: BTreeMap<String, String> = [
            ("kubernetes.io/hostname", NODE),
            ("kubernetes.io/os", "linux"),
            ("node-role.kubernetes.io/worker", ""),
            ("team", "infra"),
            ("topology.kubernetes.io/zone", "eu-west-1b"),
            ("workload", "ingress"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let edit = NodeEdit {
            taints: vec![
                taint("dedicated", Some("ingress"), "NoSchedule", None),
                taint("gpu", None, "PreferNoSchedule", None),
                taint(
                    "node.kubernetes.io/unschedulable",
                    None,
                    "NoSchedule",
                    Some("2026-10-02T08:00:00Z"),
                ),
            ],
            labels,
            resource_version: "9912".to_owned(),
        };
        let target = EditorTarget {
            cluster: ClusterRef {
                kubeconfig: std::path::PathBuf::from("fixture.yaml"),
                context: "prod-eu-1".to_owned(),
            },
            cluster_name: "prod-eu-1".into(),
            node: NODE.to_owned(),
            kind,
        };
        let shell = cx.weak_entity();
        let title = kind.title(NODE);
        let editor = cx.new(|cx| NodeEditor {
            shell,
            cluster: target.cluster,
            cluster_name: target.cluster_name,
            node: target.node,
            kind,
            notice: None,
            kept: None,
            state: ready(kind, edit, None, window, cx),
            _load: None,
        });
        for (key, value, effect) in extra_taints {
            editor.update(cx, |editor, cx| {
                editor.add_fixture_taint(key, value, effect, window, cx);
            });
        }
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(title.clone())
                .w(px(DIALOG_WIDTH))
                .child(editor.clone())
        });
    }
}

/// `--screen node-labels-bulk-editor`: the bulk editor of three fixed ticked nodes with a Set and a
/// Remove typed. No node is ticked behind it, so Review… could build nothing.
#[cfg(feature = "screenshot")]
impl AppShell {
    pub(super) fn open_bulk_label_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = ["node-a", "node-b", "node-c"].map(str::to_owned);
        let cluster = ClusterRef {
            kubeconfig: std::path::PathBuf::from("fixture.yaml"),
            context: "prod-eu-1".to_owned(),
        };
        let shell = cx.weak_entity();
        let editor = cx.new(|cx| {
            let targets = node_names_text(&names).into();
            let mut editor = BulkLabelEditor::new(shell, cluster, targets, window, cx);
            editor.fill_fixture(window, cx);
            editor
        });
        self.show_bulk_label_editor(editor, names.len(), window, cx);
    }
}

#[cfg(feature = "screenshot")]
impl BulkLabelEditor {
    /// The two rows of the picture: `team = infra` (Set) and `old-key` (Remove).
    fn fill_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_row(window, cx);
        let mut fill = |row: &BulkRow, key: &str, value: &str, operation: usize| {
            row.key
                .update(cx, |input, cx| input.set_value(key.to_owned(), window, cx));
            row.value.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            });
            row.operation.update(cx, |select, cx| {
                select.set_selected_index(Some(IndexPath::default().row(operation)), window, cx);
            });
        };
        if let [first, second] = self.rows.as_slice() {
            fill(first, "team", "infra", 0);
            fill(second, "old-key", "", BULK_REMOVE);
        }
    }
}

#[cfg(feature = "screenshot")]
impl NodeEditor {
    /// Appends a taint row with the given text, as if typed.
    fn add_fixture_taint(
        &mut self,
        key: &str,
        value: &str,
        effect: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_row(window, cx);
        let EditorState::Ready {
            rows: Rows::Taints(rows),
            ..
        } = &self.state
        else {
            return;
        };
        let Some(row) = rows.last() else {
            return;
        };
        row.key
            .update(cx, |input, cx| input.set_value(key.to_owned(), window, cx));
        row.value.update(cx, |input, cx| {
            input.set_value(value.to_owned(), window, cx)
        });
        if let Some(index) = EFFECT_CHOICES.iter().position(|choice| *choice == effect) {
            row.effect.update(cx, |select, cx| {
                select.set_selected_index(Some(IndexPath::default().row(index)), window, cx);
            });
        }
    }
}

/// What the shell tests read from, and do to, an open bulk editor.
#[cfg(test)]
impl BulkLabelEditor {
    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Puts the cursor in the key field of the last row, as a click would.
    pub(crate) fn focus_last_key(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.last() {
            row.key.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    /// Fills the last row; `is_remove` picks Remove, whose value is ignored.
    pub(crate) fn fill_last_row(
        &mut self,
        key: &str,
        value: &str,
        is_remove: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.last() else {
            return;
        };
        row.key
            .update(cx, |input, cx| input.set_value(key.to_owned(), window, cx));
        row.value.update(cx, |input, cx| {
            input.set_value(value.to_owned(), window, cx)
        });
        row.operation.update(cx, |select, cx| {
            let index = if is_remove { BULK_REMOVE } else { 0 };
            select.set_selected_index(Some(IndexPath::default().row(index)), window, cx);
        });
        // Setting a value from code raises no change event, as typing does.
        self.refresh_problem(cx);
    }

    pub(crate) fn push_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_row(window, cx);
    }

    pub(crate) fn current_changes(&self, cx: &App) -> Vec<LabelChange> {
        self.changes(cx)
    }

    pub(crate) fn current_targets(&self) -> &str {
        &self.targets
    }

    pub(crate) fn current_problem(&self) -> Option<SharedString> {
        self.problem.clone()
    }

    /// How many subscriptions the rows hold: one set per row still in the editor.
    pub(crate) fn row_subscription_count(&self) -> usize {
        self.rows.iter().map(|row| row._subscriptions.len()).sum()
    }

    pub(crate) fn drop_row(&mut self, index: usize, cx: &mut Context<Self>) {
        self.remove_row(index, cx);
    }

    pub(crate) fn press_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.review(window, cx);
    }
}

/// What the shell tests read from an open editor.
#[cfg(test)]
impl NodeEditor {
    pub(crate) fn is_loaded(&self) -> bool {
        matches!(self.state, EditorState::Ready { .. })
    }

    pub(crate) fn failure(&self) -> Option<SharedString> {
        match &self.state {
            EditorState::Failed(text) => Some(text.clone()),
            _ => None,
        }
    }

    pub(crate) fn notice(&self) -> Option<SharedString> {
        self.notice.clone()
    }

    pub(crate) fn row_count(&self) -> usize {
        match &self.state {
            EditorState::Ready {
                rows: Rows::Taints(rows),
                ..
            } => rows.len(),
            EditorState::Ready {
                rows: Rows::Labels(rows),
                ..
            } => rows.len(),
            _ => 0,
        }
    }

    /// Appends a row with the given text; a taint row gets `effect`.
    pub(crate) fn add_row_with(
        &mut self,
        key: &str,
        value: &str,
        effect: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_row(window, cx);
        let EditorState::Ready { rows, .. } = &self.state else {
            return;
        };
        let (key_input, value_input, select) = match rows {
            Rows::Taints(rows) => rows
                .last()
                .map(|row| (&row.key, &row.value, Some(&row.effect))),
            Rows::Labels(rows) => rows.last().map(|row| (&row.key, &row.value, None)),
        }
        .expect("a row was added");
        key_input.update(cx, |input, cx| input.set_value(key.to_owned(), window, cx));
        value_input.update(cx, |input, cx| {
            input.set_value(value.to_owned(), window, cx)
        });
        if let Some(select) = select
            && let Some(index) = EFFECT_CHOICES.iter().position(|choice| *choice == effect)
        {
            select.update(cx, |select, cx| {
                select.set_selected_index(Some(IndexPath::default().row(index)), window, cx);
            });
        }
    }

    pub(crate) fn press_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.review(window, cx);
    }

    /// Puts the cursor in the key field of the last row, as a click would.
    pub(crate) fn focus_last_key(&self, window: &mut Window, cx: &mut Context<Self>) {
        let EditorState::Ready { rows, .. } = &self.state else {
            return;
        };
        let key = match rows {
            Rows::Taints(rows) => rows.last().map(|row| row.key.clone()),
            Rows::Labels(rows) => rows.last().map(|row| row.key.clone()),
        };
        if let Some(key) = key {
            key.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub(crate) fn current_intent(&self, cx: &App) -> Result<WriteIntent, SharedString> {
        self.intent(cx)
    }
}
