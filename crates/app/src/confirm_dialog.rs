//! The confirm dialog (spec 0030): one small modal for every guarded action. The write variant
//! shows the object, the changes, the server-side dry-run, and the typed-name field of the
//! `TypeName` tier; the unlock variant shows the same without an object or a dry-run.
//!
//! It never sends anything itself: its buttons call back into the shell (`commit_write`,
//! `finish_unlock`), which re-checks the gate and the lock of the cluster the dialog names.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};

use crate::app_shell::AppShell;
use crate::app_shell::batch_write::{
    BatchCommit, BatchExtras, BatchFailure, BatchIntent, ItemProgress, dry_run_progress,
    summarize_dry_runs,
};
use crate::app_shell::node_editor::{CHANGED_NOTICE, NodeEditKind};
use crate::app_shell::object_delete::{
    delete_dry_run_progress, propagation_choices, with_propagation,
};
use crate::app_shell::write_flow::{
    CommitMode, ConnectCommit, ConnectIntent, DryRunState, TypedMatch, WriteIntent, WriteStep,
    checked_write, commit_block, confirmed, dry_run_state_of, typed_match, unlock_block,
};
use crate::cluster_registry::ClusterRef;
use crate::environment::{Environment, environment_badge};
use crate::resource_actions::ResourceAction;
use crate::settings::AppSettings;
use crate::write_guard::{ActionRisk, DialogConfirm, confirm_step};

const DIALOG_WIDTH: f32 = 480.;
/// The object list of a batch scrolls past this height.
const ITEMS_MAX_HEIGHT: f32 = 240.;

/// Destructive and privileged actions confirm with the danger button.
fn has_danger_button(risk: ActionRisk) -> bool {
    matches!(risk, ActionRisk::Destructive | ActionRisk::Privileged)
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
            Self::Connect(intent) => intent.typed_hint().to_owned(),
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

pub(crate) struct ConfirmDialog {
    shell: WeakEntity<AppShell>,
    kind: DialogKind,
    confirm: DialogConfirm,
    environment: Environment,
    generation: u64,
    /// `None` for an unlock, which has nothing to check.
    dry_run: Option<DryRunState>,
    /// Where each item of a batch stands, in the order of its plan; empty for any other dialog.
    items: Vec<ItemProgress>,
    typed: Entity<InputState>,
    note: Entity<InputState>,
    is_note_shown: bool,
    is_committing: bool,
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
        let typed = cx.new(|cx| InputState::new(window, cx).placeholder("Type here"));
        let note = cx.new(|cx| InputState::new(window, cx).placeholder("Note"));
        // The block and the match line follow the field as it is typed.
        let subscription = cx.subscribe_in(&typed, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
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
            dry_run,
            items,
            typed,
            note,
            is_note_shown: false,
            is_committing: false,
            is_open: true,
            needs_focus: true,
            focus_handle: cx.focus_handle(),
            dry_run_task: None,
            #[cfg(feature = "screenshot")]
            is_fixture: false,
            _subscriptions: vec![subscription],
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

    /// Opens `dialog` as the window's modal.
    pub(crate) fn open(dialog: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let view = dialog.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let closed = view.clone();
            dialog
                .title(view.read(cx).title(cx))
                .w(px(DIALOG_WIDTH))
                .child(view.clone())
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
            .child(environment_badge(self.environment, cx))
            // The label names the object, which can be long: it wraps instead of running out of the
            // dialog.
            .child(div().flex_1().min_w_0().child(text))
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
        self.dry_run = Some(DryRunState::Running);
        // Dropping the dialog drops the task: closing it ends the dry-run.
        self.dry_run_task = Some(cx.spawn(async move |this, cx| {
            let result = checked_write(&shell, step, cx).await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.dry_run = Some(dry_run_state_of(result));
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
    pub(crate) fn commit_failed(&mut self, text: String, cx: &mut Context<Self>) -> bool {
        if !self.is_open {
            return false;
        }
        self.is_committing = false;
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
        if self.is_committing || self.block(cx).is_some() {
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
        if !(self.focus_handle.is_focused(window) || typed_focus.is_focused(window)) {
            return;
        }
        // The kit also clicks a focused element on the Enter key-up unless the press was handled.
        window.prevent_default();
        cx.stop_propagation();
        if !event.is_held {
            self.confirm(window, cx);
        }
    }

    /// Retry runs the dry-run again. The taint editor is the exception: its change carries the
    /// `resourceVersion` it was read at, which cannot pass a second time, so Retry reads the node
    /// again and reopens the editor fresh (the old rows could resurrect a removed taint).
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reopen = match &self.kind {
            DialogKind::Write(intent) if intent.action == ResourceAction::EditTaints => Some((
                intent.cluster.clone(),
                intent.request.target().name().to_owned(),
            )),
            _ => None,
        };
        let (Some((cluster, node)), Some(shell)) = (reopen, self.shell.upgrade()) else {
            self.start_dry_run(cx);
            return;
        };
        self.close(window, cx);
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_node_editor(
                    NodeEditKind::Taints,
                    &cluster,
                    &node,
                    Some(CHANGED_NOTICE.into()),
                    window,
                    cx,
                );
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
            ItemProgress::Passed | ItemProgress::Done => theme.success,
            ItemProgress::Rejected(_) | ItemProgress::Failed(_) | ItemProgress::Unknown => {
                theme.danger
            }
            ItemProgress::Pending(_) => theme.warning,
            ItemProgress::Waiting
            | ItemProgress::Checking
            | ItemProgress::Applying
            | ItemProgress::NotSent(_)
            | ItemProgress::Gone => theme.muted_foreground,
        };
        let rows = batch
            .plan
            .items
            .iter()
            .zip(&self.items)
            .map(|(item, progress)| {
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_family(mono.clone())
                            .child(item.object.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(tone(progress))
                            .child(progress.text()),
                    )
            });
        let skipped = batch.plan.skipped.iter().map(|skip| {
            h_flex()
                .gap_2()
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
            BatchExtras::Delete(_) | BatchExtras::None | BatchExtras::DefaultClass(_) => None,
        };
        v_flex()
            .id("batch-items")
            .gap_1()
            .max_h(px(ITEMS_MAX_HEIGHT))
            .overflow_y_scroll()
            .children(rows)
            .children(skipped)
            .children(gone)
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
        let count = intent.request.changed_fields().len();
        let unit = if count == 1 { "field" } else { "fields" };
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
                        .child(format!("{count} {unit} changed")),
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
            let lines = intent.fields.iter().map(|field| {
                let text = match &field.value {
                    Some(value) => format!("{} → {value}", field.path),
                    None => field.path.clone(),
                };
                div().text_sm().font_family(mono.clone()).child(text)
            });
            let warnings = intent.warnings.iter().map(|warning| {
                div()
                    .text_sm()
                    .text_color(theme.warning)
                    .child(warning.clone())
            });
            return Some(v_flex().gap_1().children(lines).children(warnings));
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
        let lines = requests
            .into_iter()
            .flat_map(|request| request.changed_fields())
            .map(|field| {
                let text = match field.value {
                    Some(value) => format!("{} → {value}", field.path),
                    None if is_ordered => format!("{} → removed", field.path),
                    None => field.path.into_owned(),
                };
                div().text_sm().font_family(mono.clone()).child(text)
            });
        let warnings = warnings.iter().map(|warning| {
            div()
                .text_sm()
                .text_color(theme.warning)
                .child(warning.clone())
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

    fn render_dry_run(&self, cx: &App) -> Option<AnyElement> {
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
                    theme.success,
                )
            }
            DryRunState::Passed { elapsed } => (
                format!(
                    "Server dry-run passed · {} ms{}",
                    elapsed.as_millis(),
                    self.passed_note()
                ),
                theme.success,
            ),
            DryRunState::Failed(text) => (text.to_string(), theme.danger),
            DryRunState::Rejected(reason) => (
                format!("An admission webhook does not support dry-run: {reason}"),
                theme.danger,
            ),
        };
        Some(
            div()
                .text_sm()
                .text_color(color)
                .child(text)
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
        if !extras.kind.owns_dependents() {
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
        if matches!(self.live_tier(cx), DialogConfirm::Click) {
            return None;
        }
        let theme = cx.theme();
        let matches = self.typed_match(cx) == TypedMatch::Matches;
        Some(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(format!("Type {} to confirm", self.kind.typed_hint())),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().child(Input::new(&self.typed)))
                        .children(
                            matches.then(|| {
                                div().text_sm().text_color(theme.success).child("matches")
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The note field once the checkbox is ticked, and where the line goes when it is not saved.
    fn render_note_input(&self, cx: &App) -> Option<AnyElement> {
        if matches!(self.kind, DialogKind::Unlock { .. }) {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let no_folder = AppSettings::config_dir(cx).is_none();
        if !self.is_note_shown && !no_folder {
            return None;
        }
        Some(
            v_flex()
                .gap_1()
                .children(self.is_note_shown.then(|| Input::new(&self.note)))
                .children(no_folder.then(|| {
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("Not recorded: settings are not saved this session")
                }))
                .into_any_element(),
        )
    }

    /// The button row: the audit note checkbox on the left (a change only), Retry, Back, and the
    /// confirm button on the right.
    fn render_buttons(&self, block: bool, cx: &mut Context<Self>) -> AnyElement {
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
        let back = Button::new("write-back")
            .label("Back")
            .small()
            .outline()
            .on_click(cx.listener(|dialog, _, window, cx| dialog.close(window, cx)));
        let retry = self.can_retry().then(|| {
            Button::new("write-retry")
                .label("Retry")
                .small()
                .outline()
                .on_click(cx.listener(|dialog, _, window, cx| dialog.retry(window, cx)))
        });
        let note = (!matches!(self.kind, DialogKind::Unlock { .. })).then(|| {
            Checkbox::new("write-note")
                .label("Add a note to the audit log")
                .checked(self.is_note_shown)
                .on_click(cx.listener(|dialog, checked: &bool, _, cx| {
                    dialog.is_note_shown = *checked;
                    cx.notify();
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
        let typed_reason = format!("Type {} to confirm", self.kind.expected());
        let block_text = block
            .as_ref()
            .filter(|reason| reason.as_ref() != typed_reason)
            .filter(|_| matches!(self.dry_run, None | Some(DryRunState::Passed { .. })))
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
            .children(self.render_note_input(cx))
            .children(block_text.map(|text| div().text_xs().text_color(muted).child(text)))
            .child(self.render_buttons(block.is_some(), cx))
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

    pub(crate) fn environment(&self) -> Environment {
        self.environment
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

    pub(crate) fn press_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx);
    }

    pub(crate) fn press_retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.retry(window, cx);
    }

    pub(crate) fn choose_propagation_for_test(&mut self, index: usize, cx: &mut Context<Self>) {
        self.choose_propagation(index, cx);
    }
}
