//! The run of a drain (spec 0034 step 3b): cordon every node first, then evict and wait node by
//! node until each is drained, stuck, or the run is cancelled or stopped. A pure state machine over
//! simulated time (`now` is the time since the run started): it decides the next request and takes
//! the result of the last one, and sends nothing. The driver (`drain_driver`) sends every request
//! through `checked_write`, one at a time.

use std::collections::HashSet;
use std::time::Duration;

use cluster::{ControllerRef, DrainPod, PendingPod, WriteError, WriteOutcome};
use gpui_kit::SharedString;

use crate::app_shell::write_flow::{CheckedWriteError, Confirmed, write_error_text};
use crate::drain_plan::{
    BudgetPolicy, BudgetRefusal, DrainOptions, PodKey, PodVerdict, SkipReason, pod_count,
    run_verdict, timeout_text,
};
use crate::status_tone::StatusTone;

/// How often the node's pods are listed while evicted pods are being deleted.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_secs(3);
const BACKOFF_BASE: Duration = Duration::from_secs(5);
const BACKOFF_CAP: Duration = Duration::from_secs(30);
const NO_ANSWER: &str = "No answer; retrying";
/// How long after the last node ended the tab keeps looking for the replacements of the pods the
/// drain evicted.
const FOLLOW_WINDOW: Duration = Duration::from_secs(60);
/// A replacement is recreated within seconds; with none Pending for this long, the tab stops
/// looking before the window ends.
const FOLLOW_QUIET: Duration = Duration::from_secs(15);
const NOT_SCHEDULED_YET: &str = "not scheduled yet";

/// How long to wait before the `attempt`th retry of a refused eviction (1-based): at least the
/// server's hint, at least `5 s * 2^(attempt - 1)`, at most 30 s. No jitter: one client, one
/// request at a time.
pub(crate) fn retry_delay(attempt: u32, retry_after: Option<Duration>) -> Duration {
    let doublings = attempt.saturating_sub(1).min(16);
    let backoff = BACKOFF_BASE.saturating_mul(2u32.saturating_pow(doublings));
    retry_after
        .unwrap_or_default()
        .max(backoff)
        .min(BACKOFF_CAP)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PodProgress {
    /// Not evicted yet.
    Pending,
    /// The budget (or the API's priority and fairness) refused it for now.
    Refused {
        attempt: u32,
        retry_at: Duration,
        message: SharedString,
    },
    /// The eviction was accepted; waiting for the pod to go.
    Evicted,
    Gone,
    /// Gone from the node, and its controller made a replacement that is still Pending.
    Recreated {
        reason: SharedString,
    },
    Failed(SharedString),
    /// Already terminating when the node started: awaited, not evicted.
    Awaited,
    Skipped(SkipReason),
}

/// What the driver does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NextStep {
    Cordon(String),
    /// Lists the pods of the node that starts now.
    Read(String),
    DryRun(PodKey),
    Evict(PodKey),
    /// Lists the pods of the current node to see which evicted ones are gone.
    Poll,
    Sleep(Duration),
    NodeDone(NodeOutcome),
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeOutcome {
    Drained,
    Stuck { reason: SharedString },
}

/// How a node ended, for the tab and the audit line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeEnd {
    Drained,
    Stuck(SharedString),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunEnd {
    /// Every node was drained, or one got stuck and the rest were left alone.
    Finished,
    Cancelled,
    /// Stopped by the app: a lock, a switch, a reconnect, a failed cordon, or a quit.
    Stopped(SharedString),
}

struct PodRun {
    key: PodKey,
    controller: Option<ControllerRef>,
    progress: PodProgress,
    /// The server accepted a dry-run of this eviction (or the dialog showed its answer).
    is_checked: bool,
    /// The eviction request of this run was accepted.
    was_evicted: bool,
}

struct NodeRun {
    name: String,
    /// The pods were listed when the node started.
    is_read: bool,
    pods: Vec<PodRun>,
    end: Option<NodeEnd>,
    is_summarized: bool,
}

/// What a run starts from.
pub(crate) struct RunInput {
    /// In the order the nodes drain.
    pub(crate) nodes: Vec<String>,
    /// The nodes that are not cordoned yet; cordoned all first.
    pub(crate) to_cordon: Vec<String>,
    pub(crate) options: DrainOptions,
    /// The proof the dialog's confirm step was satisfied; `Copy`, sent with every commit.
    pub(crate) confirmed: Confirmed,
    pub(crate) generation: u64,
    /// Uids whose eviction the dialog already dry-ran.
    pub(crate) checked: HashSet<String>,
}

/// The look at the replacements of the evicted pods, after the nodes ended.
struct Follow {
    /// Run-relative time the look began.
    started: Duration,
    last_poll: Option<Duration>,
}

/// What the driver does next while it follows the replacements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FollowStep {
    /// Lists the Pending pods of the cluster.
    Poll,
    Sleep(Duration),
    Done,
}

pub(crate) struct DrainRun {
    nodes: Vec<NodeRun>,
    options: DrainOptions,
    to_cordon: Vec<String>,
    /// Cordoned by this run, in order: what the Uncordon button offers.
    cordoned: Vec<String>,
    current: usize,
    /// Run-relative start of the current node; the per-node timeout counts from it.
    node_started: Duration,
    last_poll: Option<Duration>,
    poll_error: Option<SharedString>,
    follow: Option<Follow>,
    end: Option<RunEnd>,
    confirmed: Confirmed,
    generation: u64,
    checked: HashSet<String>,
    /// The end notification was handed out.
    is_notified: bool,
    /// The commit (a cordon or an eviction) whose request was sent and has not answered yet: if
    /// the app quits now, its outcome is unknown.
    in_flight: Option<NextStep>,
}

/// One line of the audit summary of a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeSummary {
    pub(crate) node: String,
    pub(crate) evicted: usize,
    pub(crate) refused: usize,
    pub(crate) failed: usize,
    pub(crate) skipped: usize,
    /// Evictions sent and not answered when the run ended (a quit): they may have landed.
    pub(crate) unknown: usize,
    pub(crate) outcome: SummaryOutcome,
    /// Why a stuck node is stuck.
    pub(crate) reason: Option<SharedString>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SummaryOutcome {
    Drained,
    Stuck,
    Cancelled,
    Stopped,
}

impl DrainRun {
    pub(crate) fn new(input: RunInput) -> Self {
        Self {
            nodes: input
                .nodes
                .into_iter()
                .map(|name| NodeRun {
                    name,
                    is_read: false,
                    pods: Vec::new(),
                    end: None,
                    is_summarized: false,
                })
                .collect(),
            options: input.options,
            to_cordon: input.to_cordon,
            cordoned: Vec::new(),
            current: 0,
            node_started: Duration::ZERO,
            last_poll: None,
            poll_error: None,
            follow: None,
            end: None,
            confirmed: input.confirmed,
            generation: input.generation,
            checked: input.checked,
            is_notified: false,
            in_flight: None,
        }
    }

    pub(crate) fn options(&self) -> &DrainOptions {
        &self.options
    }

    pub(crate) fn confirmed(&self) -> Confirmed {
        self.confirmed
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn is_running(&self) -> bool {
        self.end.is_none()
    }

    pub(crate) fn end(&self) -> Option<&RunEnd> {
        self.end.as_ref()
    }

    /// Every node ended drained: the run ended clean.
    pub(crate) fn is_drained(&self) -> bool {
        self.end == Some(RunEnd::Finished)
            && self
                .nodes
                .iter()
                .all(|node| node.end == Some(NodeEnd::Drained))
    }

    /// The nodes this run cordoned, in order. They stay cordoned whatever happens.
    pub(crate) fn cordoned(&self) -> &[String] {
        &self.cordoned
    }

    pub(crate) fn poll_error(&self) -> Option<&SharedString> {
        self.poll_error.as_ref()
    }

    /// The node the run is working on now, `None` once every node ended.
    pub(crate) fn current_node(&self) -> Option<&str> {
        self.nodes.get(self.current).map(|node| node.name.as_str())
    }

    /// What to do next. The first match wins; once the run ended, every call says `Finished`, so no
    /// request is sent after Cancel.
    pub(crate) fn next_step(&self, now: Duration) -> NextStep {
        if self.end.is_some() {
            return NextStep::Finished;
        }
        if let Some(node) = self.to_cordon.first() {
            return NextStep::Cordon(node.clone());
        }
        let Some(node) = self.nodes.get(self.current) else {
            return NextStep::Finished;
        };
        if !node.is_read {
            return NextStep::Read(node.name.clone());
        }
        let elapsed = now.saturating_sub(self.node_started);
        if elapsed >= self.options.timeout {
            return NextStep::NodeDone(NodeOutcome::Stuck {
                reason: format!(
                    "Timed out after {}: {} left",
                    timeout_text(self.options.timeout),
                    pod_count(node.left())
                )
                .into(),
            });
        }
        if let Some(pod) = node
            .pods
            .iter()
            .find(|pod| pod.progress == PodProgress::Pending && !pod.is_checked)
        {
            return NextStep::DryRun(pod.key.clone());
        }
        let due = node.pods.iter().find(|pod| match &pod.progress {
            PodProgress::Pending => true,
            PodProgress::Refused { retry_at, .. } => *retry_at <= now,
            _ => false,
        });
        if let Some(pod) = due {
            return NextStep::Evict(pod.key.clone());
        }
        let is_waiting_for_deletion = node
            .pods
            .iter()
            .any(|pod| matches!(pod.progress, PodProgress::Evicted | PodProgress::Awaited));
        let next_poll = self
            .last_poll
            .map_or(Duration::ZERO, |polled| polled + POLL_INTERVAL);
        if is_waiting_for_deletion && next_poll <= now {
            return NextStep::Poll;
        }
        let is_open = |pod: &&PodRun| {
            matches!(
                pod.progress,
                PodProgress::Pending
                    | PodProgress::Refused { .. }
                    | PodProgress::Evicted
                    | PodProgress::Awaited
            )
        };
        if !node.pods.iter().any(|pod| is_open(&pod)) {
            let failure = node.pods.iter().find_map(|pod| match &pod.progress {
                PodProgress::Failed(text) => Some(text.clone()),
                _ => None,
            });
            return NextStep::NodeDone(match failure {
                Some(reason) => NodeOutcome::Stuck { reason },
                None => NodeOutcome::Drained,
            });
        }
        // Wake for the earliest of: a retry, the next poll, the node's timeout.
        let mut wake = self.node_started + self.options.timeout;
        for pod in &node.pods {
            if let PodProgress::Refused { retry_at, .. } = pod.progress {
                wake = wake.min(retry_at);
            }
        }
        if is_waiting_for_deletion {
            wake = wake.min(next_poll);
        }
        NextStep::Sleep(wake.saturating_sub(now).max(Duration::from_millis(1)))
    }

    /// The driver is about to send `step`. Only a commit is kept: a dry-run changes nothing, so it
    /// has no outcome to lose.
    pub(crate) fn begin_write(&mut self, step: &NextStep) {
        self.in_flight =
            matches!(step, NextStep::Cordon(_) | NextStep::Evict(_)).then(|| step.clone());
    }

    /// The commit sent and not answered yet.
    pub(crate) fn in_flight(&self) -> Option<&NextStep> {
        self.in_flight.as_ref()
    }

    /// The result of the request `sent` (a `Cordon`, `DryRun`, or `Evict`). A result that arrives
    /// after Cancel is applied too: the request had already left.
    pub(crate) fn on_write(
        &mut self,
        sent: &NextStep,
        result: Result<WriteOutcome, CheckedWriteError>,
        now: Duration,
    ) {
        self.in_flight = None;
        // A blocked write (lock, session switch, reconnect) stops the whole run.
        if let Err(CheckedWriteError::Blocked(text)) = &result {
            self.stop(format!("{text}; drain stopped"));
            return;
        }
        match sent {
            NextStep::Cordon(node) => self.on_cordon(node, result),
            NextStep::DryRun(key) => self.on_dry_run(key, result),
            NextStep::Evict(key) => self.on_evict(key, result, now),
            _ => {}
        }
    }

    fn on_cordon(&mut self, node: &str, result: Result<WriteOutcome, CheckedWriteError>) {
        match result {
            Ok(_) => {
                self.to_cordon.retain(|name| name != node);
                self.cordoned.push(node.to_owned());
            }
            Err(error) => {
                let text = failure_text(&error);
                self.stop(format!("Could not cordon {node}: {text}"));
            }
        }
    }

    fn pod_mut(&mut self, key: &PodKey) -> Option<&mut PodRun> {
        let node = self.nodes.get_mut(self.current)?;
        node.pods.iter_mut().find(|pod| pod.key == *key)
    }

    fn on_dry_run(&mut self, key: &PodKey, result: Result<WriteOutcome, CheckedWriteError>) {
        let Some(pod) = self.pod_mut(key) else {
            return;
        };
        match result {
            // A refusal is an expected wait: the eviction itself retries it.
            Ok(_) | Err(CheckedWriteError::Write(WriteError::TooManyRequests { .. })) => {
                pod.is_checked = true;
            }
            Err(error) => pod.progress = PodProgress::Failed(failure_text(&error).into()),
        }
    }

    fn on_evict(
        &mut self,
        key: &PodKey,
        result: Result<WriteOutcome, CheckedWriteError>,
        now: Duration,
    ) {
        let Some(pod) = self.pod_mut(key) else {
            return;
        };
        let attempt = match pod.progress {
            PodProgress::Refused { attempt, .. } => attempt + 1,
            _ => 1,
        };
        let refused = |message: SharedString, after: Option<Duration>| PodProgress::Refused {
            attempt,
            retry_at: now + retry_delay(attempt, after),
            message,
        };
        pod.progress = match result {
            Ok(_) => {
                pod.was_evicted = true;
                PodProgress::Evicted
            }
            Err(CheckedWriteError::Write(WriteError::TooManyRequests {
                message,
                retry_after,
            })) => refused(message.into(), retry_after),
            // The uid precondition failed or the pod is gone: the pod with that uid no longer exists.
            Err(CheckedWriteError::Write(WriteError::NotFound | WriteError::Conflict { .. })) => {
                PodProgress::Gone
            }
            // A repeat carries the uid precondition, so it is safe; the poll marks the pod gone if
            // the first request landed.
            Err(CheckedWriteError::Write(WriteError::OutcomeUnknown)) => {
                refused(NO_ANSWER.into(), None)
            }
            Err(error) => PodProgress::Failed(failure_text(&error).into()),
        };
    }

    /// The pods of the node that starts: classified with the dialog's options. A pod that needs an
    /// option the user did not tick is failed up front and makes the node stuck.
    pub(crate) fn on_read(&mut self, read: Result<Vec<DrainPod>, SharedString>, now: Duration) {
        let Some(node) = self.nodes.get_mut(self.current) else {
            return;
        };
        node.is_read = true;
        self.node_started = now;
        self.last_poll = None;
        match read {
            Ok(pods) => {
                node.pods = pods
                    .iter()
                    .map(|pod| classify(pod, &self.options, &self.checked))
                    .collect();
            }
            Err(text) => node.end = Some(NodeEnd::Stuck(text)),
        }
        if matches!(node.end, Some(NodeEnd::Stuck(_))) {
            self.end = self.end.take().or(Some(RunEnd::Finished));
        }
    }

    /// The pods of the current node, listed to find which evicted ones are gone: an evicted or
    /// awaited pod whose uid is absent is gone (a new pod of the same name has another uid).
    pub(crate) fn on_poll(&mut self, read: Result<Vec<DrainPod>, SharedString>, now: Duration) {
        self.last_poll = Some(now);
        let present = match read {
            Ok(present) => present,
            Err(text) => {
                self.poll_error = Some(text);
                return;
            }
        };
        self.poll_error = None;
        let Some(node) = self.nodes.get_mut(self.current) else {
            return;
        };
        let uids: HashSet<&str> = present.iter().map(|pod| pod.uid.as_str()).collect();
        for pod in &mut node.pods {
            let is_waiting = matches!(pod.progress, PodProgress::Evicted | PodProgress::Awaited);
            if is_waiting && !uids.contains(pod.key.uid.as_str()) {
                pod.progress = PodProgress::Gone;
            }
        }
    }

    /// The current node ended. A stuck node stops the run: the later nodes stay cordoned and
    /// untouched. The last node ends the run too.
    pub(crate) fn on_node_done(&mut self, outcome: NodeOutcome) {
        let Some(node) = self.nodes.get_mut(self.current) else {
            return;
        };
        let is_stuck = matches!(outcome, NodeOutcome::Stuck { .. });
        node.end = Some(match outcome {
            NodeOutcome::Drained => NodeEnd::Drained,
            NodeOutcome::Stuck { reason } => NodeEnd::Stuck(reason),
        });
        self.current += 1;
        self.last_poll = None;
        self.poll_error = None;
        if is_stuck || self.current >= self.nodes.len() {
            self.end.get_or_insert(RunEnd::Finished);
        }
    }

    /// Starts following the replacements of the evicted pods: only after the run finished, and
    /// only when it evicted a pod that has a controller to make a replacement.
    pub(crate) fn start_follow(&mut self, now: Duration) {
        let has_replaceable = self
            .nodes
            .iter()
            .flat_map(|node| &node.pods)
            .any(|pod| pod.was_evicted && pod.controller.is_some());
        if self.end == Some(RunEnd::Finished) && has_replaceable && self.follow.is_none() {
            self.follow = Some(Follow {
                started: now,
                last_poll: None,
            });
        }
    }

    /// What to do next while following: the Pending pods are listed every poll interval until the
    /// window ends, or until a quiet spell with none of the evicted pods recreated Pending.
    pub(crate) fn next_follow(&self, now: Duration) -> FollowStep {
        let Some(follow) = &self.follow else {
            return FollowStep::Done;
        };
        let elapsed = now.saturating_sub(follow.started);
        let is_quiet = elapsed >= FOLLOW_QUIET && self.pending_replacements() == 0;
        if elapsed >= FOLLOW_WINDOW || is_quiet {
            return FollowStep::Done;
        }
        let next_poll = follow
            .last_poll
            .map_or(Duration::ZERO, |polled| polled + POLL_INTERVAL);
        if next_poll <= now {
            return FollowStep::Poll;
        }
        FollowStep::Sleep(next_poll - now)
    }

    /// Whether the tab still looks for replacements at `now`.
    pub(crate) fn is_following(&self, now: Duration) -> bool {
        self.next_follow(now) != FollowStep::Done
    }

    /// The Pending pods of the cluster: an evicted pod whose controller made a replacement that is
    /// still Pending reads `recreated`; one whose replacement is gone from the list reads `Gone`
    /// again. A replacement is told from the pod it replaces by its uid.
    pub(crate) fn on_follow(&mut self, read: Result<Vec<PendingPod>, SharedString>, now: Duration) {
        if let Some(follow) = &mut self.follow {
            follow.last_poll = Some(now);
        }
        let pending = match read {
            Ok(pending) => pending,
            Err(text) => {
                self.poll_error = Some(text);
                return;
            }
        };
        self.poll_error = None;
        let known: HashSet<String> = self
            .nodes
            .iter()
            .flat_map(|node| &node.pods)
            .map(|pod| pod.key.uid.clone())
            .collect();
        let mut taken: HashSet<&str> = HashSet::new();
        let followed = self
            .nodes
            .iter_mut()
            .flat_map(|node| &mut node.pods)
            .filter(|pod| pod.was_evicted && pod.controller.is_some())
            .filter(|pod| {
                matches!(
                    pod.progress,
                    PodProgress::Gone | PodProgress::Recreated { .. }
                )
            });
        for pod in followed {
            let replacement = pending.iter().find(|candidate| {
                candidate.namespace == pod.key.namespace
                    && candidate.controller == pod.controller
                    && !known.contains(&candidate.uid)
                    && !taken.contains(candidate.uid.as_str())
            });
            pod.progress = match replacement {
                Some(replacement) => {
                    taken.insert(&replacement.uid);
                    PodProgress::Recreated {
                        reason: replacement
                            .reason
                            .as_deref()
                            .unwrap_or(NOT_SCHEDULED_YET)
                            .to_owned()
                            .into(),
                    }
                }
                None => PodProgress::Gone,
            };
        }
    }

    /// Evicted pods whose replacement is Pending now.
    pub(crate) fn pending_replacements(&self) -> usize {
        self.nodes
            .iter()
            .map(|node| node.count(|progress| matches!(progress, PodProgress::Recreated { .. })))
            .sum()
    }

    /// The notification when the look ended with replacements still Pending, `None` otherwise.
    pub(crate) fn follow_notice(&self) -> Option<String> {
        match self.pending_replacements() {
            0 => None,
            1 => Some("Drain: 1 evicted pod has a replacement that stays Pending".to_owned()),
            count => Some(format!(
                "Drain: {count} evicted pods have replacements that stay Pending"
            )),
        }
    }

    /// Cancel: no new request is sent, nodes stay cordoned, no eviction is undone.
    pub(crate) fn cancel(&mut self) {
        self.end.get_or_insert(RunEnd::Cancelled);
    }

    /// The app stops the run (a lock, a switch, a reconnect, a quit), like Cancel with a reason.
    pub(crate) fn stop(&mut self, reason: impl Into<SharedString>) {
        self.end.get_or_insert(RunEnd::Stopped(reason.into()));
    }

    /// The audit lines of the nodes that were reached and have none yet: each node ended
    /// (`drained`, `stuck`), or the run ended while it was current (`cancelled`, `stopped`). A node
    /// never reached has no line.
    pub(crate) fn take_summaries(&mut self) -> Vec<NodeSummary> {
        let run_end = self.end.clone();
        let in_flight = self.in_flight.clone();
        let current = self.current;
        let mut lines = Vec::new();
        for (index, node) in self.nodes.iter_mut().enumerate() {
            if node.is_summarized || !node.is_read {
                continue;
            }
            let outcome = match (&node.end, &run_end) {
                (Some(NodeEnd::Drained), _) => SummaryOutcome::Drained,
                (Some(NodeEnd::Stuck(_)), _) => SummaryOutcome::Stuck,
                // The node the run was on when it ended.
                (None, Some(RunEnd::Cancelled)) if index == current => SummaryOutcome::Cancelled,
                (None, Some(RunEnd::Stopped(_))) if index == current => SummaryOutcome::Stopped,
                (None, _) => continue,
            };
            node.is_summarized = true;
            let unknown = match &in_flight {
                Some(NextStep::Evict(key)) if index == current => {
                    usize::from(node.pods.iter().any(|pod| pod.key == *key))
                }
                _ => 0,
            };
            let reason = match &node.end {
                Some(NodeEnd::Stuck(reason)) => Some(reason.clone()),
                _ => None,
            };
            lines.push(NodeSummary {
                node: node.name.clone(),
                evicted: node.pods.iter().filter(|pod| pod.was_evicted).count(),
                refused: node.count(|progress| matches!(progress, PodProgress::Refused { .. })),
                failed: node.count(|progress| matches!(progress, PodProgress::Failed(_))),
                skipped: node.count(|progress| matches!(progress, PodProgress::Skipped(_))),
                unknown,
                outcome,
                reason,
            });
        }
        lines
    }
}

impl NodeRun {
    fn count(&self, wanted: impl Fn(&PodProgress) -> bool) -> usize {
        self.pods.iter().filter(|pod| wanted(&pod.progress)).count()
    }

    /// Pods that still need the drain's attention: not gone and not skipped.
    fn left(&self) -> usize {
        self.count(|progress| {
            !matches!(
                progress,
                PodProgress::Gone | PodProgress::Recreated { .. } | PodProgress::Skipped(_)
            )
        })
    }
}

fn failure_text(error: &CheckedWriteError) -> String {
    match error {
        CheckedWriteError::Blocked(text) => text.to_string(),
        CheckedWriteError::Write(error) => write_error_text(error),
    }
}

fn classify(pod: &DrainPod, options: &DrainOptions, checked: &HashSet<String>) -> PodRun {
    let progress = match run_verdict(pod, options) {
        PodVerdict::Evict(_) => PodProgress::Pending,
        PodVerdict::Terminating => PodProgress::Awaited,
        PodVerdict::Skip(reason) => PodProgress::Skipped(reason),
        PodVerdict::Needs(option) => {
            PodProgress::Failed(format!("Not evicted: needs {}", option.label()).into())
        }
        PodVerdict::Refused(text) => PodProgress::Failed(text),
    };
    PodRun {
        key: PodKey::of(pod),
        controller: pod.controller.clone(),
        progress,
        is_checked: checked.contains(&pod.uid),
        was_evicted: false,
    }
}

// ---- What the tab shows ----

/// The state of a node in the tab's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeState {
    Waiting,
    Cordoning,
    /// `gone` of `total` pods the drain evicts or awaits.
    Evicting {
        gone: usize,
        total: usize,
        /// `Skip` reads `Deleting`: the pods are deleted, not evicted.
        budgets: BudgetPolicy,
    },
    Drained,
    Stuck(SharedString),
    Cancelled,
    Stopped,
}

impl NodeState {
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Waiting => "Waiting".to_owned(),
            Self::Cordoning => "Cordoning".to_owned(),
            Self::Evicting {
                gone,
                total,
                budgets: BudgetPolicy::Respect,
            } => format!("Evicting {gone}/{total}"),
            Self::Evicting {
                gone,
                total,
                budgets: BudgetPolicy::Skip,
            } => format!("Deleting {gone}/{total}"),
            Self::Drained => "Drained".to_owned(),
            Self::Stuck(_) => "Stuck".to_owned(),
            Self::Cancelled => "Cancelled".to_owned(),
            Self::Stopped => "Stopped".to_owned(),
        }
    }
}

/// A PodDisruptionBudget that refused an eviction of a stuck drain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlockingBudget {
    pub(crate) namespace: String,
    pub(crate) name: String,
}

/// The header of the tab in three parts, so the tab can draw the budget names as links:
/// `lead` · blocked by `blockers` `tail`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusLine {
    pub(crate) lead: String,
    pub(crate) blockers: Vec<BlockingBudget>,
    /// Starts with its own separator, or is empty.
    pub(crate) tail: String,
}

impl StatusLine {
    #[cfg(test)]
    fn text(&self) -> String {
        let blockers = match self.blockers.as_slice() {
            [] => String::new(),
            budgets => {
                let names: Vec<&str> = budgets.iter().map(|budget| budget.name.as_str()).collect();
                format!(" · blocked by {}", names.join(", "))
            }
        };
        format!("{}{blockers}{}", self.lead, self.tail)
    }
}

/// One pod line of the tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PodRow {
    pub(crate) pod: SharedString,
    pub(crate) text: SharedString,
    pub(crate) tone: StatusTone,
}

impl DrainRun {
    pub(crate) fn node_states(&self) -> Vec<(String, NodeState)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.name.clone(), self.state_of(index, node)))
            .collect()
    }

    fn state_of(&self, index: usize, node: &NodeRun) -> NodeState {
        match &node.end {
            Some(NodeEnd::Drained) => return NodeState::Drained,
            Some(NodeEnd::Stuck(reason)) => return NodeState::Stuck(reason.clone()),
            None => {}
        }
        let is_current = index == self.current;
        match &self.end {
            // The node the run was working on; the others were never touched.
            Some(RunEnd::Cancelled) if is_current && node.is_read => NodeState::Cancelled,
            Some(RunEnd::Stopped(_)) if is_current && node.is_read => NodeState::Stopped,
            Some(_) => NodeState::Waiting,
            None if !self.to_cordon.is_empty() => NodeState::Cordoning,
            None if is_current => {
                let (gone, total) = node.progress();
                NodeState::Evicting {
                    gone,
                    total,
                    budgets: self.options.budgets,
                }
            }
            None => NodeState::Waiting,
        }
    }

    /// Pods done and pods to evict on the current node, for the progress bar.
    pub(crate) fn progress(&self) -> (usize, usize) {
        self.nodes
            .get(self.current.min(self.nodes.len().saturating_sub(1)))
            .map_or((0, 0), NodeRun::progress)
    }

    /// Time left on the current node, `None` between nodes and after the run.
    pub(crate) fn timeout_left(&self, now: Duration) -> Option<Duration> {
        let node = self.nodes.get(self.current)?;
        if !node.is_read || self.end.is_some() {
            return None;
        }
        Some(
            self.options
                .timeout
                .saturating_sub(now.saturating_sub(self.node_started)),
        )
    }

    /// The pod lines of the current node: bad and warning rows first, then the rest in list order.
    pub(crate) fn pod_rows(&self, now: Duration) -> Vec<PodRow> {
        let index = self.current.min(self.nodes.len().saturating_sub(1));
        let Some(node) = self.nodes.get(index) else {
            return Vec::new();
        };
        let mut rows: Vec<(u8, PodRow)> = node
            .pods
            .iter()
            .map(|pod| {
                let (text, tone) =
                    pod_text(&pod.progress, now, self.options.budgets, self.end.is_some());
                let rank = match tone {
                    StatusTone::Bad => 0,
                    StatusTone::Warn => 1,
                    _ => 2,
                };
                (
                    rank,
                    PodRow {
                        pod: pod.key.text().into(),
                        text: text.into(),
                        tone,
                    },
                )
            })
            .collect();
        rows.sort_by_key(|(rank, _)| *rank);
        rows.into_iter().map(|(_, row)| row).collect()
    }

    /// The notification at the end of the run, once: whoever ends the run (the driver, or the app
    /// releasing the cluster) takes it, and the other finds nothing to say again.
    pub(crate) fn take_end_notice(&mut self) -> Option<String> {
        if self.is_notified {
            return None;
        }
        let notice = self.end_notice()?;
        self.is_notified = true;
        Some(notice)
    }

    /// `status_line` as one string.
    #[cfg(test)]
    pub(crate) fn status_text(&self, now: Duration) -> String {
        self.status_line(now).text()
    }

    /// The budgets that refused the open pods of the node the run is stuck on, in the order the
    /// pods list them. The budget lives in its pod's namespace.
    fn blocking_budgets(&self, node: &NodeRun) -> Vec<BlockingBudget> {
        let mut budgets: Vec<BlockingBudget> = Vec::new();
        for pod in &node.pods {
            let PodProgress::Refused { message, .. } = &pod.progress else {
                continue;
            };
            let Some(refusal) = BudgetRefusal::parse(message) else {
                continue;
            };
            let budget = BlockingBudget {
                namespace: pod.key.namespace.clone(),
                name: refusal.name.to_owned(),
            };
            if !budgets.contains(&budget) {
                budgets.push(budget);
            }
        }
        budgets
    }

    /// The header line of the tab: what the run is doing, or how it ended.
    pub(crate) fn status_line(&self, now: Duration) -> StatusLine {
        let cordoned = || {
            if self.cordoned.is_empty() {
                String::new()
            } else {
                format!(" · cordoned: {}", self.cordoned.join(", "))
            }
        };
        let lead = match &self.end {
            None if !self.to_cordon.is_empty() => {
                format!("Cordoning {}…", self.to_cordon.join(", "))
            }
            None => {
                let node = self.current_node().unwrap_or_default();
                let position = match self.nodes.len() {
                    0 | 1 => String::new(),
                    total => format!(" ({}/{total})", self.current + 1),
                };
                match self.timeout_left(now) {
                    Some(left) => format!(
                        "Draining {node}{position} · Timeout in {}:{:02}",
                        left.as_secs() / 60,
                        left.as_secs() % 60
                    ),
                    None => format!("Draining {node}{position} · Reading pods…"),
                }
            }
            Some(RunEnd::Cancelled) => {
                let (gone, total) = self.progress();
                let node = self.current_node().unwrap_or_default();
                format!(
                    "Cancelled · {gone} of {total} evicted on {node}{}",
                    cordoned()
                )
            }
            Some(RunEnd::Stopped(text)) => format!("Stopped: {text}{}", cordoned()),
            Some(RunEnd::Finished) => {
                let stuck = self.nodes.iter().find_map(|node| match &node.end {
                    Some(NodeEnd::Stuck(reason)) => Some((node, reason)),
                    _ => None,
                });
                match stuck {
                    Some((node, reason)) => {
                        return StatusLine {
                            lead: format!("Stuck on {}: {reason}", node.name),
                            blockers: self.blocking_budgets(node),
                            tail: format!("{}{}", self.replacement_text(now), cordoned()),
                        };
                    }
                    None => format!("Drained{}", self.replacement_text(now)),
                }
            }
        };
        StatusLine {
            lead,
            blockers: Vec::new(),
            tail: String::new(),
        }
    }

    /// ` · 2 pending replacements` or ` · checking replacements`, or nothing: what the tab says about
    /// the pods the drain evicted, after the nodes ended.
    fn replacement_text(&self, now: Duration) -> String {
        match self.pending_replacements() {
            0 if self.is_following(now) => " · checking replacements".to_owned(),
            0 => String::new(),
            1 => " · 1 pending replacement".to_owned(),
            count => format!(" · {count} pending replacements"),
        }
    }

    /// The notification at the end of the run.
    pub(crate) fn end_notice(&self) -> Option<String> {
        let names: Vec<&str> = self.nodes.iter().map(|node| node.name.as_str()).collect();
        Some(match self.end.as_ref()? {
            RunEnd::Cancelled => "Drain cancelled".to_owned(),
            RunEnd::Stopped(text) => format!("Drain stopped: {text}"),
            RunEnd::Finished => {
                let stuck = self.nodes.iter().find_map(|node| match &node.end {
                    Some(NodeEnd::Stuck(reason)) => Some((&node.name, reason)),
                    _ => None,
                });
                match (stuck, names.as_slice()) {
                    (Some((node, reason)), _) => {
                        format!("Drain stopped: {node} stuck ({reason})")
                    }
                    (None, [only]) => format!("Drain: {only} drained"),
                    (None, nodes) => format!("Drain: {} nodes drained", nodes.len()),
                }
            }
        })
    }
}

impl NodeRun {
    /// Pods that are gone, and pods the drain had to evict or wait for.
    fn progress(&self) -> (usize, usize) {
        let counted = |pod: &&PodRun| !matches!(pod.progress, PodProgress::Skipped(_));
        let total = self.pods.iter().filter(counted).count();
        let gone = self
            .pods
            .iter()
            .filter(|pod| {
                matches!(
                    pod.progress,
                    PodProgress::Gone | PodProgress::Recreated { .. }
                )
            })
            .count();
        (gone, total)
    }
}

/// The state text of a pod and its tone. Once the run `is_ended` nothing retries or waits any more,
/// so an open pod reads as where it was left, not as work in progress.
fn pod_text(
    progress: &PodProgress,
    now: Duration,
    budgets: BudgetPolicy,
    is_ended: bool,
) -> (String, StatusTone) {
    let (verb, past) = match budgets {
        BudgetPolicy::Respect => ("Evicting…", "Eviction sent, still on node"),
        BudgetPolicy::Skip => ("Deleting…", "Delete sent, still on node"),
    };
    match progress {
        PodProgress::Pending if is_ended => ("Not evicted".to_owned(), StatusTone::Done),
        PodProgress::Pending => ("Waiting".to_owned(), StatusTone::Done),
        PodProgress::Refused { message, .. } if is_ended => (
            match (budgets, BudgetRefusal::parse(message)) {
                (BudgetPolicy::Respect, Some(refusal)) => {
                    format!("Blocked by PDB {}", refusal.name)
                }
                (BudgetPolicy::Respect, None) => format!("Blocked by PDB: {message}"),
                (BudgetPolicy::Skip, _) => format!("Refused: {message}"),
            },
            StatusTone::Warn,
        ),
        PodProgress::Evicted if is_ended => (past.to_owned(), StatusTone::Warn),
        PodProgress::Refused {
            attempt,
            retry_at,
            message,
        } => {
            let seconds = retry_at.saturating_sub(now).as_secs();
            // A delete asks no budget: its refusal is the API's own rate limiting.
            let by = match budgets {
                BudgetPolicy::Respect => " by PDB",
                BudgetPolicy::Skip => "",
            };
            (
                format!("Refused{by}: {message} · retry in {seconds} s (attempt {attempt})"),
                StatusTone::Warn,
            )
        }
        PodProgress::Evicted => (verb.to_owned(), StatusTone::Info),
        PodProgress::Awaited => ("Terminating".to_owned(), StatusTone::Info),
        PodProgress::Gone => ("Gone".to_owned(), StatusTone::Ok),
        PodProgress::Recreated { reason } => {
            (format!("recreated · Pending: {reason}"), StatusTone::Warn)
        }
        PodProgress::Failed(error) => (format!("Failed: {error}"), StatusTone::Bad),
        PodProgress::Skipped(reason) => (
            format!(
                "Skipped: {}",
                match reason {
                    SkipReason::DaemonSet => "DaemonSet pod",
                    SkipReason::Mirror => "static pod",
                }
            ),
            StatusTone::Done,
        ),
    }
}

#[cfg(test)]
#[path = "drain_run_tests.rs"]
mod drain_run_tests;
