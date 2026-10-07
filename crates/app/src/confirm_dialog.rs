//! The confirm dialog (spec 0030): one small modal for every guarded action. The write variant
//! shows the object, the changes, the server-side dry-run, and the typed-name field of the
//! `TypeName` tier; the unlock variant shows the same without an object or a dry-run.
//!
//! It never sends anything itself: its buttons call back into the shell (`commit_write`,
//! `finish_unlock`), which re-checks the gate and the lock of the cluster the dialog names.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use cluster::{WriteError, WriteOperation};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Div, Entity, FocusHandle,
    Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity,
    Window, div, px,
};

use crate::app_shell::AppShell;
use crate::app_shell::batch_write::{
    BatchCommit, BatchExtras, BatchFailure, BatchIntent, ItemProgress, dry_run_progress,
    summarize_dry_runs,
};
use crate::app_shell::object_delete::{
    delete_dry_run_progress, has_dependents, pods_without_controller, propagation_choices,
    with_propagation,
};
use crate::app_shell::write_flow::{
    CheckedWriteError, CommitMode, ConnectCommit, ConnectIntent, DryRunState, TypedMatch,
    WriteIntent, WriteStep, checked_write, commit_block, confirmed, dry_run_state_of,
    is_secret_form_action, typed_match, unlock_block,
};
use crate::cluster_registry::ClusterRef;
use crate::counted_text::counted_text;
use crate::environment::{Environment, environment_badge};
use crate::node_edits::taint_rows_of_request;
use crate::port_forwards::{LOCAL_PORT_FIELD_ERROR, LocalPortSpec, parse_local_port_field};
use crate::resource_actions::{ResourceAction, with_next_step};
use crate::settings::AppSettings;
use crate::status_tone::{StatusTone, tone_color};
use crate::write_guard::{ActionRisk, DialogConfirm, confirm_step};

const DIALOG_WIDTH: f32 = 480.;

/// The path `changed_fields` gives an eviction: a request to the eviction subresource, no field of
/// the pod.
const EVICTION_PATH: &str = "pods/eviction";

/// `Eviction request · pod's own grace period`, from the `grace pod default` or `grace 30s` value.
fn eviction_line(grace: &str) -> String {
    match grace.strip_prefix("grace ") {
        Some("pod default") => "Eviction request · pod's own grace period".to_owned(),
        Some(seconds) => format!("Eviction request · grace period {seconds}"),
        None => format!("Eviction request · {grace}"),
    }
}

/// The field Trigger now and Re-run send: the name prefix of the Job the server creates.
const GENERATE_NAME_PATH: &str = "metadata.generateName";

/// One changed field: `spec.replicas: 3 → 0` when the old value is known, else `path → new`.
fn value_line(path: &str, from: Option<&str>, value: &str) -> String {
    match from {
        Some(from) => format!("{path}: {from} → {value}"),
        None => format!("{path} → {value}"),
    }
}

/// What a Trigger now or Re-run does, instead of the field it sends: `Create Job
/// report-failed-rerun-… from Job report-failed`. A batch has one prefix per row, so it says only
/// that each row creates a Job.
fn created_job_line(generate_name: &str, source: Option<(&str, &str)>) -> String {
    match source {
        Some((kind, name)) => format!("Create Job {generate_name}… from {kind} {name}"),
        None => "Create one Job from each row".to_owned(),
    }
}
/// One row of the object list of a batch, and the gap between two rows. Fixed, so the height of
/// the list below is exactly `ITEMS_VISIBLE` rows.
const ITEM_ROW_HEIGHT: f32 = 23.;
const ITEM_ROW_GAP: f32 = 4.;
/// The rows of the list that are in view; more than that scroll.
const ITEMS_VISIBLE: usize = 9;
/// The object list of a batch scrolls past this height: `ITEMS_VISIBLE` rows and the gaps between.
const ITEMS_MAX_HEIGHT: f32 =
    ITEMS_VISIBLE as f32 * ITEM_ROW_HEIGHT + (ITEMS_VISIBLE - 1) as f32 * ITEM_ROW_GAP;

/// How many rows of a list of `rows` are out of view, which the `+N more` line counts.
fn hidden_rows(rows: usize) -> usize {
    rows.saturating_sub(ITEMS_VISIBLE)
}

/// Destructive and privileged actions confirm with the danger button.
fn has_danger_button(risk: ActionRisk) -> bool {
    matches!(risk, ActionRisk::Destructive | ActionRisk::Privileged)
}

/// The words around the exact text of the typed-name prompt, shared by every dialog that asks
/// for one so they read the same.
const TYPED_PROMPT_BEFORE: &str = "Type";
const TYPED_PROMPT_AFTER: &str = "to confirm";

/// `Type api to confirm`: the prompt as plain text, also the reason of the disabled button.
pub(crate) fn typed_prompt_text(expected: &str) -> String {
    format!("{TYPED_PROMPT_BEFORE} {expected} {TYPED_PROMPT_AFTER}")
}

/// The prompt with `expected` bold in the monospace face: the exact text, not the
/// kind of name it is.
pub(crate) fn typed_prompt(expected: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .gap_1()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(TYPED_PROMPT_BEFORE)
        .child(
            div()
                .font_semibold()
                .font_family(theme.mono_font_family.clone())
                .text_color(theme.foreground)
                .child(expected.to_owned()),
        )
        .child(TYPED_PROMPT_AFTER)
        .into_any_element()
}

/// The words the dialog shows for a stream start field. The audit line keeps the raw path.
fn connect_field_label(path: &str) -> &str {
    match path {
        "remote_port" => "Remote port",
        "local_port" => "Local port",
        other => other,
    }
}

/// What the dialog asks about.
pub(crate) enum DialogKind {
    /// Unlock `cluster` for changes; no object, no dry-run.
    Unlock {
        cluster: ClusterRef,
        cluster_name: SharedString,
    },
    Write(Rc<WriteIntent>),
    /// Start a stream (a shell): no object change, no dry-run.
    Connect(Rc<ConnectIntent>),
    /// Several objects of one cluster, listed in the dialog and changed one at a time.
    Batch(Rc<BatchIntent>),
}

impl DialogKind {
    fn cluster(&self) -> &ClusterRef {
        match self {
            Self::Unlock { cluster, .. } => cluster,
            Self::Write(intent) => &intent.cluster,
            Self::Connect(intent) => &intent.cluster,
            Self::Batch(intent) => &intent.cluster,
        }
    }

    fn cluster_name(&self) -> &SharedString {
        match self {
            Self::Unlock { cluster_name, .. } => cluster_name,
            Self::Write(intent) => &intent.cluster_name,
            Self::Connect(intent) => &intent.cluster_name,
            Self::Batch(intent) => &intent.cluster_name,
        }
    }

    /// What the `TypeName` tier asks to type.
    fn expected(&self) -> &str {
        match self {
            Self::Unlock { cluster_name, .. } => cluster_name,
            Self::Write(intent) => intent.expected(),
            Self::Connect(intent) => intent.expected(),
            Self::Batch(intent) => intent.expected(),
        }
    }

    fn typed_hint(&self) -> String {
        match self {
            Self::Unlock { .. } => "the cluster name".to_owned(),
            Self::Write(intent) => intent.typed_hint(),
            Self::Batch(batch) => batch.typed_hint(),
            Self::Connect(intent) => intent.typed_hint(),
        }
    }

    fn risk(&self) -> ActionRisk {
        match self {
            Self::Unlock { .. } => ActionRisk::Change,
            Self::Write(intent) => intent.risk,
            Self::Connect(intent) => intent.risk,
            Self::Batch(intent) => intent.risk,
        }
    }
}

/// Everything a dialog starts from, taken from the guard of its cluster when the action began.
pub(crate) struct DialogInputs {
    pub(crate) shell: WeakEntity<AppShell>,
    pub(crate) kind: DialogKind,
    pub(crate) confirm: DialogConfirm,
    pub(crate) environment: Environment,
    /// The session generation the dialog opened on.
    pub(crate) generation: u64,
}

/// What a batch that did not go through entirely leaves in its dialog.
struct BatchOutcome {
    /// The notice of the run, with the names of the objects that did not go through.
    notice: SharedString,
    /// The failed and unsent objects as a batch of their own, `None` when there is nothing to send
    /// again (an unknown outcome, an ordered plan).
    retry: Option<BatchIntent>,
}

pub(crate) struct ConfirmDialog {
    shell: WeakEntity<AppShell>,
    kind: DialogKind,
    confirm: DialogConfirm,
    environment: Environment,
    generation: u64,
    /// The window the dialog opened in, for closing it from its dry-run task.
    window: AnyWindowHandle,
    /// `None` for an unlock, which has nothing to check.
    dry_run: Option<DryRunState>,
    /// Where each item of a batch stands, in the order of its plan; empty for any other dialog.
    items: Vec<ItemProgress>,
    typed: Entity<InputState>,
    note: Entity<InputState>,
    /// The local port of a forward start, which the user may change before it starts.
    local_port: Option<Entity<InputState>>,
    local_port_error: Option<&'static str>,
    is_note_shown: bool,
    /// The note box was just ticked: the next render focuses the field.
    needs_note_focus: bool,
    is_committing: bool,
    /// The Stop button of a running batch was pressed; the commit loop reads it between items.
    stop_requested: Rc<Cell<bool>>,
    /// How a batch ended when something did not go through: the dialog stays as its result.
    outcome: Option<BatchOutcome>,
    /// The last failed check or commit was a 409: Retry of a taint edit then reads the node again.
    is_conflict: bool,
    /// False once the dialog is closed, by any button, Escape, or the overlay.
    is_open: bool,
    needs_focus: bool,
    focus_handle: FocusHandle,
    dry_run_task: Option<Task<()>>,
    /// `--screen cordon-confirm`: a fixed picture of the dialog that can never commit.
    #[cfg(feature = "screenshot")]
    is_fixture: bool,
    _subscriptions: Vec<Subscription>,
}

impl ConfirmDialog {
    pub(crate) fn new(inputs: DialogInputs, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The prompt above the field names the exact text; the placeholder says what kind of name it is.
        let hint = inputs.kind.typed_hint();
        let typed = cx.new(|cx| InputState::new(window, cx).placeholder(hint));
        let note = cx.new(|cx| InputState::new(window, cx).placeholder("Note"));
        let local_port = match &inputs.kind {
            DialogKind::Connect(intent) => intent.local_port().map(|choice| {
                let placeholder = format!("automatic ({})", choice.automatic);
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx).placeholder(placeholder);
                    if let LocalPortSpec::Exact(port) = choice.initial {
                        input.set_value(port.to_string(), window, cx);
                    }
                    input
                })
            }),
            DialogKind::Unlock { .. } | DialogKind::Write(_) | DialogKind::Batch(_) => None,
        };
        // The block and the match line follow the field as it is typed.
        let subscription = cx.subscribe_in(&typed, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let port_subscription = local_port.as_ref().map(|input| {
            cx.subscribe_in(input, window, |dialog, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    dialog.local_port_error = None;
                    cx.notify();
                }
            })
        });
        let items = match &inputs.kind {
            DialogKind::Batch(batch) => vec![ItemProgress::Waiting; batch.plan.items.len()],
            DialogKind::Unlock { .. } | DialogKind::Write(_) | DialogKind::Connect(_) => Vec::new(),
        };
        let dry_run = match inputs.kind {
            DialogKind::Unlock { .. } => None,
            DialogKind::Write(_) | DialogKind::Batch(_) => Some(DryRunState::Running),
            // A start that writes first checks the write; any other start has nothing to check.
            DialogKind::Connect(ref intent) if intent.create().is_some() => {
                Some(DryRunState::Running)
            }
            DialogKind::Connect(_) => Some(DryRunState::NotSupported),
        };
        Self {
            shell: inputs.shell,
            kind: inputs.kind,
            confirm: inputs.confirm,
            environment: inputs.environment,
            generation: inputs.generation,
            window: window.window_handle(),
            dry_run,
            items,
            typed,
            note,
            local_port,
            local_port_error: None,
            is_note_shown: false,
            needs_note_focus: false,
            is_committing: false,
            stop_requested: Rc::new(Cell::new(false)),
            outcome: None,
            is_conflict: false,
            is_open: true,
            needs_focus: true,
            focus_handle: cx.focus_handle(),
            dry_run_task: None,
            #[cfg(feature = "screenshot")]
            is_fixture: false,
            _subscriptions: std::iter::once(subscription)
                .chain(port_subscription)
                .collect(),
        }
    }

    /// The picture of `--screen cordon-confirm`: the dry-run has passed in 412 ms, and the dialog
    /// ignores its confirm button and Enter, so it can never send anything.
    #[cfg(feature = "screenshot")]
    pub(crate) fn show_fixture(&mut self) {
        self.show_fixture_after(Duration::from_millis(412));
    }

    /// `show_fixture` with the dry-run line reading `elapsed` (`--screen node-shell-confirm`).
    #[cfg(feature = "screenshot")]
    pub(crate) fn show_fixture_after(&mut self, elapsed: Duration) {
        // A stream start has no dry-run to pass: its line stays as it is.
        let has_dry_run = match &self.kind {
            DialogKind::Write(_) | DialogKind::Batch(_) => true,
            DialogKind::Connect(intent) => intent.create().is_some(),
            DialogKind::Unlock { .. } => false,
        };
        if has_dry_run {
            self.dry_run = Some(DryRunState::Passed { elapsed });
        }
        // A batch shows every item as checked.
        self.items = vec![ItemProgress::Passed; self.items.len()];
        self.is_fixture = true;
    }

    /// The picture of `--screen evict-confirm`: every dry-run was refused with `reason`, so the
    /// dry-run line fails and the confirm button stays off.
    #[cfg(feature = "screenshot")]
    pub(crate) fn show_fixture_refused(&mut self, reason: SharedString) {
        self.items = vec![ItemProgress::Rejected(reason); self.items.len()];
        self.dry_run = Some(summarize_dry_runs(&self.items, Duration::ZERO));
        self.is_fixture = true;
    }

    /// Opens `dialog` as the window's modal.
    pub(crate) fn open(dialog: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let view = dialog.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let closed = view.clone();
            let cancelled = view.clone();
            dialog
                .title(view.read(cx).title(cx))
                .w(px(DIALOG_WIDTH))
                .child(view.clone())
                // Escape, the overlay, and the close button must not hide a batch that keeps
                // sending: only the Stop button ends it.
                .on_cancel(move |_, _, cx| !cancelled.read(cx).is_batch_committing())
                // Runs after Escape and the overlay as well, so a late commit result knows.
                .on_close(move |_, _, cx| closed.update(cx, |dialog, _| dialog.is_open = false))
        });
    }

    fn title(&self, cx: &App) -> AnyElement {
        let name = self.kind.cluster_name();
        let text = match &self.kind {
            DialogKind::Unlock { .. } => format!("Unlock {name} for changes?"),
            DialogKind::Write(intent) => format!("{} on {name}?", intent.label),
            DialogKind::Connect(intent) => format!("{} on {name}?", intent.label),
            DialogKind::Batch(intent) => format!("{} on {name}?", intent.label),
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(environment_badge(&self.environment, cx))
            // The label names the object, which can be long: it wraps instead of running out of the
            // dialog, and it stops short of the close button in the corner.
            .child(div().flex_1().min_w_0().pr_6().child(text))
            .into_any_element()
    }

    /// Runs the server-side dry-run of the change; its result replaces the dry-run line.
    pub(crate) fn start_dry_run(&mut self, cx: &mut Context<Self>) {
        let intent = match &self.kind {
            DialogKind::Write(intent) => intent,
            DialogKind::Batch(batch) => {
                self.start_batch_dry_runs(Rc::clone(batch), cx);
                return;
            }
            DialogKind::Connect(connect) => match connect.create() {
                Some(create) => create,
                None => return,
            },
            DialogKind::Unlock { .. } => return,
        };
        let step = WriteStep {
            intent: Rc::clone(intent),
            generation: self.generation,
            mode: CommitMode::DryRun,
            note: None,
        };
        let shell = self.shell.clone();
        let window = self.window;
        let intent = Rc::clone(&step.intent);
        self.dry_run = Some(DryRunState::Running);
        // Dropping the dialog drops the task: closing it ends the dry-run.
        self.dry_run_task = Some(cx.spawn(async move |this, cx| {
            let result = checked_write(&shell, step, cx).await;
            let is_conflict = matches!(
                &result,
                Err(CheckedWriteError::Write(WriteError::Conflict { .. }))
            );
            // An edit that conflicts on the check has nothing to retry: the dialog closes and the
            // editor offers Reload and keep my changes, as for a conflict on the commit.
            if is_conflict && matches!(intent.action, ResourceAction::EditValues(_)) {
                let _ = shell.update(cx, |shell, cx| {
                    shell.values_commit_finished(&intent, &result, cx)
                });
                let _ = cx.update_window(window, |_, window, cx| {
                    let _ = this.update(cx, |dialog, cx| dialog.close(window, cx));
                });
                return;
            }
            // The same for a taint edit, which reloads the node and keeps what the node did not change:
            // the server's words about a stale `resourceVersion` are no step to read.
            if is_conflict && intent.action == ResourceAction::EditTaints {
                let _ = cx.update_window(window, |_, window, cx| {
                    let _ = this.update(cx, |dialog, cx| {
                        dialog.reload_after_conflict(window, cx);
                    });
                });
                return;
            }
            let state = dry_run_state_of(result);
            // A Secret form is still open under this dialog: the server's refusal goes back to it,
            // and the user fixes the fields there instead of retyping them.
            if let DryRunState::Refused(text) | DryRunState::Rejected(text) = &state
                && is_secret_form_action(intent.action)
                && shell
                    .update(cx, |shell, cx| shell.secret_form_refused(text.clone(), cx))
                    .unwrap_or(false)
            {
                let _ = cx.update_window(window, |_, window, cx| {
                    let _ = this.update(cx, |dialog, cx| dialog.close(window, cx));
                });
                return;
            }
            let _ = this.update(cx, |dialog, cx| {
                dialog.is_conflict = is_conflict;
                dialog.dry_run = Some(state);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The dry-runs of a batch, one item at a time in list order, none audited. The dry-run line
    /// stays `Running` until the last one answered and says `Failed` if any did not pass: a partial
    /// apply would be a surprise.
    fn start_batch_dry_runs(&mut self, batch: Rc<BatchIntent>, cx: &mut Context<Self>) {
        let shell = self.shell.clone();
        let generation = self.generation;
        self.dry_run = Some(DryRunState::Running);
        self.items = vec![ItemProgress::Waiting; batch.plan.items.len()];
        // Dropping the dialog drops the task: closing it ends the checks.
        self.dry_run_task = Some(cx.spawn(async move |this, cx| {
            let mut elapsed = Duration::ZERO;
            for (index, item) in batch.plan.items.iter().enumerate() {
                let _ = this.update(cx, |dialog, cx| {
                    dialog.set_item(index, ItemProgress::Checking, cx);
                });
                let step = WriteStep {
                    intent: Rc::new(batch.item_intent(item)),
                    generation,
                    mode: CommitMode::DryRun,
                    note: None,
                };
                let result = checked_write(&shell, step, cx).await;
                if let Ok(outcome) = &result {
                    elapsed += outcome.elapsed;
                }
                let progress = if batch.is_delete() {
                    delete_dry_run_progress(&result)
                } else {
                    dry_run_progress(&result)
                };
                let _ = this.update(cx, |dialog, cx| dialog.set_item(index, progress, cx));
            }
            let _ = this.update(cx, |dialog, cx| {
                dialog.dry_run = Some(summarize_dry_runs(&dialog.items, elapsed));
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The state of item `index` of a batch, from its dry-run to its commit.
    pub(crate) fn set_item(
        &mut self,
        index: usize,
        progress: ItemProgress,
        cx: &mut Context<Self>,
    ) {
        if let Some(slot) = self.items.get_mut(index) {
            *slot = progress;
            cx.notify();
        }
    }

    /// A commit failed in a way the user can retry. The text replaces the dry-run line as a failed
    /// check, so the confirm button stays off until Retry has checked again. `false` when the
    /// dialog is already closed.
    pub(crate) fn commit_failed(
        &mut self,
        text: String,
        is_conflict: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_open {
            return false;
        }
        self.is_committing = false;
        self.is_conflict = is_conflict;
        self.dry_run = Some(DryRunState::Failed(text.into()));
        cx.notify();
        true
    }

    /// Closes the dialog, for the buttons and for a finished commit.
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_open = false;
        window.close_dialog(cx);
    }

    pub(crate) fn is_open(&self) -> bool {
        self.is_open
    }

    fn is_batch_committing(&self) -> bool {
        self.is_committing && matches!(self.kind, DialogKind::Batch(_))
    }

    /// The commit ended with objects that did not go through: the dialog stays open as the
    /// per-object result, with Close and, when something can be sent again, Retry failed.
    pub(crate) fn show_outcome(
        &mut self,
        notice: SharedString,
        retry: Option<BatchIntent>,
        cx: &mut Context<Self>,
    ) {
        self.is_committing = false;
        self.outcome = Some(BatchOutcome { notice, retry });
        cx.notify();
    }

    /// Retry failed: a new dialog over the failed and unsent objects, which checks them again and
    /// asks the confirm of its own tier, as any batch does.
    fn retry_failed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(retry) = self
            .outcome
            .as_mut()
            .and_then(|outcome| outcome.retry.take())
        else {
            return;
        };
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        self.close(window, cx);
        // Opened after this dialog is gone, so the two never stack.
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, cx| {
                shell.start_batch(retry, window, cx);
            });
        });
    }

    /// The Stop button of a running batch: the item in flight finishes, and the rest is not sent.
    fn stop_batch(&mut self, cx: &mut Context<Self>) {
        if !self.is_batch_committing() {
            return;
        }
        self.stop_requested.set(true);
        cx.notify();
    }

    /// How many items of a delete batch turned out to be gone already.
    fn gone_count(&self) -> usize {
        self.items
            .iter()
            .filter(|state| **state == ItemProgress::Gone)
            .count()
    }

    /// The propagation radio: a change rebuilds the items and checks them all again (decision 12).
    /// Ignored while a commit runs, because the commit sends the items the user confirmed.
    fn choose_propagation(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.is_committing {
            return;
        }
        let DialogKind::Batch(batch) = &self.kind else {
            return;
        };
        let BatchExtras::Delete(extras) = &batch.plan.extras else {
            return;
        };
        let choices = propagation_choices(extras.kind, extras.targets.len() == 1);
        let Some((propagation, ..)) = choices.into_iter().nth(index) else {
            return;
        };
        if propagation == extras.propagation {
            return;
        }
        let Some(rebuilt) = with_propagation(batch, propagation, jiff::Timestamp::now()) else {
            return;
        };
        let rebuilt = Rc::new(rebuilt);
        self.kind = DialogKind::Batch(Rc::clone(&rebuilt));
        self.start_batch_dry_runs(rebuilt, cx);
    }

    /// The tier now: the one the dialog opened with, or the live one of the cluster when the user
    /// made it stricter since (Settings), whichever asks for more.
    fn live_tier(&self, cx: &App) -> DialogConfirm {
        let shell = self.shell.upgrade();
        let live = shell
            .as_ref()
            .and_then(|shell| shell.read(cx).guard_for(self.kind.cluster(), cx))
            .map(|guard| {
                confirm_step(
                    guard.profile.confirm,
                    self.kind.risk(),
                    self.kind.expected(),
                )
            });
        match live {
            Some(tier @ DialogConfirm::TypeName { .. }) => tier,
            _ => self.confirm.clone(),
        }
    }

    fn typed_match(&self, cx: &App) -> TypedMatch {
        typed_match(&self.live_tier(cx), &self.typed.read(cx).value())
    }

    /// Why the confirm button is off, `None` when it is on. The guard of the dialog's own cluster
    /// is read now, so a lock or a reconnect since the dialog opened shows at once.
    fn block(&self, cx: &App) -> Option<SharedString> {
        // A fixture is drawn from fixed data, with no cluster behind it to check.
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return None;
        }
        let shell = self.shell.upgrade();
        let guard = shell
            .as_ref()
            .and_then(|shell| shell.read(cx).guard_for(self.kind.cluster(), cx));
        let name = self.kind.cluster_name();
        let typed = self.typed_match(cx);
        // Every object went away since the dialog opened: nothing is left to send.
        if matches!(self.dry_run, Some(DryRunState::Passed { .. }))
            && !self.items.is_empty()
            && self.gone_count() == self.items.len()
        {
            return Some("Every object is already gone; nothing to delete".into());
        }
        // The gate of a start is read again too (the setting, the tier, a re-review): see `gate_block`.
        let gate_block = match (&self.kind, guard.as_ref()) {
            (DialogKind::Connect(intent), Some(guard)) => intent.gate_block(guard),
            _ => None,
        };
        let base = match &self.dry_run {
            Some(dry_run) => commit_block(
                guard.as_ref(),
                name,
                self.generation,
                dry_run,
                typed,
                self.kind.expected(),
            ),
            None => unlock_block(
                guard.as_ref(),
                name,
                self.generation,
                typed,
                self.kind.expected(),
            ),
        };
        base.or(gate_block)
    }

    /// The confirm button, or Enter on it. Nothing happens while `block` holds or a commit runs.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return;
        }
        if self.is_committing || self.outcome.is_some() || self.block(cx).is_some() {
            return;
        }
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        let generation = self.generation;
        match &self.kind {
            DialogKind::Unlock { cluster, .. } => {
                let cluster = cluster.clone();
                shell.update(cx, |shell, cx| {
                    shell.finish_unlock(&cluster, generation, window, cx);
                });
                self.close(window, cx);
            }
            DialogKind::Connect(intent) => {
                if let Some(choice) = intent.local_port() {
                    let text = self
                        .local_port
                        .as_ref()
                        .map(|input| input.read(cx).value().to_string())
                        .unwrap_or_default();
                    let Some(port) = parse_local_port_field(&text) else {
                        self.local_port_error = Some(LOCAL_PORT_FIELD_ERROR);
                        cx.notify();
                        return;
                    };
                    choice.chosen.set(Some(port));
                }
                // A start that writes first needs its passed dry-run; any other has no check.
                let proof = if intent.create().is_some() {
                    let typed = self.typed_match(cx);
                    let Some(proof) = self
                        .dry_run
                        .as_ref()
                        .and_then(|dry_run| confirmed(dry_run, typed, generation))
                    else {
                        return;
                    };
                    Some(proof)
                } else {
                    None
                };
                let commit = ConnectCommit {
                    generation,
                    confirmed: proof,
                    note: self
                        .is_note_shown
                        .then(|| self.note.read(cx).value().to_string()),
                };
                let intent = Rc::clone(intent);
                self.is_committing = true;
                shell.update(cx, |shell, cx| {
                    shell.commit_connect(&intent, commit, window, cx);
                });
                self.close(window, cx);
            }
            DialogKind::Write(intent) => {
                let typed = self.typed_match(cx);
                let Some(proof) = self
                    .dry_run
                    .as_ref()
                    .and_then(|dry_run| confirmed(dry_run, typed, generation))
                else {
                    return;
                };
                let note = self
                    .is_note_shown
                    .then(|| self.note.read(cx).value().to_string());
                let step = WriteStep {
                    intent: Rc::clone(intent),
                    generation,
                    mode: CommitMode::Commit { confirmed: proof },
                    note,
                };
                let dialog = cx.weak_entity();
                self.is_committing = true;
                shell.update(cx, |shell, cx| shell.commit_write(dialog, step, window, cx));
                cx.notify();
            }
            DialogKind::Batch(batch) => {
                let typed = self.typed_match(cx);
                let Some(proof) = self
                    .dry_run
                    .as_ref()
                    .and_then(|dry_run| confirmed(dry_run, typed, generation))
                else {
                    return;
                };
                let commit = BatchCommit {
                    proof,
                    generation,
                    note: self
                        .is_note_shown
                        .then(|| self.note.read(cx).value().to_string()),
                    gone: self
                        .items
                        .iter()
                        .map(|state| *state == ItemProgress::Gone)
                        .collect(),
                    stop: Rc::clone(&self.stop_requested),
                };
                let batch = Rc::clone(batch);
                let dialog = cx.weak_entity();
                self.is_committing = true;
                shell.update(cx, |shell, cx| {
                    shell.commit_batch(dialog, batch, commit, window, cx);
                });
                cx.notify();
            }
        }
    }

    /// Enter confirms once, from the dialog or the typed-name field, and only a fresh press: a held
    /// Enter (the one that opened the dialog from a menu) repeats with `is_held` and is ignored.
    /// A focused button keeps its own Enter, so Back stays Back.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.key != "enter" || key.modifiers.modified() {
            return;
        }
        let typed_focus = self.typed.read(cx).focus_handle(cx);
        let is_port_focused = self
            .local_port
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
        if !(self.focus_handle.is_focused(window)
            || typed_focus.is_focused(window)
            || is_port_focused)
        {
            return;
        }
        // The kit also clicks a focused element on the Enter key-up unless the press was handled.
        window.prevent_default();
        cx.stop_propagation();
        if !event.is_held {
            self.confirm(window, cx);
        }
    }

    /// A 409 on a taint edit: closes the dialog and reopens the taint editor on the node as it is
    /// now, `false` when the dialog is already closed.
    pub(crate) fn reload_after_conflict(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_open {
            return false;
        }
        self.is_conflict = true;
        self.retry(window, cx);
        true
    }

    /// Retry runs the dry-run again. The taint editor after a 409 is the exception (it comes here
    /// from `reload_after_conflict`, never from the button): its change carries the
    /// `resourceVersion` it was read at, which cannot pass a second time, so the node is read again
    /// and the editor reopens with the rows the node did not change taken from it, the user's own
    /// edits kept, and a notice of what changed on the node.
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reopen = match &self.kind {
            DialogKind::Write(intent)
                if intent.action == ResourceAction::EditTaints && self.is_conflict =>
            {
                taint_rows_of_request(intent.request.operation()).map(|rows| {
                    (
                        intent.cluster.clone(),
                        intent.request.target().name().to_owned(),
                        rows,
                    )
                })
            }
            _ => None,
        };
        let (Some((cluster, node, rows)), Some(shell)) = (reopen, self.shell.upgrade()) else {
            self.start_dry_run(cx);
            return;
        };
        self.close(window, cx);
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, cx| {
                shell.reopen_taint_editor(&cluster, &node, rows, window, cx);
            });
        });
    }

    /// A check that failed can be run again (a refusal for now, a transient error); a webhook that
    /// does not support dry-run cannot.
    fn can_retry(&self) -> bool {
        matches!(self.dry_run, Some(DryRunState::Failed(_)))
    }

    /// The objects of a batch, one line each with its state, and under them the ones the batch
    /// leaves alone with their reasons. Scrolls past a few lines.
    fn render_items(&self, batch: &BatchIntent, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let mono = theme.mono_font_family.clone();
        let tone = |progress: &ItemProgress| match progress {
            ItemProgress::Passed | ItemProgress::Done => tone_color(StatusTone::Ok, cx),
            ItemProgress::Rejected(_) | ItemProgress::Failed(_) | ItemProgress::Unknown => {
                tone_color(StatusTone::Bad, cx)
            }
            ItemProgress::Pending(_) => tone_color(StatusTone::Warn, cx),
            ItemProgress::Waiting
            | ItemProgress::Checking
            | ItemProgress::Applying
            | ItemProgress::NotSent(_)
            | ItemProgress::Gone => theme.muted_foreground,
        };
        let is_single = batch.plan.items.len() == 1;
        let loose = pods_without_controller(batch);
        let rows = batch
            .plan
            .items
            .iter()
            .zip(&self.items)
            .map(|(item, progress)| {
                // A lone object's refusal is read in the dry-run line under the list, not cut here.
                let state_text = match progress {
                    ItemProgress::Rejected(_) if is_single => "failed".to_owned(),
                    ItemProgress::Done if batch.is_restart() => "requested".to_owned(),
                    other => other.text(),
                };
                let is_loose = loose.contains(&item.object);
                h_flex()
                    .h(px(ITEM_ROW_HEIGHT))
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_family(mono.clone())
                                    .child(item.object.clone()),
                            )
                            .children(is_loose.then(|| {
                                div()
                                    .text_xs()
                                    .text_color(tone_color(StatusTone::Warn, cx))
                                    .child("no controller")
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(tone(progress))
                            .child(state_text),
                    )
            });
        let skipped = batch.plan.skipped.iter().map(|skip| {
            h_flex()
                .h(px(ITEM_ROW_HEIGHT))
                .gap_2()
                .items_center()
                .justify_between()
                .text_color(theme.muted_foreground)
                .child(
                    div()
                        .text_sm()
                        .font_family(mono.clone())
                        .child(skip.object.clone()),
                )
                .child(div().text_xs().child(format!("skipped: {}", skip.reason)))
        });
        let gone = match &batch.plan.extras {
            BatchExtras::Delete(extras) if !extras.already_gone.is_empty() => {
                let names: Vec<&str> = extras.already_gone.iter().map(AsRef::as_ref).collect();
                Some(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("already gone: {}", names.join(", "))),
                )
            }
            BatchExtras::Delete(_)
            | BatchExtras::None
            | BatchExtras::DefaultClass(_)
            | BatchExtras::Labels(_) => None,
        };
        let list = v_flex()
            .id("batch-items")
            .gap(px(ITEM_ROW_GAP))
            .max_h(px(ITEMS_MAX_HEIGHT))
            .overflow_y_scroll()
            .children(rows)
            .children(skipped)
            .children(gone);
        // The list scrolls inside the dialog; without a hint the rows below the fold look absent.
        let hidden = hidden_rows(batch.plan.items.len() + batch.plan.skipped.len());
        let more = (hidden > 0).then(|| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("+{hidden} more · scroll the list"))
        });
        v_flex()
            .gap_1()
            .child(list)
            .children(more)
            .into_any_element()
    }

    fn render_object(&self, cx: &App) -> Option<AnyElement> {
        if let DialogKind::Connect(intent) = &self.kind {
            return Some(Self::render_connect_object(intent, cx));
        }
        let intent = match &self.kind {
            DialogKind::Write(intent) => intent,
            DialogKind::Batch(batch) => return Some(self.render_items(batch, cx)),
            DialogKind::Unlock { .. } | DialogKind::Connect(_) => return None,
        };
        let theme = cx.theme();
        let target = intent.request.target();
        let name = match target.namespace() {
            Some(namespace) => format!("{namespace}/{}", target.name()),
            None => target.name().to_owned(),
        };
        // A create changes no existing object: its fields are listed below, so the row says `new`.
        // Trigger now and Re-run leave the row they act on alone and create a Job.
        let count_text = if matches!(intent.action, ResourceAction::CreateObject(_)) {
            "new".to_owned()
        } else if matches!(
            intent.action,
            ResourceAction::TriggerCronJob | ResourceAction::RerunJob
        ) {
            "creates a Job".to_owned()
        } else {
            let count = intent.request.changed_fields().len();
            let unit = if count == 1 { "field" } else { "fields" };
            format!("{count} {unit} changed")
        };
        Some(
            h_flex()
                .gap_2()
                .items_center()
                .flex_wrap()
                .child(div().text_sm().child(target.kind_name().to_owned()))
                .child(
                    div()
                        .text_sm()
                        .font_family(theme.mono_font_family.clone())
                        .child(name),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(count_text),
                )
                .into_any_element(),
        )
    }

    /// The target of a stream start: its kind and name.
    fn render_connect_object(intent: &ConnectIntent, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let object = &intent.object;
        let name = match &object.namespace {
            Some(namespace) => format!("{namespace}/{}", object.name),
            None => object.name.clone(),
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(div().text_sm().child(object.kind.clone()))
            .child(
                div()
                    .text_sm()
                    .font_family(theme.mono_font_family.clone())
                    .child(name),
            )
            .into_any_element()
    }

    fn render_changes(&self, cx: &App) -> Option<Div> {
        if let DialogKind::Connect(intent) = &self.kind {
            let theme = cx.theme();
            let mono = theme.mono_font_family.clone();
            // The field below replaces the fixed line of the local port.
            let has_port_field = self.local_port.is_some();
            let lines = intent
                .fields
                .iter()
                .filter(|field| !(has_port_field && field.path == "local_port"))
                .map(|field| {
                    let text = match &field.value {
                        Some(value) => format!("{} → {value}", connect_field_label(&field.path)),
                        None => connect_field_label(&field.path).to_owned(),
                    };
                    div().text_sm().font_family(mono.clone()).child(text)
                });
            let port_field = self.local_port.as_ref().map(|input| {
                let error = self
                    .local_port_error
                    .map(|text| div().text_xs().text_color(theme.danger).child(text));
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Local port (empty = automatic)"),
                    )
                    .child(Input::new(input).small())
                    .children(error)
            });
            let warnings = intent.warnings.iter().map(|warning| {
                div()
                    .text_sm()
                    .text_color(tone_color(StatusTone::Warn, cx))
                    .child(counted_text(warning.clone()))
            });
            return Some(
                v_flex()
                    .gap_1()
                    .children(lines)
                    .children(port_field)
                    .children(warnings),
            );
        }
        // A batch changes the same field of every item, so it is shown once. An ordered plan
        // (Set default) changes different fields per item, so every item's are shown.
        let (requests, warnings) = match &self.kind {
            DialogKind::Write(intent) => (vec![&intent.request], &intent.warnings),
            DialogKind::Batch(batch) if batch.plan.on_failure == BatchFailure::Stop => (
                batch.plan.items.iter().map(|item| &item.request).collect(),
                &batch.warnings,
            ),
            DialogKind::Batch(batch) => (vec![&batch.plan.items.first()?.request], &batch.warnings),
            DialogKind::Unlock { .. } | DialogKind::Connect(_) => return None,
        };
        let theme = cx.theme();
        let mono = theme.mono_font_family.clone();
        // The value of a path is `None` when it is not recorded, or in an ordered plan when the
        // key is removed (the beta default annotation).
        let is_ordered = matches!(&self.kind, DialogKind::Batch(batch) if batch.plan.on_failure == BatchFailure::Stop);
        let is_batch = matches!(self.kind, DialogKind::Batch(_));
        // An intent that names its own lines (Set image, Edit YAML) replaces the field list.
        let own_lines = match &self.kind {
            DialogKind::Write(intent) => intent.change_lines.as_slice(),
            _ => &[],
        };
        let lines = requests
            .into_iter()
            .filter(|_| own_lines.is_empty())
            .flat_map(|request| match request.operation() {
                // The values edit lists the ConfigMap text before and after; the audit line never has it.
                WriteOperation::SetDataValues(edit) => edit.confirm_lines(),
                _ => {
                    let target = request.target();
                    let source = (!is_batch).then(|| (target.kind_name(), target.name()));
                    request
                        .changed_fields()
                        .into_iter()
                        .map(|field| match field.value {
                            Some(value) if field.path == EVICTION_PATH => eviction_line(&value),
                            Some(value) if field.path == GENERATE_NAME_PATH => {
                                created_job_line(&value, source)
                            }
                            Some(value) => value_line(&field.path, field.from.as_deref(), &value),
                            None if is_ordered => format!("{} → removed", field.path),
                            None => field.path.into_owned(),
                        })
                        .collect()
                }
            })
            .chain(own_lines.iter().map(ToString::to_string))
            .map(|text| div().text_sm().font_family(mono.clone()).child(text));
        let warnings = warnings.iter().map(|warning| {
            div()
                .text_sm()
                .text_color(tone_color(StatusTone::Warn, cx))
                .child(counted_text(warning.clone()))
        });
        Some(v_flex().gap_1().children(lines).children(warnings))
    }

    /// What a passed dry-run adds: the replace of an edit carries the base `resourceVersion`, so a
    /// pass also says nothing changed on the server since the editor opened.
    fn passed_note(&self) -> &'static str {
        match &self.kind {
            DialogKind::Write(intent) if matches!(intent.action, ResourceAction::EditYaml(_)) => {
                " · unchanged since you opened it"
            }
            _ => "",
        }
    }

    /// What the dialog says when no audit folder is set. A stream start is not a change.
    fn unlogged_note(&self) -> &'static str {
        match self.kind {
            DialogKind::Connect(_) => "Audit file unavailable: this action won't be logged",
            _ => "Audit file unavailable: this change won't be logged",
        }
    }

    fn render_dry_run(&self, cx: &App) -> Option<AnyElement> {
        // The result of the run replaces the dry-run line, which is about a past check.
        if let Some(outcome) = &self.outcome {
            return Some(
                div()
                    .text_sm()
                    .text_color(tone_color(StatusTone::Bad, cx))
                    .child(outcome.notice.clone())
                    .into_any_element(),
            );
        }
        // A stream start has nothing to dry-run, so it does not say the check is missing.
        if matches!(
            (&self.kind, self.dry_run.as_ref()),
            (DialogKind::Connect(_), Some(DryRunState::NotSupported))
        ) {
            return None;
        }
        let theme = cx.theme();
        let total = self.items.len();
        let is_batch = matches!(self.kind, DialogKind::Batch(_));
        let (text, color): (String, _) = match self.dry_run.as_ref()? {
            DryRunState::NotSupported => (
                "Dry-run not supported for this action".to_owned(),
                theme.muted_foreground,
            ),
            DryRunState::Running if is_batch => {
                let checked = self
                    .items
                    .iter()
                    .filter(|state| {
                        !matches!(state, ItemProgress::Waiting | ItemProgress::Checking)
                    })
                    .count();
                (
                    format!("Server dry-run… {checked} of {total}"),
                    theme.muted_foreground,
                )
            }
            DryRunState::Running => ("Server dry-run…".to_owned(), theme.muted_foreground),
            DryRunState::Passed { .. } if is_batch => {
                let passed = total - self.gone_count();
                (
                    format!("Server dry-run passed for {passed} of {total}"),
                    tone_color(StatusTone::Ok, cx),
                )
            }
            DryRunState::Passed { elapsed } => (
                format!(
                    "Server dry-run passed · {} ms{}",
                    elapsed.as_millis(),
                    self.passed_note()
                ),
                tone_color(StatusTone::Ok, cx),
            ),
            DryRunState::Failed(text) | DryRunState::Refused(text) => {
                (text.to_string(), tone_color(StatusTone::Bad, cx))
            }
            DryRunState::Rejected(reason) => (
                format!("An admission webhook does not support dry-run: {reason}"),
                tone_color(StatusTone::Bad, cx),
            ),
        };
        Some(
            div()
                .text_sm()
                .text_color(color)
                .child(counted_text(text))
                .into_any_element(),
        )
    }

    /// The Dependents choice of a delete of an owner kind (Deployment, Job, …), under the list.
    fn render_propagation(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let DialogKind::Batch(batch) = &self.kind else {
            return None;
        };
        let BatchExtras::Delete(extras) = &batch.plan.extras else {
            return None;
        };
        if self.outcome.is_some() || !has_dependents(extras) {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let choices = propagation_choices(extras.kind, extras.targets.len() == 1);
        let selected = choices
            .iter()
            .position(|(propagation, ..)| *propagation == extras.propagation);
        let group = RadioGroup::vertical("dependents")
            .selected_index(selected)
            .disabled(self.is_committing)
            .on_change(cx.listener(|dialog, index: &usize, _, cx| {
                dialog.choose_propagation(*index, cx);
            }))
            .children(choices.into_iter().map(|(_, label, consequence)| {
                Radio::new(label)
                    .label(label)
                    .child(div().text_xs().text_color(muted).child(consequence))
            }));
        Some(
            v_flex()
                .gap_1()
                .child(div().text_sm().text_color(muted).child("Dependents"))
                .child(group)
                .into_any_element(),
        )
    }

    fn render_typed(&self, cx: &App) -> Option<AnyElement> {
        if self.outcome.is_some() || matches!(self.live_tier(cx), DialogConfirm::Click) {
            return None;
        }
        let matches = self.typed_match(cx) == TypedMatch::Matches;
        Some(
            v_flex()
                .gap_1()
                .child(typed_prompt(self.kind.expected(), cx))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().child(Input::new(&self.typed)))
                        .children(matches.then(|| {
                            div()
                                .text_sm()
                                .text_color(tone_color(StatusTone::Ok, cx))
                                .child("matches")
                        })),
                )
                .into_any_element(),
        )
    }

    /// The note checkbox: ticking it shows the field and moves the focus into it.
    fn show_note(&mut self, is_shown: bool, cx: &mut Context<Self>) {
        self.is_note_shown = is_shown;
        self.needs_note_focus = is_shown;
        cx.notify();
    }

    /// Where the line goes when it is not saved.
    fn render_unlogged_note(&self, cx: &App) -> Option<AnyElement> {
        if self.outcome.is_some() || matches!(self.kind, DialogKind::Unlock { .. }) {
            return None;
        }
        AppSettings::config_dir(cx).is_none().then(|| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(self.unlogged_note())
                .into_any_element()
        })
    }

    /// The note field once the checkbox is ticked. It sits below the button row so ticking the
    /// box does not move the buttons.
    fn render_note_input(&self) -> Option<AnyElement> {
        if self.outcome.is_some() || matches!(self.kind, DialogKind::Unlock { .. }) {
            return None;
        }
        self.is_note_shown
            .then(|| Input::new(&self.note).into_any_element())
    }

    /// The button row: the audit note checkbox on the left (a change only), Retry, Back, and the
    /// confirm button on the right.
    fn render_buttons(&self, block: bool, cx: &mut Context<Self>) -> AnyElement {
        if let Some(outcome) = &self.outcome {
            let retry = outcome.retry.as_ref().map(|_| {
                Button::new("write-retry-failed")
                    .label("Retry failed")
                    .small()
                    .primary()
                    .on_click(cx.listener(|dialog, _, window, cx| dialog.retry_failed(window, cx)))
            });
            let close = Button::new("write-close")
                .label("Close")
                .small()
                .outline()
                .on_click(cx.listener(|dialog, _, window, cx| dialog.close(window, cx)));
            return h_flex()
                .w_full()
                .gap_2()
                .justify_end()
                .child(close)
                .children(retry)
                .into_any_element();
        }
        let (label, is_danger) = match &self.kind {
            DialogKind::Unlock { .. } => (SharedString::from("Unlock"), false),
            DialogKind::Write(intent) => (intent.button.clone(), has_danger_button(intent.risk)),
            DialogKind::Connect(intent) => (intent.button.clone(), has_danger_button(intent.risk)),
            DialogKind::Batch(batch) => (
                batch.confirm_label(self.gone_count()).into(),
                has_danger_button(batch.risk),
            ),
        };
        let primary = Button::new("write-confirm")
            .label(label)
            .small()
            .disabled(block || self.is_committing)
            .on_click(cx.listener(|dialog, _, window, cx| dialog.confirm(window, cx)));
        let primary = if is_danger {
            primary.danger()
        } else {
            primary.primary()
        };
        // While a batch sends, Back would only hide it: the one way out is to stop the rest.
        let back = if self.is_batch_committing() {
            let is_stopping = self.stop_requested.get();
            Button::new("write-stop")
                .label(match is_stopping {
                    true => "Stopping…",
                    false => "Stop after current item",
                })
                .small()
                .outline()
                .disabled(is_stopping)
                .on_click(cx.listener(|dialog, _, _, cx| dialog.stop_batch(cx)))
        } else {
            Button::new("write-back")
                .label("Back")
                .small()
                .outline()
                .on_click(cx.listener(|dialog, _, window, cx| dialog.close(window, cx)))
        };
        let retry = self.can_retry().then(|| {
            Button::new("write-retry")
                .label("Retry")
                .small()
                .outline()
                .on_click(cx.listener(|dialog, _, window, cx| dialog.retry(window, cx)))
        });
        // Without an audit folder there is no line to add a note to.
        let has_audit_file = AppSettings::config_dir(cx).is_some();
        let note = (has_audit_file && !matches!(self.kind, DialogKind::Unlock { .. })).then(|| {
            Checkbox::new("write-note")
                .label("Add a note to the audit log")
                .checked(self.is_note_shown)
                .on_click(cx.listener(|dialog, checked: &bool, _, cx| {
                    dialog.show_note(*checked, cx);
                }))
        });
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .justify_between()
            .child(h_flex().children(note))
            .child(h_flex().gap_2().children(retry).child(back).child(primary))
            .into_any_element()
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_note_focus {
            self.needs_note_focus = false;
            self.note.update(cx, |input, cx| input.focus(window, cx));
        }
        if self.needs_focus {
            self.needs_focus = false;
            match self.live_tier(cx) {
                DialogConfirm::TypeName { .. } => {
                    self.typed.update(cx, |input, cx| input.focus(window, cx))
                }
                DialogConfirm::Click => window.focus(&self.focus_handle, cx),
            }
        }
        let block = self.block(cx);
        let muted = cx.theme().muted_foreground;
        // The block text explains a disabled button when the dry-run line does not: after a pass, it
        // is the lock, a reconnect, or the name. The typed-name hint above the field already says
        // what is missing, so it is not repeated.
        let typed_reason = typed_prompt_text(self.kind.expected());
        let block_text = block
            .as_ref()
            .filter(|reason| reason.as_ref() != typed_reason)
            .filter(|_| matches!(self.dry_run, None | Some(DryRunState::Passed { .. })))
            .filter(|_| self.outcome.is_none())
            .cloned();
        let unlock_note = matches!(self.kind, DialogKind::Unlock { .. }).then(|| {
            div().text_sm().text_color(muted).child(
                "Changes are offered again in this session. Locking is immediate and needs no confirmation.",
            )
        });
        v_flex()
            .key_context("WriteConfirm")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .children(self.render_object(cx))
            .children(self.render_changes(cx))
            .children(self.render_propagation(cx))
            .children(unlock_note)
            .children(self.render_dry_run(cx))
            .children(self.render_typed(cx))
            .children(self.render_unlogged_note(cx))
            .children(block_text.map(|text| {
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(with_next_step(&text))
            }))
            .child(self.render_buttons(block.is_some(), cx))
            .children(self.render_note_input())
    }
}

/// What the shell tests read from, and do to, an open dialog.
#[cfg(test)]
impl ConfirmDialog {
    pub(crate) fn dry_run_state(&self) -> Option<DryRunState> {
        self.dry_run.clone()
    }

    /// The start the dialog asks about, for the tests that call the commit directly.
    pub(crate) fn connect_intent(&self) -> Option<Rc<ConnectIntent>> {
        match &self.kind {
            DialogKind::Connect(intent) => Some(Rc::clone(intent)),
            _ => None,
        }
    }

    pub(crate) fn tier(&self) -> &DialogConfirm {
        &self.confirm
    }

    pub(crate) fn environment(&self) -> &Environment {
        &self.environment
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn warning_lines(&self) -> Vec<SharedString> {
        match &self.kind {
            DialogKind::Write(intent) => intent.warnings.clone(),
            DialogKind::Connect(intent) => intent.warnings.clone(),
            DialogKind::Batch(batch) => batch.warnings.clone(),
            DialogKind::Unlock { .. } => Vec::new(),
        }
    }

    /// The lines the dialog lists as the change, when the intent names its own.
    pub(crate) fn change_lines(&self) -> Vec<SharedString> {
        match &self.kind {
            DialogKind::Write(intent) => intent.change_lines.clone(),
            DialogKind::Batch(_) | DialogKind::Connect(_) | DialogKind::Unlock { .. } => Vec::new(),
        }
    }

    /// What the change is called in the title of the dialog.
    pub(crate) fn label(&self) -> Option<SharedString> {
        match &self.kind {
            DialogKind::Write(intent) => Some(intent.label.clone()),
            DialogKind::Batch(batch) => Some(batch.label.clone()),
            DialogKind::Connect(intent) => Some(intent.label.clone()),
            DialogKind::Unlock { .. } => None,
        }
    }

    /// Where each item of a batch stands.
    pub(crate) fn item_states(&self) -> Vec<ItemProgress> {
        self.items.clone()
    }

    /// The text of the confirm button.
    pub(crate) fn confirm_text(&self) -> Option<String> {
        match &self.kind {
            DialogKind::Write(intent) => Some(intent.button.to_string()),
            DialogKind::Batch(batch) => Some(batch.confirm_label(self.gone_count())),
            DialogKind::Connect(intent) => Some(intent.button.to_string()),
            DialogKind::Unlock { .. } => None,
        }
    }

    /// What a passed dry-run adds to its line.
    pub(crate) fn dry_run_note(&self) -> &'static str {
        self.passed_note()
    }

    pub(crate) fn block_reason(&self, cx: &App) -> Option<SharedString> {
        self.block(cx)
    }

    pub(crate) fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.typed
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// Ticks the note checkbox.
    pub(crate) fn tick_note(&mut self, cx: &mut Context<Self>) {
        self.show_note(true, cx);
    }

    /// Whether the text cursor is in the note field.
    pub(crate) fn is_note_focused(&self, window: &Window, cx: &App) -> bool {
        self.note.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Types into the local port field of a forward confirm.
    pub(crate) fn type_local_port(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = &self.local_port {
            input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
    }

    /// The text under the local port field.
    pub(crate) fn local_port_error(&self) -> Option<&'static str> {
        self.local_port_error
    }

    pub(crate) fn press_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx);
    }

    pub(crate) fn press_retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.retry(window, cx);
    }

    pub(crate) fn press_stop(&mut self, cx: &mut Context<Self>) {
        self.stop_batch(cx);
    }

    /// The notice a finished batch left in the dialog, `None` while the dialog is a question.
    pub(crate) fn outcome_notice(&self) -> Option<SharedString> {
        self.outcome.as_ref().map(|outcome| outcome.notice.clone())
    }

    /// The objects Retry failed would send, `None` when the button is not offered.
    pub(crate) fn retry_labels(&self) -> Option<Vec<SharedString>> {
        let retry = self.outcome.as_ref()?.retry.as_ref()?;
        Some(
            retry
                .plan
                .items
                .iter()
                .map(|item| item.object.clone())
                .collect(),
        )
    }

    pub(crate) fn press_retry_failed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.retry_failed(window, cx);
    }

    pub(crate) fn is_stop_offered(&self) -> bool {
        self.is_batch_committing() && !self.stop_requested.get()
    }

    pub(crate) fn choose_propagation_for_test(&mut self, index: usize, cx: &mut Context<Self>) {
        self.choose_propagation(index, cx);
    }
}

#[cfg(test)]
mod eviction_line_tests {
    use super::eviction_line;

    #[test]
    fn an_eviction_reads_as_a_sentence_not_a_path() {
        assert_eq!(
            eviction_line("grace pod default"),
            "Eviction request · pod's own grace period"
        );
        assert_eq!(
            eviction_line("grace 30s"),
            "Eviction request · grace period 30s"
        );
    }
}

#[cfg(test)]
mod typed_prompt_tests {
    use super::typed_prompt_text;

    #[test]
    fn the_prompt_names_the_exact_text() {
        assert_eq!(typed_prompt_text("api"), "Type api to confirm");
        assert_eq!(
            typed_prompt_text("uat-monitor"),
            "Type uat-monitor to confirm"
        );
    }
}

#[cfg(test)]
mod item_list_tests {
    use super::{ITEM_ROW_GAP, ITEM_ROW_HEIGHT, ITEMS_MAX_HEIGHT, ITEMS_VISIBLE, hidden_rows};

    #[test]
    fn the_more_line_counts_the_rows_that_are_out_of_view() {
        assert_eq!(ITEMS_VISIBLE, 9);
        assert_eq!(hidden_rows(12), 3);
        assert_eq!(hidden_rows(10), 1);
        assert_eq!(hidden_rows(ITEMS_VISIBLE), 0);
        assert_eq!(hidden_rows(2), 0);
    }

    #[test]
    fn the_list_is_exactly_as_tall_as_the_rows_in_view() {
        let rows = ITEMS_VISIBLE as f32;
        assert_eq!(
            ITEMS_MAX_HEIGHT,
            rows * ITEM_ROW_HEIGHT + (rows - 1.) * ITEM_ROW_GAP
        );
    }
}

#[cfg(test)]
mod connect_field_tests {
    use super::connect_field_label;

    #[test]
    fn a_forward_confirm_words_its_port_fields() {
        assert_eq!(connect_field_label("remote_port"), "Remote port");
        assert_eq!(connect_field_label("local_port"), "Local port");
        // Any other field keeps the path the audit line records.
        assert_eq!(connect_field_label("container"), "container");
    }
}

#[cfg(test)]
mod created_job_line_tests {
    use super::created_job_line;

    #[test]
    fn a_rerun_names_the_job_it_creates_and_the_job_it_copies() {
        assert_eq!(
            created_job_line("report-failed-rerun-", Some(("Job", "report-failed"))),
            "Create Job report-failed-rerun-… from Job report-failed"
        );
    }

    #[test]
    fn a_trigger_names_the_cronjob_it_runs() {
        assert_eq!(
            created_job_line("heartbeat-manual-", Some(("CronJob", "heartbeat"))),
            "Create Job heartbeat-manual-… from CronJob heartbeat"
        );
    }

    #[test]
    fn a_batch_has_no_single_source() {
        assert_eq!(
            created_job_line("a-rerun-", None),
            "Create one Job from each row"
        );
    }
}

#[cfg(test)]
mod value_line_tests {
    use super::value_line;

    #[test]
    fn a_field_with_its_old_value_reads_old_to_new() {
        assert_eq!(
            value_line("spec.maxReplicas", Some("1"), "3"),
            "spec.maxReplicas: 1 → 3"
        );
    }

    #[test]
    fn a_field_without_one_keeps_the_path_and_the_new_value() {
        assert_eq!(
            value_line("spec.paused", None, "true"),
            "spec.paused → true"
        );
    }
}
