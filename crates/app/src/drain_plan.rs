//! The drain plan (spec 0034 step 3a): what a drain would do with every pod of a node, as the W6
//! dialog previews it. Pure: it reads the pods and the PodDisruptionBudgets it is given and
//! decides; it sends nothing. The verdicts follow `kubectl drain`'s filters, plus the budget state
//! of 0013 (`disruption_state`).

use std::collections::HashMap;
use std::time::Duration;

use cluster::{BlockCause, DisruptionState, DrainPod, GracePeriod, PodDisruptionBudgetSummary};
use gpui_kit::SharedString;

use crate::app_shell::write_flow::DryRunState;
use crate::status_tone::StatusTone;

/// The most a drain waits for one node when the user picks nothing else.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The timeouts the dialog offers, per node.
pub(crate) const TIMEOUT_CHOICES: [Duration; 4] = [
    Duration::from_secs(2 * 60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(10 * 60),
    Duration::from_secs(30 * 60),
];
/// The grace periods the dialog offers after `Pod default`, in seconds.
pub(crate) const GRACE_CHOICES: [u32; 4] = [10, 30, 60, 120];
const DAEMON_SET: &str = "DaemonSet";

/// The kubectl flags a drain can be given, with the consequence of each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DrainOptions {
    /// `--ignore-daemonsets`
    pub(crate) ignore_daemon_sets: bool,
    /// `--delete-emptydir-data`
    pub(crate) delete_empty_dir: bool,
    /// `--force`
    pub(crate) force_unmanaged: bool,
    pub(crate) grace: GracePeriod,
    /// Per node.
    pub(crate) timeout: Duration,
}

impl Default for DrainOptions {
    fn default() -> Self {
        Self {
            ignore_daemon_sets: true,
            delete_empty_dir: false,
            force_unmanaged: false,
            grace: GracePeriod::PodDefault,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// An option a pod needs before it can be evicted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrainOption {
    IgnoreDaemonSets,
    DeleteEmptyDir,
    ForceUnmanaged,
}

impl DrainOption {
    /// The checkbox label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::IgnoreDaemonSets => "Ignore DaemonSet pods",
            Self::DeleteEmptyDir => "Delete emptyDir data",
            Self::ForceUnmanaged => "Force unmanaged pods",
        }
    }
}

/// What the eviction API will do with the pod's PodDisruptionBudget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Budget {
    /// No budget covers the pod, or the API skips it for this pod.
    None,
    Allows {
        name: String,
        allowed: u32,
    },
    /// The budget allows fewer evictions than this node has pods of it: this pod waits for a replacement.
    Waits {
        name: String,
        allowed: u32,
    },
    Blocked {
        name: String,
        cause: BlockCause,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SkipReason {
    DaemonSet,
    Mirror,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PodVerdict {
    Evict(Budget),
    /// Not evicted until the user ticks the option.
    Needs(DrainOption),
    /// The API refuses this pod for good.
    Refused(SharedString),
    /// Already going away: awaited, not evicted.
    Terminating,
    Skip(SkipReason),
}

/// The decision of rows 1 to 6 of the verdict table, which need no budget; `None` when the pod
/// reaches the budget row.
fn verdict_before_budget(pod: &DrainPod, options: &DrainOptions) -> Option<PodVerdict> {
    if pod.is_mirror {
        return Some(PodVerdict::Skip(SkipReason::Mirror));
    }
    if pod.is_terminating {
        return Some(PodVerdict::Terminating);
    }
    // Checked before DaemonSet ownership, as kubectl does: a finished DaemonSet pod is evicted.
    // The API deletes a terminal pod without a budget check.
    if pod.is_finished {
        return Some(PodVerdict::Evict(Budget::None));
    }
    let controller = pod.controller.as_ref();
    if controller.is_some_and(|controller| controller.kind == DAEMON_SET) {
        return Some(if options.ignore_daemon_sets {
            PodVerdict::Skip(SkipReason::DaemonSet)
        } else {
            PodVerdict::Needs(DrainOption::IgnoreDaemonSets)
        });
    }
    if controller.is_none() && !options.force_unmanaged {
        return Some(PodVerdict::Needs(DrainOption::ForceUnmanaged));
    }
    if pod.has_empty_dir && !options.delete_empty_dir {
        return Some(PodVerdict::Needs(DrainOption::DeleteEmptyDir));
    }
    // The eviction API skips the budget check for a pod that is not running yet.
    pod.is_pending.then_some(PodVerdict::Evict(Budget::None))
}

/// A pod as a drain names it: pinned by uid, so a pod recreated under the same name (a
/// StatefulSet's) is never evicted by a request that was meant for the old one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PodKey {
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) uid: String,
}

impl PodKey {
    pub(crate) fn of(pod: &DrainPod) -> Self {
        Self {
            namespace: pod.namespace.clone(),
            name: pod.name.clone(),
            uid: pod.uid.clone(),
        }
    }

    /// `payments/api-1`, as the tab and the audit line name the pod.
    pub(crate) fn text(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

/// What a running drain does with a pod it reads when a node starts: the rows of the verdict table
/// that need no budget (the eviction API decides the budget itself, and a refusal is retried).
pub(crate) fn run_verdict(pod: &DrainPod, options: &DrainOptions) -> PodVerdict {
    verdict_before_budget(pod, options).unwrap_or(PodVerdict::Evict(Budget::None))
}

/// The budgets of the pod's namespace whose selector matches its labels.
fn matching_budgets<'a>(
    pod: &DrainPod,
    budgets: &'a [PodDisruptionBudgetSummary],
) -> Vec<&'a PodDisruptionBudgetSummary> {
    budgets
        .iter()
        .filter(|budget| {
            budget.namespace == pod.namespace
                && budget
                    .selector
                    .as_ref()
                    .is_some_and(|selector| selector.matches(&pod.labels))
        })
        .collect()
}

/// What a drain does with `pod`; the first matching row of the table wins. `rank_in_budget` is the
/// 1-based position of the pod among the pods of the same budget on its node, in plan order: it
/// explains why the second pod of a budget that allows one waits.
pub(crate) fn pod_verdict(
    pod: &DrainPod,
    budgets: &[PodDisruptionBudgetSummary],
    options: &DrainOptions,
    rank_in_budget: u32,
) -> PodVerdict {
    if let Some(verdict) = verdict_before_budget(pod, options) {
        return verdict;
    }
    let matching = matching_budgets(pod, budgets);
    let [budget] = matching.as_slice() else {
        return match matching.len() {
            0 => PodVerdict::Evict(Budget::None),
            count => PodVerdict::Refused(
                format!("Matches {count} PDBs; the API refuses to evict it").into(),
            ),
        };
    };
    let name = budget.name.clone();
    PodVerdict::Evict(match budget.disruption_state() {
        DisruptionState::Blocked(cause) => Budget::Blocked { name, cause },
        DisruptionState::Allowed(allowed) if rank_in_budget <= allowed => {
            Budget::Allows { name, allowed }
        }
        DisruptionState::Allowed(allowed) => Budget::Waits { name, allowed },
        DisruptionState::NoPods => Budget::None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedPod {
    pub(crate) pod: DrainPod,
    pub(crate) verdict: PodVerdict,
}

/// Every pod of one node with what a drain does with it, in the order the node listed them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodePlan {
    pub(crate) node: String,
    pub(crate) pods: Vec<PlannedPod>,
}

impl NodePlan {
    /// The pods the drain will evict.
    pub(crate) fn evictions(&self) -> impl Iterator<Item = &PlannedPod> {
        self.pods
            .iter()
            .filter(|planned| matches!(planned.verdict, PodVerdict::Evict(_)))
    }
}

pub(crate) fn node_plan(
    node: &str,
    pods: &[DrainPod],
    budgets: &[PodDisruptionBudgetSummary],
    options: &DrainOptions,
) -> NodePlan {
    let mut ranks: HashMap<(&str, &str), u32> = HashMap::new();
    let planned = pods
        .iter()
        .map(|pod| {
            // Only a pod that reaches the budget row counts against its budget.
            let rank = if verdict_before_budget(pod, options).is_none() {
                match matching_budgets(pod, budgets).as_slice() {
                    [budget] => {
                        let seen = ranks.entry((&budget.namespace, &budget.name)).or_insert(0);
                        *seen += 1;
                        *seen
                    }
                    _ => 0,
                }
            } else {
                0
            };
            PlannedPod {
                pod: pod.clone(),
                verdict: pod_verdict(pod, budgets, options, rank),
            }
        })
        .collect();
    NodePlan {
        node: node.to_owned(),
        pods: planned,
    }
}

/// `1 pod` or `2 pods`.
pub(crate) fn pod_count(count: usize) -> String {
    match count {
        1 => "1 pod".to_owned(),
        count => format!("{count} pods"),
    }
}

/// Why Drain is off while Cordon only is not: a pod needs an option the user left unticked, or the
/// API refuses a pod for good.
pub(crate) fn drain_blocker(plans: &[NodePlan]) -> Option<SharedString> {
    let verdicts = || plans.iter().flat_map(|plan| &plan.pods);
    let needed = verdicts().find_map(|planned| match planned.verdict {
        PodVerdict::Needs(option) => Some(option),
        _ => None,
    });
    if let Some(option) = needed {
        let count = verdicts()
            .filter(|planned| planned.verdict == PodVerdict::Needs(option))
            .count();
        let verb = if count == 1 { "needs" } else { "need" };
        return Some(
            format!(
                "{} {verb} \"{}\": tick it, or use Cordon only",
                pod_count(count),
                option.label()
            )
            .into(),
        );
    }
    verdicts().find_map(|planned| match &planned.verdict {
        PodVerdict::Refused(text) => Some(
            format!(
                "{}/{} cannot be evicted: {text}",
                planned.pod.namespace, planned.pod.name
            )
            .into(),
        ),
        _ => None,
    })
}

/// How many pods each option affects, whatever its checkbox says. A static pod is the kubelet's and
/// never counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OptionCounts {
    pub(crate) daemon_sets: usize,
    pub(crate) empty_dir: usize,
    pub(crate) unmanaged: usize,
}

pub(crate) fn option_counts(plans: &[NodePlan]) -> OptionCounts {
    let mut counts = OptionCounts::default();
    for pod in plans.iter().flat_map(|plan| &plan.pods).map(|p| &p.pod) {
        if pod.is_mirror {
            continue;
        }
        match &pod.controller {
            Some(controller) if controller.kind == DAEMON_SET => counts.daemon_sets += 1,
            Some(_) => {}
            None => counts.unmanaged += 1,
        }
        if pod.has_empty_dir {
            counts.empty_dir += 1;
        }
    }
    counts
}

/// The small text under a checkbox.
pub(crate) fn option_hint(option: DrainOption, counts: &OptionCounts) -> String {
    // The verb agrees with the count: `1 pod stays`, `6 pods stay`.
    let (count, one, many) = match option {
        DrainOption::IgnoreDaemonSets => {
            (counts.daemon_sets, "stays on the node", "stay on the node")
        }
        DrainOption::DeleteEmptyDir => (
            counts.empty_dir,
            "loses local scratch data",
            "lose local scratch data",
        ),
        DrainOption::ForceUnmanaged => (
            counts.unmanaged,
            "has no controller and will not come back",
            "have no controller and will not come back",
        ),
    };
    let rest = if count == 1 { one } else { many };
    format!("{} {rest}", pod_count(count))
}

/// Where the dry-run of one eviction stands. The server answer is fresher than the budget status
/// in the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum PodCheck {
    #[default]
    Waiting,
    Running,
    Accepted,
    /// A 429: the budget refuses it now. An expected wait, not a failure.
    Refused(SharedString),
    Failed(SharedString),
}

/// One line of the preview list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PreviewLine {
    /// A node of a plan of several.
    Node(SharedString),
    Pod {
        namespace: SharedString,
        name: SharedString,
        result: SharedString,
        tone: StatusTone,
    },
    /// Pods the drain leaves alone, grouped by reason: `6 DaemonSet pods`.
    Skipped { text: SharedString },
}

/// The result column of a pod before any server answer, and its tone.
fn local_result(planned: &PlannedPod) -> (String, StatusTone) {
    match &planned.verdict {
        PodVerdict::Refused(text) => (text.to_string(), StatusTone::Bad),
        PodVerdict::Evict(Budget::Blocked { name, .. }) => (
            format!("Blocked by PDB {name} (0 allowed)"),
            StatusTone::Bad,
        ),
        PodVerdict::Needs(DrainOption::ForceUnmanaged) => {
            ("No controller, needs Force".to_owned(), StatusTone::Warn)
        }
        PodVerdict::Needs(DrainOption::DeleteEmptyDir) => (
            "Uses emptyDir, needs Delete emptyDir data".to_owned(),
            StatusTone::Warn,
        ),
        PodVerdict::Needs(DrainOption::IgnoreDaemonSets) => (
            "DaemonSet pod, needs Ignore DaemonSet pods".to_owned(),
            StatusTone::Warn,
        ),
        PodVerdict::Evict(Budget::Waits { name, allowed }) => (
            format!("Waits on PDB {name} (allows {allowed})"),
            StatusTone::Warn,
        ),
        PodVerdict::Evict(Budget::Allows { name, allowed }) => {
            (format!("PDB {name} allows {allowed}"), StatusTone::Ok)
        }
        PodVerdict::Evict(Budget::None) if planned.pod.controller.is_none() => {
            ("Will not come back".to_owned(), StatusTone::Warn)
        }
        PodVerdict::Evict(Budget::None) if planned.pod.has_empty_dir => {
            ("Loses local data".to_owned(), StatusTone::Warn)
        }
        PodVerdict::Evict(Budget::None) => ("Will be rescheduled".to_owned(), StatusTone::Ok),
        PodVerdict::Terminating => ("Already terminating".to_owned(), StatusTone::Done),
        PodVerdict::Skip(_) => ("Skipped".to_owned(), StatusTone::Done),
    }
}

/// The text and tone of a pod's result with its dry-run: a server refusal or failure replaces the
/// local guess, and an accepted dry-run downgrades a local `Blocked` or `Waits` to `Dry-run
/// accepted` (the server would evict the pod now).
pub(crate) fn pod_result(planned: &PlannedPod, check: &PodCheck) -> (SharedString, StatusTone) {
    match check {
        PodCheck::Refused(message) => {
            return (format!("Blocked by PDB: {message}").into(), StatusTone::Bad);
        }
        PodCheck::Failed(error) => return (format!("Failed: {error}").into(), StatusTone::Bad),
        PodCheck::Accepted
            if matches!(
                planned.verdict,
                PodVerdict::Evict(Budget::Blocked { .. } | Budget::Waits { .. })
            ) =>
        {
            return ("Dry-run accepted".into(), StatusTone::Ok);
        }
        PodCheck::Accepted | PodCheck::Waiting | PodCheck::Running => {}
    }
    let (text, tone) = local_result(planned);
    (text.into(), tone)
}

/// Where a pod sorts in the preview: blocked pods float to the top (W6 note 3). The server answer
/// moves a pod too.
fn preview_rank(planned: &PlannedPod, check: &PodCheck) -> u8 {
    if matches!(check, PodCheck::Refused(_) | PodCheck::Failed(_)) {
        return 1;
    }
    match &planned.verdict {
        PodVerdict::Refused(_) => 0,
        PodVerdict::Evict(Budget::Blocked { .. }) if *check == PodCheck::Accepted => 5,
        PodVerdict::Evict(Budget::Waits { .. }) if *check == PodCheck::Accepted => 5,
        PodVerdict::Evict(Budget::Blocked { .. }) => 1,
        PodVerdict::Needs(_) => 2,
        PodVerdict::Evict(Budget::Waits { .. }) => 3,
        PodVerdict::Evict(Budget::Allows { .. }) => 4,
        PodVerdict::Evict(_) => 5,
        PodVerdict::Terminating => 6,
        PodVerdict::Skip(_) => 7,
    }
}

/// The preview list: per node (with a header when there are several), the pods by rank, then one
/// grouped line per skip reason. `check_of` gives the dry-run state of a pod by uid.
pub(crate) fn preview_lines(
    plans: &[NodePlan],
    check_of: impl Fn(&str) -> PodCheck,
) -> Vec<PreviewLine> {
    let mut lines = Vec::new();
    for plan in plans {
        if plans.len() > 1 {
            lines.push(PreviewLine::Node(plan.node.clone().into()));
        }
        let mut shown: Vec<(u8, &PlannedPod, PodCheck)> = plan
            .pods
            .iter()
            .filter(|planned| !matches!(planned.verdict, PodVerdict::Skip(_)))
            .map(|planned| {
                let check = check_of(&planned.pod.uid);
                (preview_rank(planned, &check), planned, check)
            })
            .collect();
        // A stable sort keeps the order the node listed the pods in within a rank.
        shown.sort_by_key(|(rank, ..)| *rank);
        for (_, planned, check) in shown {
            let (result, tone) = pod_result(planned, &check);
            lines.push(PreviewLine::Pod {
                namespace: planned.pod.namespace.clone().into(),
                name: planned.pod.name.clone().into(),
                result,
                tone,
            });
        }
        let count = |reason: SkipReason| {
            plan.pods
                .iter()
                .filter(|planned| planned.verdict == PodVerdict::Skip(reason))
                .count()
        };
        for (reason, one, many) in [
            (SkipReason::DaemonSet, "DaemonSet pod", "DaemonSet pods"),
            (SkipReason::Mirror, "static pod", "static pods"),
        ] {
            match count(reason) {
                0 => {}
                1 => lines.push(PreviewLine::Skipped {
                    text: format!("1 {one} · Skipped").into(),
                }),
                total => lines.push(PreviewLine::Skipped {
                    text: format!("{total} {many} · Skipped").into(),
                }),
            }
        }
    }
    lines
}

/// The pods the plans evict, over every node.
pub(crate) fn eviction_count(plans: &[NodePlan]) -> usize {
    plans.iter().map(|plan| plan.evictions().count()).sum()
}

/// `2m`, `30m`: the timeout choice as the select shows it.
pub(crate) fn timeout_text(timeout: Duration) -> String {
    format!("{}m", timeout.as_secs() / 60)
}

/// `Pod default`, `30 s`: the grace choice as the select shows it.
pub(crate) fn grace_text(grace: GracePeriod) -> String {
    match grace {
        GracePeriod::PodDefault => "Pod default".to_owned(),
        GracePeriod::Seconds(seconds) => format!("{seconds} s"),
    }
}

/// HEADS UP: only when a pod waits on a budget. `api-pdb and 2 more` for several budgets.
pub(crate) fn heads_up(plans: &[NodePlan], timeout: Duration) -> Option<String> {
    let mut names: Vec<&str> = Vec::new();
    for planned in plans.iter().flat_map(|plan| &plan.pods) {
        if let PodVerdict::Evict(Budget::Blocked { name, .. } | Budget::Waits { name, .. }) =
            &planned.verdict
            && !names.contains(&name.as_str())
        {
            names.push(name);
        }
    }
    let (first, more) = names.split_first()?;
    let subject = match more.len() {
        0 => (*first).to_owned(),
        count => format!("{first} and {count} more"),
    };
    Some(format!(
        "Drain will wait on {subject} until a replacement pod is ready elsewhere, or stop at the {} timeout.",
        timeout_text(timeout)
    ))
}

/// Where the dry-run of a node's cordon stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum CordonCheck {
    #[default]
    Waiting,
    Running,
    Passed,
    Failed(SharedString),
}

/// The dry-run state of a whole drain for `commit_block`: `Running` until every check answered,
/// `Failed` when a cordon failed or an eviction failed with anything but a 429, else `Passed`. A
/// 429 is an expected wait, not a failure.
pub(crate) fn drain_dry_run(
    cordons: &[CordonCheck],
    pods: &[PodCheck],
    elapsed: Duration,
) -> DryRunState {
    let is_waiting = cordons
        .iter()
        .any(|check| matches!(check, CordonCheck::Waiting | CordonCheck::Running))
        || pods
            .iter()
            .any(|check| matches!(check, PodCheck::Waiting | PodCheck::Running));
    if is_waiting {
        return DryRunState::Running;
    }
    for check in cordons {
        if let CordonCheck::Failed(text) = check {
            return DryRunState::Failed(format!("Dry-run failed: cordon: {text}").into());
        }
    }
    for check in pods {
        if let PodCheck::Failed(text) = check {
            return DryRunState::Failed(format!("Dry-run failed: {text}").into());
        }
    }
    DryRunState::Passed { elapsed }
}

/// The dry-run line of the dialog: `Server dry-run: cordon passed · 21 of 23 evictions accepted,
/// 2 refused by PDB`.
pub(crate) fn dry_run_text(
    state: &DryRunState,
    cordons: &[CordonCheck],
    pods: &[PodCheck],
) -> String {
    match state {
        DryRunState::Running => {
            let answered = cordons
                .iter()
                .filter(|check| !matches!(check, CordonCheck::Waiting | CordonCheck::Running))
                .count()
                + pods
                    .iter()
                    .filter(|check| !matches!(check, PodCheck::Waiting | PodCheck::Running))
                    .count();
            format!(
                "Server dry-run… {answered} of {}",
                cordons.len() + pods.len()
            )
        }
        DryRunState::Failed(text) => text.to_string(),
        DryRunState::Rejected(reason) => {
            format!("An admission webhook does not support dry-run: {reason}")
        }
        DryRunState::NotSupported => "Dry-run not supported for this action".to_owned(),
        DryRunState::Passed { .. } => {
            let accepted = pods
                .iter()
                .filter(|check| **check == PodCheck::Accepted)
                .count();
            let refused = pods
                .iter()
                .filter(|check| matches!(check, PodCheck::Refused(_)))
                .count();
            let mut parts = Vec::new();
            if !cordons.is_empty() {
                parts.push("cordon passed".to_owned());
            }
            if !pods.is_empty() {
                let mut evictions = format!("{accepted} of {} evictions accepted", pods.len());
                if refused > 0 {
                    evictions.push_str(&format!(", {refused} refused by PDB"));
                }
                parts.push(evictions);
            }
            if parts.is_empty() {
                "Server dry-run: nothing to check".to_owned()
            } else {
                format!("Server dry-run: {}", parts.join(" · "))
            }
        }
    }
}

#[cfg(test)]
#[path = "drain_plan_tests.rs"]
mod drain_plan_tests;
