//! The drain dialog (spec 0034 step 3a, wireframe W6): the options with their consequences, the
//! per-pod preview from the PodDisruptionBudgets, the server dry-run of the cordon and of every
//! eviction, and the typed name. It is a W6 front end, not a `GuardedKind`: it reuses
//! `confirm_step`, `TypedMatch`, `confirmed`, and the fresh-Enter rule, and sends every request
//! through `checked_write`, so the lock, the generation, and the audit stay in one place.
//!
//! It never sends a change itself except through `checked_write`: dry-runs while it is open, and
//! the cordons of `Cordon only` after the confirm. A child of `app_shell`, like `node_editor`:
//! the cluster, its guard, its connection, and its tier are the nodes' own, never the primary's.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use cluster::{
    AccessCheck, ClusterConnection, ClusterError, DrainPod, GracePeriod, NamespaceScope,
    NodeScheduling, ObjectKind, PodDisruptionBudgetSummary,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};

use super::AppShell;
use super::batch_write::{
    BATCH_RUNNING_REASON, ItemProgress, MAX_BATCH_ITEMS, batch_notice, commit_progress,
};
use super::drain_driver::DrainStart;
use super::write_flow::{
    CommitMode, Confirmed, DryRunState, TypedMatch, WriteIntent, WriteStep, checked_write,
    commit_block, confirmed, notify, notify_with, typed_match,
};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::confirm_dialog::{typed_prompt, typed_prompt_text};
use crate::drain_plan::{
    BudgetPolicy, CordonCheck, DrainOption, DrainOptions, GRACE_CHOICES, NodePlan, OptionCounts,
    PodCheck, PodKey, PreviewLine, TIMEOUT_CHOICES, bypass_note, drain_blocker, drain_dry_run,
    dry_run_text, eviction_count, grace_text, heads_up, node_plan, option_counts, option_hint,
    preview_lines, timeout_text,
};
use crate::drain_writes::{DrainScope, cordon_write, removal_write};
use crate::drawer::truncated_text_with_tooltip;
use crate::environment::{Environment, environment_badge};
use crate::keymap::FORWARD_FORM;
use crate::resource_actions::{
    ActionAvailability, ResourceAction, action_availability, action_label, unavailable_text,
    with_next_step,
};
use crate::status_tone::{StatusTone, tone_color};
use crate::write_guard::{ActionRisk, ConfirmMode, DialogConfirm, WriteLock, confirm_step};

const DIALOG_WIDTH: f32 = 600.;
/// Every preview row is this tall, so the scroll area cuts between rows, never through one.
const PREVIEW_ROW_HEIGHT: f32 = 24.;
const PREVIEW_VISIBLE_ROWS: f32 = 8.;
/// The result column of the preview is cut with an ellipsis past this width.
const RESULT_MAX_WIDTH: f32 = 300.;
/// The body above the typed name and the buttons scrolls past this height, so a small window
/// still reaches them.
const BODY_MAX_HEIGHT: f32 = 590.;

/// What the dialog reads when it opens: every budget of the cluster and the pods of each node, in
/// the order of the nodes.
struct Reads {
    budgets: Result<Vec<PodDisruptionBudgetSummary>, ClusterError>,
    pods: Vec<Result<Vec<DrainPod>, ClusterError>>,
}

/// The pods of one node: being read, not readable, or read.
enum PodsLoad {
    Loading,
    Failed(SharedString),
    Ready(Vec<DrainPod>),
}

/// The PodDisruptionBudgets of the cluster, read once.
enum BudgetsLoad {
    Loading,
    Failed(SharedString),
    Ready(Vec<PodDisruptionBudgetSummary>),
}

struct NodeData {
    name: String,
    /// The node was cordoned when the dialog opened: nothing to cordon, nothing to check.
    is_cordoned: bool,
    pods: PodsLoad,
}

/// The cluster-wide `delete pods` review behind Skip PodDisruptionBudgets: a drain deletes the pods
/// of every namespace on the node, so a right in the session's namespaces alone is not enough.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeleteReview {
    Checking,
    Allowed,
    Denied,
    Unknown,
}

/// What the dry-runs have answered so far.
#[derive(Default)]
struct Checks {
    /// By node name; only nodes that are not cordoned have one.
    cordons: HashMap<String, CordonCheck>,
    /// By pod uid.
    pods: HashMap<String, PodCheck>,
    elapsed: Duration,
}

/// The two buttons that confirm: each has its own tier, because only a drain touches budgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrainButton {
    Drain,
    CordonOnly,
}

/// One dry-run to send.
enum Job {
    Cordon(String),
    Evict(DrainPod),
}

/// The body of the drain dialog.
pub(crate) struct DrainDialog {
    shell: WeakEntity<AppShell>,
    cluster: ClusterRef,
    cluster_name: SharedString,
    environment: Environment,
    generation: u64,
    /// The tier the dialog opened with; the live one can only be stricter (`live_tier`).
    confirm: DialogConfirm,
    nodes: Vec<NodeData>,
    budgets: BudgetsLoad,
    options: DrainOptions,
    delete_review: DeleteReview,
    /// The grace the user chose before ticking Skip, put back when it is unticked.
    grace_before_skip: Option<GracePeriod>,
    plans: Vec<NodePlan>,
    checks: Checks,
    typed: Entity<InputState>,
    note: Entity<InputState>,
    grace: Entity<SelectState<Vec<String>>>,
    timeout: Entity<SelectState<Vec<String>>>,
    is_note_shown: bool,
    is_committing: bool,
    /// A dry-run loop is running; only one request is in flight at a time.
    is_checking: bool,
    /// False once the dialog is closed, by any button, Escape, or the overlay.
    is_open: bool,
    needs_focus: bool,
    focus_handle: FocusHandle,
    _load: Option<Task<()>>,
    _delete_review: Option<Task<()>>,
    _checks: Option<Task<()>>,
    /// `--screen drain-dialog`: a fixed picture that never loads, checks, or sends.
    #[cfg(feature = "screenshot")]
    is_fixture: bool,
    _subscriptions: Vec<Subscription>,
}

/// What a dialog is opened on.
struct DrainTarget {
    cluster: ClusterRef,
    cluster_name: SharedString,
    environment: Environment,
    generation: u64,
    confirm: DialogConfirm,
    /// The nodes in the order they were ticked, each with whether it is cordoned already.
    nodes: Vec<(String, bool)>,
}

fn select_of(
    items: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SelectState<Vec<String>>> {
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn grace_choices() -> Vec<GracePeriod> {
    std::iter::once(GracePeriod::PodDefault)
        .chain(GRACE_CHOICES.into_iter().map(GracePeriod::Seconds))
        .collect()
}

impl DrainDialog {
    fn new(
        shell: WeakEntity<AppShell>,
        target: DrainTarget,
        load: Option<(ClusterConnection, ClusterRuntime)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = DrainOptions::default();
        let typed = cx.new(|cx| InputState::new(window, cx).placeholder("Type here"));
        let note = cx.new(|cx| InputState::new(window, cx).placeholder("Note"));
        let grace_items: Vec<String> = grace_choices().into_iter().map(grace_text).collect();
        let grace = select_of(grace_items, 0, window, cx);
        let timeout_index = TIMEOUT_CHOICES
            .iter()
            .position(|choice| *choice == options.timeout)
            .unwrap_or(0);
        let timeout_items = TIMEOUT_CHOICES.into_iter().map(timeout_text).collect();
        let timeout = select_of(timeout_items, timeout_index, window, cx);
        let subscriptions = vec![
            // The match line follows the field as it is typed.
            cx.subscribe_in(&typed, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &grace,
                window,
                |dialog, _, _: &SelectEvent<Vec<String>>, _, cx| dialog.grace_picked(cx),
            ),
            cx.subscribe_in(
                &timeout,
                window,
                |dialog, _, _: &SelectEvent<Vec<String>>, _, cx| dialog.timeout_picked(cx),
            ),
        ];
        let nodes: Vec<NodeData> = target
            .nodes
            .iter()
            .map(|(name, is_cordoned)| NodeData {
                name: name.clone(),
                is_cordoned: *is_cordoned,
                pods: PodsLoad::Loading,
            })
            .collect();
        let mut checks = Checks::default();
        for node in nodes.iter().filter(|node| !node.is_cordoned) {
            checks
                .cordons
                .insert(node.name.clone(), CordonCheck::Waiting);
        }
        let names: Vec<String> = nodes.iter().map(|node| node.name.clone()).collect();
        // One review, cluster-wide (no namespace on the SelfSubjectAccessReview), asked now.
        let reviewing = load.as_ref().map(|(connection, runtime)| {
            let (connection, runtime) = (connection.clone(), runtime.clone());
            cx.spawn_in(window, async move |this, cx| {
                let read = runtime
                    .spawn(async move {
                        connection
                            .review_access_for(
                                &[AccessCheck::Delete(ObjectKind::Pod)],
                                NamespaceScope::All,
                            )
                            .await
                    })
                    .await;
                let _ = this.update(cx, |dialog, cx| dialog.reviewed(read, cx));
            })
        });
        let loading = load.map(|(connection, runtime)| {
            cx.spawn_in(window, async move |this, cx| {
                let read = runtime
                    .spawn(async move {
                        let budgets = connection.list_pod_disruption_budgets().await;
                        let mut pods = Vec::new();
                        for node in &names {
                            pods.push(connection.drain_pods(node).await);
                        }
                        Reads { budgets, pods }
                    })
                    .await;
                let _ = this.update(cx, |dialog, cx| dialog.loaded(read, cx));
            })
        });
        Self {
            shell,
            cluster: target.cluster,
            cluster_name: target.cluster_name,
            environment: target.environment,
            generation: target.generation,
            confirm: target.confirm,
            nodes,
            budgets: BudgetsLoad::Loading,
            options,
            // The picture has no cluster to ask.
            delete_review: if reviewing.is_some() {
                DeleteReview::Checking
            } else {
                DeleteReview::Allowed
            },
            grace_before_skip: None,
            plans: Vec::new(),
            checks,
            typed,
            note,
            grace,
            timeout,
            is_note_shown: false,
            is_committing: false,
            is_checking: false,
            is_open: true,
            needs_focus: true,
            focus_handle: cx.focus_handle(),
            _load: loading,
            _delete_review: reviewing,
            _checks: None,
            #[cfg(feature = "screenshot")]
            is_fixture: false,
            _subscriptions: subscriptions,
        }
    }

    /// The cluster-wide `delete pods` review answered.
    fn reviewed(
        &mut self,
        read: Result<Result<cluster::AccessReport, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.delete_review = match read {
            Ok(Ok(report)) if report.is_allowed(AccessCheck::Delete(ObjectKind::Pod)) => {
                DeleteReview::Allowed
            }
            Ok(Ok(_)) => DeleteReview::Denied,
            Ok(Err(_)) | Err(_) => DeleteReview::Unknown,
        };
        cx.notify();
    }

    /// The pods and budgets came back: each failure is shown where it belongs, and the dry-runs
    /// start from what could be read.
    fn loaded(&mut self, read: Result<Reads, tokio::task::JoinError>, cx: &mut Context<Self>) {
        match read {
            Ok(Reads { budgets, pods }) => {
                self.budgets = match budgets {
                    Ok(budgets) => BudgetsLoad::Ready(budgets),
                    Err(error) => BudgetsLoad::Failed(
                        format!("Could not list PodDisruptionBudgets: {error}").into(),
                    ),
                };
                for (node, pods) in self.nodes.iter_mut().zip(pods) {
                    node.pods = match pods {
                        Ok(pods) => PodsLoad::Ready(pods),
                        Err(error) => PodsLoad::Failed(
                            format!("Could not list pods on {}: {error}", node.name).into(),
                        ),
                    };
                }
            }
            Err(_) => {
                let text: SharedString = "The request task stopped".into();
                self.budgets = BudgetsLoad::Failed(text.clone());
                for node in &mut self.nodes {
                    node.pods = PodsLoad::Failed(text.clone());
                }
            }
        }
        self.replan();
        self.pump(cx);
        cx.notify();
    }

    /// The plans of every node whose pods and budgets are read, with the options as they are now.
    fn replan(&mut self) {
        let BudgetsLoad::Ready(budgets) = &self.budgets else {
            self.plans.clear();
            return;
        };
        self.plans = self
            .nodes
            .iter()
            .filter_map(|node| match &node.pods {
                PodsLoad::Ready(pods) => Some(node_plan(&node.name, pods, budgets, &self.options)),
                PodsLoad::Loading | PodsLoad::Failed(_) => None,
            })
            .collect();
        // A pod that left the plan keeps its answer; one that joined it waits for its dry-run.
        for planned in self.plans.iter().flat_map(|plan| plan.evictions()) {
            self.checks.pods.entry(planned.pod.uid.clone()).or_default();
        }
    }

    fn is_loading(&self) -> bool {
        matches!(self.budgets, BudgetsLoad::Loading)
            || self
                .nodes
                .iter()
                .any(|node| matches!(node.pods, PodsLoad::Loading))
    }

    /// The first thing that stops the drain from being previewed: a read that failed.
    fn load_problem(&self) -> Option<SharedString> {
        if let BudgetsLoad::Failed(text) = &self.budgets {
            return Some(text.clone());
        }
        self.nodes.iter().find_map(|node| match &node.pods {
            PodsLoad::Failed(text) => Some(text.clone()),
            PodsLoad::Loading | PodsLoad::Ready(_) => None,
        })
    }

    fn scope(&self) -> DrainScope<'_> {
        DrainScope {
            cluster: &self.cluster,
            cluster_name: &self.cluster_name,
        }
    }

    /// The next dry-run to send: the cordons first (they need no pod list), then the eviction of
    /// every `Evict` pod in list order. Marks it running.
    fn claim_next(&mut self) -> Option<Job> {
        let cordon = self
            .nodes
            .iter()
            .find(|node| self.checks.cordons.get(&node.name) == Some(&CordonCheck::Waiting));
        if let Some(node) = cordon {
            let name = node.name.clone();
            self.checks
                .cordons
                .insert(name.clone(), CordonCheck::Running);
            return Some(Job::Cordon(name));
        }
        let pod = self
            .plans
            .iter()
            .flat_map(|plan| plan.evictions())
            .find(|planned| self.checks.pods.get(&planned.pod.uid) == Some(&PodCheck::Waiting))
            .map(|planned| planned.pod.clone());
        match pod {
            Some(pod) => {
                self.checks.pods.insert(pod.uid.clone(), PodCheck::Running);
                Some(Job::Evict(pod))
            }
            None => {
                self.is_checking = false;
                None
            }
        }
    }

    /// Starts the dry-run loop when it is idle and something waits for a check. One request at a
    /// time, none audited; closing the dialog drops the task and ends the checks.
    fn pump(&mut self, cx: &mut Context<Self>) {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return;
        }
        if self.is_checking {
            return;
        }
        self.is_checking = true;
        let shell = self.shell.clone();
        self._checks = Some(cx.spawn(async move |this, cx| {
            loop {
                let step = this.update(cx, |dialog, _| dialog.next_step());
                let Ok(Some((job, step))) = step else {
                    break;
                };
                let result = checked_write(&shell, step, cx).await;
                if this
                    .update(cx, |dialog, cx| dialog.record(&job, result, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// The claimed job with the dry-run `WriteStep` that checks it.
    fn next_step(&mut self) -> Option<(Job, WriteStep)> {
        let job = self.claim_next()?;
        let intent = match &job {
            Job::Cordon(node) => cordon_write(self.scope(), node),
            Job::Evict(pod) => removal_write(self.scope(), &PodKey::of(pod), &self.options),
        };
        let Some(intent) = intent else {
            // A name the write path refuses: the check fails here, nothing is sent.
            self.fail_unsendable(&job);
            return self.next_step();
        };
        let step = WriteStep {
            intent: Rc::new(intent),
            generation: self.generation,
            mode: CommitMode::DryRun,
            note: None,
        };
        Some((job, step))
    }

    fn fail_unsendable(&mut self, job: &Job) {
        let text: SharedString = "The name is not valid for Kubernetes".into();
        match job {
            Job::Cordon(node) => {
                self.checks
                    .cordons
                    .insert(node.clone(), CordonCheck::Failed(text));
            }
            Job::Evict(pod) => {
                self.checks
                    .pods
                    .insert(pod.uid.clone(), PodCheck::Failed(text));
            }
        }
    }

    /// The answer of one dry-run. A 429 is the budget refusing the pod now: server truth for the
    /// preview, not a failure.
    fn record(
        &mut self,
        job: &Job,
        result: Result<cluster::WriteOutcome, super::write_flow::CheckedWriteError>,
        cx: &mut Context<Self>,
    ) {
        use cluster::WriteError;

        use super::write_flow::{CheckedWriteError, write_error_text};
        if let Ok(outcome) = &result {
            self.checks.elapsed += outcome.elapsed;
        }
        let failure = |error: &CheckedWriteError| -> SharedString {
            match error {
                CheckedWriteError::Blocked(text) => text.clone(),
                CheckedWriteError::Write(error) => write_error_text(error).into(),
            }
        };
        match job {
            Job::Cordon(node) => {
                let check = match &result {
                    Ok(_) => CordonCheck::Passed,
                    Err(error) => CordonCheck::Failed(failure(error)),
                };
                self.checks.cordons.insert(node.clone(), check);
            }
            Job::Evict(pod) => {
                let check = match &result {
                    Ok(_) => PodCheck::Accepted,
                    Err(CheckedWriteError::Write(WriteError::TooManyRequests {
                        message, ..
                    })) => PodCheck::Refused(message.clone().into()),
                    Err(error) => PodCheck::Failed(failure(error)),
                };
                self.checks.pods.insert(pod.uid.clone(), check);
            }
        }
        cx.notify();
    }

    fn grace_picked(&mut self, cx: &mut Context<Self>) {
        let index = self.grace.read(cx).selected_index(cx).map(|ix| ix.row);
        if let Some(grace) = index.and_then(|index| grace_choices().into_iter().nth(index)) {
            self.options.grace = grace;
            cx.notify();
        }
    }

    fn timeout_picked(&mut self, cx: &mut Context<Self>) {
        let index = self.timeout.read(cx).selected_index(cx).map(|ix| ix.row);
        if let Some(timeout) = index.and_then(|index| TIMEOUT_CHOICES.get(index)) {
            self.options.timeout = *timeout;
            cx.notify();
        }
    }

    fn set_option(&mut self, option: DrainOption, is_on: bool, cx: &mut Context<Self>) {
        if self.is_committing {
            return;
        }
        match option {
            DrainOption::IgnoreDaemonSets => self.options.ignore_daemon_sets = is_on,
            DrainOption::DeleteEmptyDir => self.options.delete_empty_dir = is_on,
            DrainOption::ForceUnmanaged => self.options.force_unmanaged = is_on,
        }
        // A ticked option adds pods to the plan; their dry-runs follow.
        self.replan();
        self.pump(cx);
        cx.notify();
    }

    /// Whether Skip PodDisruptionBudgets may be ticked: the cluster is open and unlocked, and the
    /// cluster-wide `delete pods` review allows it, because a drain deletes the pods of every
    /// namespace on the node. The dialog's own gate stays the eviction's.
    fn skip_gate(&self, cx: &App) -> ActionAvailability {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return ActionAvailability::Enabled;
        }
        let shell = self.shell.upgrade();
        let guard = shell
            .as_ref()
            .and_then(|shell| shell.read(cx).guard_for(&self.cluster, cx));
        let reason: SharedString = match (guard, self.delete_review) {
            (None, _) => "the cluster is not open".into(),
            (Some(guard), _) if guard.lock == WriteLock::Locked => {
                format!("{} is read-only", guard.display_name()).into()
            }
            (Some(_), DeleteReview::Checking) => "Checking permissions…".into(),
            (Some(_), DeleteReview::Unknown) => "Permissions could not be checked".into(),
            (Some(_), DeleteReview::Denied) => "Not permitted: delete pods".into(),
            (Some(_), DeleteReview::Allowed) => return ActionAvailability::Enabled,
        };
        ActionAvailability::Disabled { reason }
    }

    /// Why the Skip checkbox cannot change now, `None` when it can: the gate says no, a dry-run is
    /// in flight (its answer would be of the other request kind), or a commit runs.
    fn skip_block(&self, cx: &App) -> Option<SharedString> {
        // The picture has no cluster behind it, and its checkbox reads as an open one.
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return None;
        }
        if let ActionAvailability::Disabled { reason } = self.skip_gate(cx) {
            return Some(reason);
        }
        if self.is_committing {
            return Some("A drain is starting".into());
        }
        self.is_checking.then(|| "Waiting for the dry-runs".into())
    }

    /// Ticks or unticks Skip PodDisruptionBudgets. The request kind changes, so every pod's dry-run
    /// is asked again; the cordon dry-runs stand. The grace select reads `Pod default` while it is
    /// ticked: a delete takes no grace.
    fn set_skip_budgets(&mut self, is_on: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Not while a dry-run is in flight, so no late answer of the old kind is ever recorded.
        if self.skip_block(cx).is_some() {
            return;
        }
        let policy = if is_on {
            BudgetPolicy::Skip
        } else {
            BudgetPolicy::Respect
        };
        if self.options.budgets == policy {
            return;
        }
        self.options.budgets = policy;
        if is_on {
            // The name to type appears: the field takes the focus.
            self.typed.update(cx, |input, cx| input.focus(window, cx));
            self.grace_before_skip = Some(self.options.grace);
            self.options.grace = GracePeriod::PodDefault;
            self.show_grace(window, cx);
        } else if let Some(grace) = self.grace_before_skip.take() {
            self.options.grace = grace;
            self.show_grace(window, cx);
        }
        self.checks.pods.clear();
        self.checks.elapsed = Duration::ZERO;
        self.replan();
        self.pump(cx);
        cx.notify();
    }

    /// Makes the grace select show `self.options.grace`.
    fn show_grace(&self, window: &mut Window, cx: &mut Context<Self>) {
        let row = grace_choices()
            .iter()
            .position(|choice| *choice == self.options.grace)
            .unwrap_or(0);
        self.grace.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::default().row(row)), window, cx);
        });
    }

    fn drain_check(&self, uid: &str) -> PodCheck {
        self.checks.pods.get(uid).cloned().unwrap_or_default()
    }

    fn eviction_checks(&self) -> Vec<PodCheck> {
        self.plans
            .iter()
            .flat_map(|plan| plan.evictions())
            .map(|planned| self.drain_check(&planned.pod.uid))
            .collect()
    }

    fn cordon_checks(&self) -> Vec<CordonCheck> {
        self.nodes
            .iter()
            .filter_map(|node| self.checks.cordons.get(&node.name).cloned())
            .collect()
    }

    /// The dry-run state of the whole drain: the cordons and every eviction.
    fn drain_state(&self) -> DryRunState {
        if let Some(problem) = self.load_problem() {
            return DryRunState::Failed(problem);
        }
        if self.is_loading() {
            return DryRunState::Running;
        }
        drain_dry_run(
            &self.cordon_checks(),
            &self.eviction_checks(),
            self.checks.elapsed,
        )
    }

    /// The dry-run state of `Cordon only`: the cordons alone, so a refused eviction never holds it.
    fn cordon_state(&self) -> DryRunState {
        drain_dry_run(&self.cordon_checks(), &[], self.checks.elapsed)
    }

    /// Drain wants the single node's name for one node, the cluster's for several.
    fn expected(&self) -> &str {
        match self.nodes.as_slice() {
            [only] => &only.name,
            _ => &self.cluster_name,
        }
    }

    /// The tier of `button` now: the one the dialog opened with, or the live one of the cluster when
    /// the user made it stricter since (Settings), whichever asks for more. A drain that skips the
    /// budgets types the name in every tier; a cordon never touches budgets, so `Cordon only` keeps
    /// its own.
    fn live_tier(&self, button: DrainButton, cx: &App) -> DialogConfirm {
        if button == DrainButton::Drain && self.options.budgets == BudgetPolicy::Skip {
            return confirm_step(ConfirmMode::Click, ActionRisk::Privileged, self.expected());
        }
        let live = self.shell.upgrade().and_then(|shell| {
            shell.read(cx).guard_for(&self.cluster, cx).map(|guard| {
                confirm_step(
                    guard.profile.confirm,
                    ActionRisk::Destructive,
                    self.expected(),
                )
            })
        });
        match live {
            Some(tier @ DialogConfirm::TypeName { .. }) => tier,
            _ => self.confirm.clone(),
        }
    }

    fn typed_match(&self, button: DrainButton, cx: &App) -> TypedMatch {
        typed_match(&self.live_tier(button, cx), &self.typed.read(cx).value())
    }

    /// `commit_block` over `state`: the cluster is gone or reconnected, locked, the dry-run has not
    /// passed, or the name differs.
    fn commit_block_of(
        &self,
        state: &DryRunState,
        button: DrainButton,
        cx: &App,
    ) -> Option<SharedString> {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return None;
        }
        let shell = self.shell.upgrade();
        let guard = shell
            .as_ref()
            .and_then(|shell| shell.read(cx).guard_for(&self.cluster, cx));
        commit_block(
            guard.as_ref(),
            &self.cluster_name,
            self.generation,
            state,
            self.typed_match(button, cx),
            self.expected(),
        )
    }

    /// Why Drain is off, `None` when it is on: the pods are read, no pod needs an option that is not
    /// ticked, and the dry-run, the lock, and the typed name allow it.
    fn drain_block(&self, cx: &App) -> Option<SharedString> {
        if self.is_loading() {
            return Some("Loading pods…".into());
        }
        if let Some(problem) = self.load_problem() {
            return Some(problem);
        }
        if let Some(blocker) = drain_blocker(&self.plans) {
            return Some(blocker);
        }
        self.commit_block_of(&self.drain_state(), DrainButton::Drain, cx)
    }

    /// Why Cordon only is off, `None` when it is on.
    fn cordon_block(&self, cx: &App) -> Option<SharedString> {
        if self.checks.cordons.is_empty() {
            return Some(
                if self.nodes.len() == 1 {
                    "The node is already cordoned"
                } else {
                    "Every node is already cordoned"
                }
                .into(),
            );
        }
        self.commit_block_of(&self.cordon_state(), DrainButton::CordonOnly, cx)
    }

    /// Drain: closes the dialog and starts the run with the plan, the options, the proof of the
    /// confirm step, and the note. The first request is sent by the run, through `checked_write`.
    fn drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return;
        }
        if self.is_committing || self.drain_block(cx).is_some() {
            return;
        }
        let typed = self.typed_match(DrainButton::Drain, cx);
        let Some(proof) = confirmed(&self.drain_state(), typed, self.generation) else {
            return;
        };
        // A pod the dialog dry-ran (accepted or refused) is not dry-run again by the run.
        let checked = self
            .checks
            .pods
            .iter()
            .filter(|(_, check)| matches!(check, PodCheck::Accepted | PodCheck::Refused(_)))
            .map(|(uid, _)| uid.clone())
            .collect();
        let start = DrainStart {
            cluster: self.cluster.clone(),
            nodes: self.nodes.iter().map(|node| node.name.clone()).collect(),
            to_cordon: self
                .nodes
                .iter()
                .filter(|node| self.checks.cordons.contains_key(&node.name))
                .map(|node| node.name.clone())
                .collect(),
            options: self.options,
            confirmed: proof,
            generation: self.generation,
            checked,
            note: self
                .is_note_shown
                .then(|| self.note.read(cx).value().to_string()),
        };
        self.is_committing = true;
        if let Some(shell) = self.shell.upgrade() {
            shell.update(cx, |shell, cx| shell.start_drain_run(start, window, cx));
        }
        self.close(window, cx);
    }

    /// Cordon only: commits the cordon of every node that is not cordoned, one at a time through
    /// `checked_write`, then closes. No eviction is sent.
    fn cordon_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return;
        }
        if self.is_committing || self.cordon_block(cx).is_some() {
            return;
        }
        let typed = self.typed_match(DrainButton::CordonOnly, cx);
        let Some(proof) = confirmed(&self.cordon_state(), typed, self.generation) else {
            return;
        };
        let intents: Vec<WriteIntent> = self
            .nodes
            .iter()
            .filter(|node| self.checks.cordons.contains_key(&node.name))
            .filter_map(|node| cordon_write(self.scope(), &node.name))
            .collect();
        let note = self
            .is_note_shown
            .then(|| self.note.read(cx).value().to_string());
        let commit = CordonCommit {
            intents,
            proof,
            generation: self.generation,
            note,
        };
        self.is_committing = true;
        let (dialog, cluster) = (cx.weak_entity(), self.cluster.clone());
        if let Some(shell) = self.shell.upgrade() {
            shell.update(cx, |shell, cx| {
                shell.commit_cordons(&dialog, &cluster, commit, window, cx);
            });
        }
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_open = false;
        window.close_dialog(cx);
    }

    /// Whether the dialog is still open, for a commit result that arrives after Escape.
    pub(crate) fn is_open(&self) -> bool {
        self.is_open
    }

    /// Enter drains once, from the dialog or the typed-name field, and only a fresh press: a held
    /// Enter (the one that opened the dialog from a menu) repeats with `is_held` and is ignored.
    /// A focused button keeps its own Enter, so Cancel stays Cancel.
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
        if !event.is_held && self.drain_block(cx).is_none() {
            self.drain(window, cx);
        }
    }

    fn title(&self, cx: &App) -> AnyElement {
        let text = match self.nodes.as_slice() {
            [only] => format!("Drain node {}?", only.name),
            nodes => format!("Drain {} nodes?", nodes.len()),
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(environment_badge(&self.environment, cx))
            .child(div().flex_1().min_w_0().child(text))
            .into_any_element()
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

    fn render_steps(&self, cx: &App) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let evictions = eviction_count(&self.plans);
        let step = |number: &'static str, text: String| {
            h_flex()
                .gap_1()
                .items_center()
                .child(div().font_semibold().child(number))
                .child(div().text_color(muted).child(text))
        };
        h_flex()
            .gap_3()
            .flex_wrap()
            .text_xs()
            .child(step("1", "Cordon: stop new pods".to_owned()))
            .child(step(
                "2",
                match self.options.budgets {
                    BudgetPolicy::Respect => format!("Evict {evictions} pods"),
                    BudgetPolicy::Skip => format!("Delete {evictions} pods"),
                },
            ))
            .child(step("3", "Wait until done or timeout".to_owned()))
            .into_any_element()
    }

    fn render_option(
        &self,
        option: DrainOption,
        is_on: bool,
        counts: &OptionCounts,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_0p5()
            .child(
                Checkbox::new(option.label())
                    .label(option.label())
                    .checked(is_on)
                    .disabled(self.is_committing)
                    .on_click(cx.listener(move |dialog, checked: &bool, _, cx| {
                        dialog.set_option(option, *checked, cx);
                    })),
            )
            .child(
                div()
                    .pl_6()
                    .text_xs()
                    .text_color(muted)
                    .child(option_hint(option, counts)),
            )
            .into_any_element()
    }

    fn render_options(&self, cx: &mut Context<Self>) -> AnyElement {
        let counts = option_counts(&self.plans);
        let (muted, danger) = (cx.theme().muted_foreground, tone_color(StatusTone::Bad, cx));
        let skip_block = self.skip_block(cx);
        v_flex()
            .gap_2()
            .child(self.render_option(
                DrainOption::IgnoreDaemonSets,
                self.options.ignore_daemon_sets,
                &counts,
                cx,
            ))
            .child(self.render_option(
                DrainOption::DeleteEmptyDir,
                self.options.delete_empty_dir,
                &counts,
                cx,
            ))
            .child(self.render_option(
                DrainOption::ForceUnmanaged,
                self.options.force_unmanaged,
                &counts,
                cx,
            ))
            // Skipping the budgets deletes pods directly, which is a delete operation (spec 0033); it
            // starts off on every open and is never remembered (spec 0040).
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        Checkbox::new("drain-skip-pdbs")
                            .label("Skip PodDisruptionBudgets")
                            .checked(self.options.budgets == BudgetPolicy::Skip)
                            .disabled(skip_block.is_some())
                            .on_click(cx.listener(|dialog, checked: &bool, window, cx| {
                                dialog.set_skip_budgets(*checked, window, cx);
                            })),
                    )
                    .child(
                        h_flex()
                            .pl_6()
                            .gap_2()
                            .text_xs()
                            .child(
                                div()
                                    .text_color(danger)
                                    .child("Deletes pods directly. Can cause downtime."),
                            )
                            .children(
                                skip_block.map(|reason| div().text_color(muted).child(reason)),
                            )
                            // The grace select reads `Pod default` and is off while it is ticked.
                            .children((self.options.budgets == BudgetPolicy::Skip).then(|| {
                                div()
                                    .text_color(muted)
                                    .child("Deletes use each pod's own grace period")
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_timing(&self, cx: &App) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let is_direct = self.options.budgets == BudgetPolicy::Skip;
        h_flex()
            .gap_2()
            .items_center()
            .child(div().text_sm().text_color(muted).child("Grace period"))
            .child(
                div()
                    .w(px(140.))
                    .child(Select::new(&self.grace).small().disabled(is_direct)),
            )
            .child(
                div()
                    .ml_3()
                    .text_sm()
                    .text_color(muted)
                    .child("Timeout (per node)"),
            )
            .child(div().w(px(90.)).child(Select::new(&self.timeout).small()))
            .into_any_element()
    }

    fn render_preview(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (muted, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
        let mut rows: Vec<AnyElement> = Vec::new();
        // Outside the scroll area, so what the drain leaves alone is always in sight.
        let mut skipped: Vec<AnyElement> = Vec::new();
        for node in &self.nodes {
            match &node.pods {
                PodsLoad::Loading => rows.push(
                    div()
                        .h(px(PREVIEW_ROW_HEIGHT))
                        .text_xs()
                        .text_color(muted)
                        .child(format!("{}: Loading pods…", node.name))
                        .into_any_element(),
                ),
                PodsLoad::Failed(text) => rows.push(
                    div()
                        .h(px(PREVIEW_ROW_HEIGHT))
                        .text_xs()
                        .text_color(tone_color(StatusTone::Bad, cx))
                        .child(text.clone())
                        .into_any_element(),
                ),
                PodsLoad::Ready(_) => {}
            }
        }
        for (index, line) in preview_lines(&self.plans, |uid| self.drain_check(uid))
            .into_iter()
            .enumerate()
        {
            let is_skipped = matches!(line, PreviewLine::Skipped { .. });
            let row = match line {
                PreviewLine::Node(node) => div()
                    .h(px(PREVIEW_ROW_HEIGHT))
                    .pt_1()
                    .text_xs()
                    .font_semibold()
                    .child(node)
                    .into_any_element(),
                PreviewLine::Pod {
                    namespace,
                    name,
                    result,
                    tone,
                    detail,
                } => h_flex()
                    .h(px(PREVIEW_ROW_HEIGHT))
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_family(mono.clone())
                            .child(div().text_color(muted).child(format!("{namespace}/")))
                            .child(div().min_w_0().truncate().child(name)),
                    )
                    .child(
                        div()
                            .max_w(px(RESULT_MAX_WIDTH))
                            .text_xs()
                            .text_right()
                            .text_color(tone_color(tone, cx))
                            .child(match detail {
                                Some(detail) => truncated_text_with_tooltip(
                                    ("drain-result", index),
                                    result,
                                    detail,
                                )
                                .into_any_element(),
                                None => div().truncate().child(result).into_any_element(),
                            }),
                    )
                    .into_any_element(),
                PreviewLine::Skipped { text } => h_flex()
                    .h(px(PREVIEW_ROW_HEIGHT))
                    .items_center()
                    .gap_2()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_family(mono.clone())
                            .text_color(muted)
                            .child(text),
                    )
                    .into_any_element(),
            };
            if is_skipped {
                skipped.push(row);
            } else {
                rows.push(row);
            }
        }
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .text_color(muted)
                    .child(format!(
                        "{} · {}",
                        match self.options.budgets {
                            BudgetPolicy::Respect => "Pods to evict",
                            BudgetPolicy::Skip => "Pods to delete",
                        },
                        eviction_count(&self.plans)
                    ))
                    .child("Result"),
            )
            .child(
                v_flex()
                    .id("drain-preview")
                    .max_h(px(PREVIEW_ROW_HEIGHT * PREVIEW_VISIBLE_ROWS))
                    .overflow_y_scroll()
                    // Rows keep their height, or the flex column squeezes them under the cap.
                    .children(rows.into_iter().map(|row| div().flex_none().child(row))),
            )
            .children(skipped)
            .into_any_element()
    }

    fn render_heads_up(&self, cx: &App) -> Option<AnyElement> {
        // Skipping the budgets is the one danger note; waiting on a budget is the neutral one.
        let (text, is_danger) = match self.options.budgets {
            BudgetPolicy::Respect => (heads_up(&self.plans, self.options.timeout)?, false),
            BudgetPolicy::Skip => (bypass_note(&self.plans)?, true),
        };
        let theme = cx.theme();
        let (border, color) = if is_danger {
            (theme.danger, tone_color(StatusTone::Bad, cx))
        } else {
            (theme.border, theme.foreground)
        };
        Some(
            div()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(border)
                .bg(theme.muted)
                .text_color(color)
                .text_xs()
                .child(
                    h_flex()
                        .gap_1()
                        .items_start()
                        .child(div().flex_shrink_0().font_semibold().child("HEADS UP"))
                        .child(div().flex_1().min_w_0().child(format!("· {text}"))),
                )
                .into_any_element(),
        )
    }

    fn render_dry_run(&self, cx: &App) -> AnyElement {
        let state = self.drain_state();
        let text = dry_run_text(
            &state,
            &self.cordon_checks(),
            &self.eviction_checks(),
            self.options.budgets,
        );
        let color = match state {
            DryRunState::Passed { .. } => tone_color(StatusTone::Ok, cx),
            DryRunState::Failed(_) | DryRunState::Rejected(_) => tone_color(StatusTone::Bad, cx),
            DryRunState::Running | DryRunState::NotSupported => cx.theme().muted_foreground,
        };
        div()
            .text_sm()
            .text_color(color)
            .child(text)
            .into_any_element()
    }

    fn render_typed(&self, cx: &App) -> Option<AnyElement> {
        // The field is Drain's: when a cordon alone asks for it, so does the drain.
        if matches!(self.live_tier(DrainButton::Drain, cx), DialogConfirm::Click) {
            return None;
        }
        let matches = self.typed_match(DrainButton::Drain, cx) == TypedMatch::Matches;
        Some(
            v_flex()
                .gap_1()
                .child(typed_prompt(self.expected(), cx))
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

    fn render_note_input(&self, cx: &App) -> Option<AnyElement> {
        let muted = cx.theme().muted_foreground;
        let no_folder = crate::settings::AppSettings::config_dir(cx).is_none();
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
                        .child("Audit file unavailable: this change won't be logged")
                }))
                .into_any_element(),
        )
    }

    fn render_buttons(
        &self,
        cordon_block: bool,
        drain_block: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let drain_label = match self.nodes.as_slice() {
            [only] => format!("Drain {}", only.name),
            nodes => format!("Drain {} nodes", nodes.len()),
        };
        let note = Checkbox::new("drain-note")
            .label("Add a note to the audit log")
            .checked(self.is_note_shown)
            .on_click(cx.listener(|dialog, checked: &bool, _, cx| {
                dialog.is_note_shown = *checked;
                cx.notify();
            }));
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .justify_between()
            .child(
                h_flex().gap_2().items_center().child(
                    Button::new("drain-cancel")
                        .label("Cancel")
                        .small()
                        .outline()
                        .on_click(cx.listener(|dialog, _, window, cx| dialog.close(window, cx))),
                ),
            )
            .child(h_flex().child(note))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("drain-cordon-only")
                            .label("Cordon only")
                            .small()
                            .outline()
                            .disabled(cordon_block || self.is_committing)
                            .on_click(
                                cx.listener(|dialog, _, window, cx| dialog.cordon_only(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("drain-confirm")
                            .label(drain_label)
                            .small()
                            .danger()
                            .disabled(drain_block || self.is_committing)
                            .on_click(
                                cx.listener(|dialog, _, window, cx| dialog.drain(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for DrainDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.needs_focus = false;
            match self.live_tier(DrainButton::Drain, cx) {
                DialogConfirm::TypeName { .. } => {
                    self.typed.update(cx, |input, cx| input.focus(window, cx));
                }
                DialogConfirm::Click => window.focus(&self.focus_handle, cx),
            }
        }
        let muted = cx.theme().muted_foreground;
        let (drain_block, cordon_block) = (self.drain_block(cx), self.cordon_block(cx));
        // The reason a button is off, when the lines above do not already say it.
        let typed_reason = typed_prompt_text(self.expected());
        let block_text = drain_block
            .clone()
            .filter(|reason| reason.as_ref() != typed_reason)
            .filter(|_| !self.is_loading());
        let body = v_flex()
            .id("drain-body")
            .gap_2()
            .max_h(px(BODY_MAX_HEIGHT))
            .overflow_y_scroll()
            .child(self.render_steps(cx))
            .child(self.render_options(cx))
            .child(self.render_timing(cx))
            .child(self.render_preview(cx))
            .children(self.render_heads_up(cx))
            .child(self.render_dry_run(cx));
        // Outside the scroll area, so the name to type, the note, and the reason Drain is off are
        // never out of sight (ticking Skip makes the name appear below a full list).
        let reason = block_text.map(|text| {
            div()
                .text_xs()
                .text_color(muted)
                .child(with_next_step(&text))
        });
        v_flex()
            .key_context(FORWARD_FORM)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .child(body)
            .children(self.render_typed(cx))
            .children(self.render_note_input(cx))
            .children(reason)
            .child(self.render_buttons(cordon_block.is_some(), drain_block.is_some(), cx))
    }
}

/// What `Cordon only` commits.
pub(crate) struct CordonCommit {
    intents: Vec<WriteIntent>,
    proof: Confirmed,
    generation: u64,
    note: Option<String>,
}

impl AppShell {
    /// Opens the drain dialog for `nodes` of `cluster`, the row's, cursor's, or ticked rows' own
    /// cluster. Nothing is sent by opening it except the dry-runs, and nothing runs from a key.
    pub(crate) fn start_drain(
        &mut self,
        cluster: &ClusterRef,
        nodes: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::Drain);
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
            // A stale menu or a key pressed in a gap cannot bypass the gate.
            if let ActionAvailability::Disabled { reason } =
                action_availability(ResourceAction::Drain, &guard)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            if self.has_running_drain(cluster, cx) {
                let reason = format!("a drain is already running on {}", guard.display_name());
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            if self.running_batches.contains(cluster) {
                let reason = format!("{BATCH_RUNNING_REASON} on {}", guard.display_name());
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            if nodes.is_empty() || nodes.len() > MAX_BATCH_ITEMS {
                let reason = format!("select between 1 and {MAX_BATCH_ITEMS} nodes");
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let mut listed = Vec::new();
            for name in nodes {
                let Some(summary) = live.nodes.items().iter().find(|node| node.name == *name)
                else {
                    notify(
                        window,
                        cx,
                        unavailable_text(label, "the node is no longer listed"),
                    );
                    return;
                };
                let is_cordoned = summary.status.scheduling == NodeScheduling::Disabled;
                listed.push((name.clone(), is_cordoned));
            }
            let expected = match nodes {
                [only] => only.as_str(),
                _ => guard.display_name(),
            };
            let target = DrainTarget {
                cluster: cluster.clone(),
                cluster_name: guard.display_name().to_owned().into(),
                environment: guard.profile.environment.clone(),
                generation: guard.generation,
                confirm: confirm_step(guard.profile.confirm, ActionRisk::Destructive, expected),
                nodes: listed,
            };
            (target, live.connection().clone())
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let shell = cx.weak_entity();
        let dialog =
            cx.new(|cx| DrainDialog::new(shell, target, Some((connection, runtime)), window, cx));
        #[cfg(test)]
        {
            self.last_drain_dialog = Some(dialog.downgrade());
        }
        DrainDialog::open(&dialog, window, cx);
    }

    /// The commit of `Cordon only`, one node at a time through `checked_write`. Its task is
    /// detached: closing the dialog does not cancel a started commit, and it ends with the audit
    /// lines and the notice. A blocked commit (lock, session switch, reconnect) stops the rest.
    fn commit_cordons(
        &mut self,
        dialog: &WeakEntity<DrainDialog>,
        cluster: &ClusterRef,
        commit: CordonCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A batch on the same cluster would interleave its audit lines with these.
        if self.running_batches.contains(cluster) {
            let dialog = dialog.clone();
            cx.defer(move |cx| {
                let _ = dialog.update(cx, |dialog, cx| {
                    dialog.is_committing = false;
                    cx.notify();
                });
            });
            return;
        }
        self.running_batches.insert(cluster.clone());
        cx.notify();
        let (handle, shell) = (window.window_handle(), cx.weak_entity());
        let (dialog, cluster) = (dialog.clone(), cluster.clone());
        cx.spawn(async move |_, cx| {
            let mut results = Vec::new();
            let mut stopped: Option<SharedString> = None;
            for intent in commit.intents {
                let progress = match &stopped {
                    Some(reason) => ItemProgress::NotSent(reason.clone()),
                    None => {
                        let step = WriteStep {
                            intent: Rc::new(intent),
                            generation: commit.generation,
                            mode: CommitMode::Commit {
                                confirmed: commit.proof,
                            },
                            note: commit.note.clone(),
                        };
                        commit_progress(checked_write(&shell, step, cx).await, &mut stopped)
                    }
                };
                results.push(progress);
            }
            let _ = shell.update(cx, |shell, cx| {
                shell.running_batches.remove(&cluster);
                cx.notify();
            });
            let notice = batch_notice("Cordon only", &results);
            let is_success = results.iter().all(ItemProgress::is_settled);
            let _ = cx.update_window(handle, |_, window, cx| {
                // Only our own dialog closes, and only while it is open.
                let _ = dialog.update(cx, |dialog, cx| {
                    if dialog.is_open() {
                        dialog.close(window, cx);
                    }
                });
                notify_with(window, cx, notice, is_success);
            });
        })
        .detach();
    }

    /// The Drain… button of the Nodes selection bar and the D key resolve here: the ticked nodes,
    /// in the order the table shows them.
    pub(super) fn start_drain_of_ticked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The same rules as the bar: one cluster, at most 50 nodes, all still listed. Ticks of
        // another cluster are never dropped silently.
        let ticked = self.checked_objects(cx);
        let (cluster, nodes) = match self.ticked_nodes(&ticked, cx) {
            Ok(found) => found,
            Err(reason) => {
                let label = action_label(ResourceAction::Drain);
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
        };
        let names: Vec<String> = nodes.into_iter().map(|node| node.name).collect();
        self.start_drain(&cluster, &names, window, cx);
    }
}

/// `--screen drain-dialog`: the W6 dialog over fixed data, with no cluster behind it. Its buttons and
/// Enter do nothing, so it can never send.
#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen drain-dialog`, and `--screen drain-dialog-skip-pdbs` (spec 0040) on a Staging
    /// cluster with the budgets skipped.
    pub(super) fn open_drain_fixture(
        &mut self,
        launch: crate::launch_options::LaunchScreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        const NODE: &str = "wk-04";
        let is_skip = launch == crate::launch_options::LaunchScreen::DrainDialogSkipPdbs;
        let environment = if is_skip {
            Environment::STAGING
        } else {
            Environment::PRODUCTION
        };
        let tier = environment.tier();
        let target = DrainTarget {
            cluster: ClusterRef {
                kubeconfig: std::path::PathBuf::from("fixture.yaml"),
                context: "onprem-hn-1".to_owned(),
            },
            cluster_name: "onprem-hn-1".into(),
            environment,
            generation: 0,
            confirm: confirm_step(ConfirmMode::for_tier(tier), ActionRisk::Destructive, NODE),
            nodes: vec![(NODE.to_owned(), false)],
        };
        let shell = cx.weak_entity();
        let dialog = cx.new(|cx| {
            let mut dialog = DrainDialog::new(shell, target, None, window, cx);
            dialog.show_fixture(is_skip, window, cx);
            dialog
        });
        DrainDialog::open(&dialog, window, cx);
    }
}

#[cfg(feature = "screenshot")]
impl DrainDialog {
    /// Fills the dialog with the pods of W6 and the answers of their dry-runs.
    fn show_fixture(&mut self, is_skip: bool, window: &mut Window, cx: &mut Context<Self>) {
        use cluster::{ControllerRef, Selector};

        let pod =
            |namespace: &str, name: &str, app: &str, owner: Option<&str>, empty_dir| DrainPod {
                namespace: namespace.to_owned(),
                name: name.to_owned(),
                uid: format!("fixture-{name}"),
                labels: vec![format!("app={app}")],
                controller: owner.map(|kind| ControllerRef {
                    kind: kind.to_owned(),
                    name: "owner".to_owned(),
                }),
                is_mirror: false,
                has_empty_dir: empty_dir,
                is_finished: false,
                is_pending: false,
                is_terminating: false,
            };
        let budget = |namespace: &str, name: &str, app: &str, expected: u32, allowed: u32| {
            PodDisruptionBudgetSummary {
                namespace: namespace.to_owned(),
                name: name.to_owned(),
                created_at: None,
                labels: Vec::new(),
                min_available: Some(expected.to_string()),
                max_unavailable: None,
                selector: Selector::of_labels(&[format!("app={app}")]),
                current_healthy: expected,
                desired_healthy: expected,
                expected_pods: expected,
                disruptions_allowed: allowed,
                unhealthy_pod_eviction_policy: None,
                conditions: Vec::new(),
                is_status_stale: false,
            }
        };
        let mut pods = vec![
            pod(
                "payments",
                "api-7d9f8c-m8n2p",
                "api",
                Some("ReplicaSet"),
                false,
            ),
            pod(
                "payments",
                "api-7d9f8c-q9z4w",
                "api",
                Some("ReplicaSet"),
                false,
            ),
            pod("data", "kafka-1", "kafka", Some("StatefulSet"), false),
            pod("data", "cache-0", "cache", Some("StatefulSet"), true),
            pod("data", "cache-1", "cache", Some("StatefulSet"), true),
            pod("default", "debug-tools", "debug", None, false),
        ];
        pods.extend((0..19).map(|index| {
            let name = format!("frontend-6b8d7-p4k{index:02}");
            pod("web", &name, "frontend", Some("ReplicaSet"), false)
        }));
        pods.extend((0..6).map(|index| {
            let name = format!("node-agent-{index}");
            pod("kube-system", &name, "agent", Some("DaemonSet"), false)
        }));
        self.budgets = BudgetsLoad::Ready(vec![
            budget("payments", "api-pdb", "api", 2, 0),
            budget("data", "kafka-pdb", "kafka", 3, 1),
        ]);
        self.nodes[0].pods = PodsLoad::Ready(pods);
        self.options.delete_empty_dir = true;
        if is_skip {
            self.options.budgets = BudgetPolicy::Skip;
        }
        self.replan();
        self.checks
            .cordons
            .insert(self.nodes[0].name.clone(), CordonCheck::Passed);
        let refusal = "The disruption budget api-pdb needs 2 healthy pods and has 2 currently";
        for planned in self.plans.iter().flat_map(|plan| plan.evictions()) {
            // Deleting directly asks no budget, so every delete is accepted.
            let check = if !is_skip && planned.pod.name.starts_with("api-7d9f8c") {
                PodCheck::Refused(refusal.into())
            } else {
                PodCheck::Accepted
            };
            self.checks.pods.insert(planned.pod.uid.clone(), check);
        }
        // The skip picture asks for the typed name, which is what it shows: the field is empty and
        // takes the focus on its first render, as it does on a Production cluster.
        if !is_skip {
            self.typed.update(cx, |input, cx| {
                input.set_value("wk-04".to_owned(), window, cx)
            });
        }
        self.needs_focus = is_skip;
        self.is_fixture = true;
    }
}

/// What the shell tests read from, and do to, an open dialog.
#[cfg(test)]
impl DrainDialog {
    pub(crate) fn plans(&self) -> &[NodePlan] {
        &self.plans
    }

    pub(crate) fn options(&self) -> DrainOptions {
        self.options
    }

    pub(crate) fn state(&self) -> DryRunState {
        self.drain_state()
    }

    pub(crate) fn cordon_dry_run(&self) -> DryRunState {
        self.cordon_state()
    }

    pub(crate) fn drain_blocked_by(&self, cx: &App) -> Option<SharedString> {
        self.drain_block(cx)
    }

    pub(crate) fn cordon_blocked_by(&self, cx: &App) -> Option<SharedString> {
        self.cordon_block(cx)
    }

    pub(crate) fn check_of(&self, uid: &str) -> PodCheck {
        self.drain_check(uid)
    }

    pub(crate) fn dry_run_line(&self) -> String {
        dry_run_text(
            &self.drain_state(),
            &self.cordon_checks(),
            &self.eviction_checks(),
            self.options.budgets,
        )
    }

    /// Why the Skip checkbox cannot change now, `None` when it can.
    pub(crate) fn skip_blocked_by(&self, cx: &App) -> Option<SharedString> {
        self.skip_block(cx)
    }

    pub(crate) fn tick_skip(&mut self, is_on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.set_skip_budgets(is_on, window, cx);
    }

    /// Whether a dry-run loop is running now.
    pub(crate) fn is_checking(&self) -> bool {
        self.is_checking
    }

    /// The tier of Drain and of Cordon only now.
    pub(crate) fn live_tiers(&self, cx: &App) -> (DialogConfirm, DialogConfirm) {
        (
            self.live_tier(DrainButton::Drain, cx),
            self.live_tier(DrainButton::CordonOnly, cx),
        )
    }

    pub(crate) fn recorded_elapsed(&self) -> Duration {
        self.checks.elapsed
    }

    pub(crate) fn preview(&self) -> Vec<PreviewLine> {
        preview_lines(&self.plans, |uid| self.drain_check(uid))
    }

    pub(crate) fn tick(&mut self, option: DrainOption, is_on: bool, cx: &mut Context<Self>) {
        self.set_option(option, is_on, cx);
    }

    pub(crate) fn pick_grace(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.grace.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::default().row(index)), window, cx);
        });
        self.grace_picked(cx);
    }

    pub(crate) fn pick_timeout(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.timeout.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::default().row(index)), window, cx);
        });
        self.timeout_picked(cx);
    }

    pub(crate) fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.typed
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    pub(crate) fn press_cordon_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cordon_only(window, cx);
    }

    pub(crate) fn press_drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drain(window, cx);
    }

    /// Cancel: closes the dialog.
    pub(crate) fn close_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    pub(crate) fn expected_name(&self) -> String {
        self.expected().to_owned()
    }

    pub(crate) fn tier(&self) -> &DialogConfirm {
        &self.confirm
    }

    pub(crate) fn environment(&self) -> &Environment {
        &self.environment
    }

    pub(crate) fn is_busy_loading(&self) -> bool {
        self.is_loading()
    }
}
