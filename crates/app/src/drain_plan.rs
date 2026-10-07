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

/// Whether the drain asks the eviction API, which checks budgets, or deletes pods directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BudgetPolicy {
    #[default]
    Respect,
    /// kubectl `--disable-eviction` (spec 0040).
    Skip,
}

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
    /// Never remembered: the dialog builds a fresh default on every open.
    pub(crate) budgets: BudgetPolicy,
}

impl Default for DrainOptions {
    fn default() -> Self {
        Self {
            ignore_daemon_sets: true,
            delete_empty_dir: false,
            force_unmanaged: false,
            grace: GracePeriod::PodDefault,
            timeout: DEFAULT_TIMEOUT,
            budgets: BudgetPolicy::Respect,
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
        /// What the budget demands against what is healthy: `minAvailable 2 = 2 healthy`.
        rule: String,
    },
    /// `BudgetPolicy::Skip`: the budgets that would have been checked, by name.
    Bypassed {
        names: Vec<String>,
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

/// What the budget demands against what is healthy: `minAvailable 2 = 2 healthy`.
fn budget_rule(budget: &PodDisruptionBudgetSummary) -> String {
    let healthy = budget.current_healthy;
    match (&budget.min_available, &budget.max_unavailable) {
        (Some(min), _) => format!("minAvailable {min} = {healthy} healthy"),
        (None, Some(max)) => format!("maxUnavailable {max}, {healthy} healthy"),
        (None, None) => format!("{healthy} healthy"),
    }
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
    // Deleting directly never asks a budget, so even a pod two budgets match is deleted.
    if options.budgets == BudgetPolicy::Skip {
        let mut names: Vec<String> = matching.iter().map(|budget| budget.name.clone()).collect();
        names.sort();
        return PodVerdict::Evict(if names.is_empty() {
            Budget::None
        } else {
            Budget::Bypassed { names }
        });
    }
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
        DisruptionState::Blocked(cause) => Budget::Blocked {
            rule: budget_rule(budget),
            name,
            cause,
        },
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

impl PlannedPod {
    /// The claim whose volume keeps the pod on this node, when the drain would evict it. A finished
    /// pod holds no volume that matters.
    pub(crate) fn pinned_volume(&self) -> Option<&str> {
        let is_evicted = matches!(self.verdict, PodVerdict::Evict(_));
        self.pod
            .pinned_volume
            .as_deref()
            .filter(|_| is_evicted && !self.pod.is_finished)
    }
}

/// Every pod of one node with what a drain does with it, in the order the node listed them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodePlan {
    pub(crate) node: String,
    pub(crate) pods: Vec<PlannedPod>,
    /// The policy the plan was made under: it words the preview.
    pub(crate) budgets: BudgetPolicy,
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
        budgets: options.budgets,
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
    /// A preview sends nothing, so nothing was asked.
    NotChecked,
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
        /// The server's own words behind a refusal or failure, for the tooltip of a cell that
        /// shows a short form of them or is cut.
        detail: Option<SharedString>,
    },
    /// Pods the drain leaves alone, grouped by reason: `6 DaemonSet pods`.
    Skipped { text: SharedString },
}

/// The budget a refusal names. The API words it
/// `The disruption budget api-pdb needs 2 healthy pods and has 2 currently`.
pub(crate) struct BudgetRefusal<'a> {
    pub(crate) name: &'a str,
    /// Healthy pods now, and healthy pods the budget needs.
    healthy: Option<(u32, u32)>,
}

impl<'a> BudgetRefusal<'a> {
    pub(crate) fn parse(message: &'a str) -> Option<Self> {
        let (name, rest) = message
            .strip_prefix("The disruption budget ")?
            .split_once(' ')?;
        let number_after = |word: &str| -> Option<u32> {
            rest.split_once(word)?
                .1
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        };
        let healthy = number_after("has ").zip(number_after("needs "));
        Some(Self { name, healthy })
    }

    /// `PDB api-pdb: 0 allowed (2/2 healthy)`: a refusal means the budget allows no disruption.
    pub(crate) fn summary(&self) -> String {
        match self.healthy {
            Some((has, needs)) => format!("PDB {}: 0 allowed ({has}/{needs} healthy)", self.name),
            None => format!("PDB {}: 0 allowed", self.name),
        }
    }
}

/// `api-pdb`, or `api-pdb and 2 more`.
fn budget_names(names: &[String]) -> String {
    match names.split_first() {
        None => String::new(),
        Some((only, [])) => only.clone(),
        Some((first, more)) => format!("{first} and {} more", more.len()),
    }
}

/// The result column of a pod before any server answer, and its tone.
fn local_result(planned: &PlannedPod) -> (String, StatusTone) {
    // A blocked budget is the sharper news; every other pod that cannot move says so.
    let is_blocked = matches!(planned.verdict, PodVerdict::Evict(Budget::Blocked { .. }));
    if let Some(claim) = planned.pinned_volume().filter(|_| !is_blocked) {
        return (
            format!("cannot move: volume {claim} lives on this node"),
            StatusTone::Warn,
        );
    }
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
        PodVerdict::Evict(Budget::Bypassed { names }) => (
            format!("Deleted directly; PDB {} not checked", budget_names(names)),
            StatusTone::Warn,
        ),
        // A finished pod is deleted at once, whatever owns it.
        PodVerdict::Evict(Budget::None) if planned.pod.is_finished => {
            ("Finished · removed".to_owned(), StatusTone::Done)
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

/// What a preview says of a pod whose plan holds no warning.
const NOT_CHECKED_TEXT: &str = "Not checked (preview)";

/// The text and tone of a pod's result with its dry-run: a server refusal or failure replaces the
/// local guess, and an accepted dry-run downgrades a local `Blocked` or `Waits` to `Dry-run
/// accepted` (the server would evict the pod now).
pub(crate) fn pod_result(
    planned: &PlannedPod,
    check: &PodCheck,
    budgets: BudgetPolicy,
) -> (SharedString, StatusTone) {
    match check {
        // A delete asks no budget, so a 429 of one is the API's own rate limiting.
        PodCheck::Refused(message) if budgets == BudgetPolicy::Skip => {
            return (format!("Refused: {message}").into(), StatusTone::Bad);
        }
        PodCheck::Refused(message) => {
            let text = BudgetRefusal::parse(message).map_or_else(
                || format!("Blocked by PDB: {message}"),
                |refusal| refusal.summary(),
            );
            return (text.into(), StatusTone::Bad);
        }
        PodCheck::Failed(error) => return (format!("Failed: {error}").into(), StatusTone::Bad),
        PodCheck::Accepted
            if planned.pinned_volume().is_none()
                && matches!(
                    planned.verdict,
                    PodVerdict::Evict(Budget::Blocked { .. } | Budget::Waits { .. })
                ) =>
        {
            return ("Dry-run accepted".into(), StatusTone::Ok);
        }
        PodCheck::Accepted | PodCheck::Waiting | PodCheck::Running | PodCheck::NotChecked => {}
    }
    let (text, tone) = local_result(planned);
    // The plan's own findings (a budget at 0, a missing option) stand; a guess that the pod will
    // go quietly is not claimed without a dry-run.
    if *check == PodCheck::NotChecked && !matches!(tone, StatusTone::Bad | StatusTone::Warn) {
        return (NOT_CHECKED_TEXT.into(), StatusTone::Info);
    }
    (text.into(), tone)
}

/// Where a pod sorts in the preview: blocked pods float to the top (W6 note 3). The server answer
/// moves a pod too.
fn preview_rank(planned: &PlannedPod, check: &PodCheck) -> u8 {
    if matches!(check, PodCheck::Refused(_) | PodCheck::Failed(_)) {
        return 1;
    }
    match &planned.verdict {
        PodVerdict::Evict(budget)
            if planned.pinned_volume().is_some() && !matches!(budget, Budget::Blocked { .. }) =>
        {
            3
        }
        PodVerdict::Refused(_) => 0,
        PodVerdict::Evict(Budget::Blocked { .. }) if *check == PodCheck::Accepted => 5,
        PodVerdict::Evict(Budget::Waits { .. }) if *check == PodCheck::Accepted => 5,
        PodVerdict::Evict(Budget::Blocked { .. }) => 1,
        PodVerdict::Needs(_) => 2,
        PodVerdict::Evict(Budget::Waits { .. } | Budget::Bypassed { .. }) => 3,
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
            let (result, tone) = pod_result(planned, &check, plan.budgets);
            let detail = match &check {
                PodCheck::Refused(message) | PodCheck::Failed(message) => Some(message.clone()),
                _ => None,
            };
            lines.push(PreviewLine::Pod {
                namespace: planned.pod.namespace.clone().into(),
                name: planned.pod.name.clone().into(),
                result,
                tone,
                detail,
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

/// The pods a drain moves and the pods it cannot, over every node. A finished pod is neither: it
/// is removed, not moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EvictionCounts {
    pub(crate) movable: usize,
    /// Evicted, but their volume lives on the node: the replacement stays Pending.
    pub(crate) pinned: usize,
}

pub(crate) fn eviction_counts(plans: &[NodePlan]) -> EvictionCounts {
    let mut counts = EvictionCounts {
        movable: 0,
        pinned: 0,
    };
    for planned in plans.iter().flat_map(|plan| plan.evictions()) {
        if planned.pod.is_finished {
            continue;
        }
        if planned.pinned_volume().is_some() {
            counts.pinned += 1;
        } else {
            counts.movable += 1;
        }
    }
    counts
}

/// ` · 1 cannot move` after a count, when some pods cannot move.
fn pinned_suffix(counts: EvictionCounts) -> String {
    match counts.pinned {
        0 => String::new(),
        pinned => format!(" · {pinned} cannot move"),
    }
}

/// Step 2 of the strip: `Evict 3 pods · 1 cannot move`.
pub(crate) fn step_text(plans: &[NodePlan], budgets: BudgetPolicy) -> String {
    let verb = match budgets {
        BudgetPolicy::Respect => "Evict",
        BudgetPolicy::Skip => "Delete",
    };
    let counts = eviction_counts(plans);
    format!(
        "{verb} {}{}",
        pod_count(counts.movable),
        pinned_suffix(counts)
    )
}

/// The header of the preview list: `Pods to evict · 3 · 1 cannot move`.
pub(crate) fn preview_header(plans: &[NodePlan], budgets: BudgetPolicy) -> String {
    let lead = match budgets {
        BudgetPolicy::Respect => "Pods to evict",
        BudgetPolicy::Skip => "Pods to delete",
    };
    let counts = eviction_counts(plans);
    format!("{lead} · {}{}", counts.movable, pinned_suffix(counts))
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

/// HEADS UP about budgets, one line each, empty when no budget stands in the way. A budget that
/// needs every pod it has (all healthy at its minimum) never lets the drain through, so it says
/// so and names the ways out; a budget that merely leaves too little room waits for a replacement.
/// `api-pdb and 2 more` for several budgets.
pub(crate) fn heads_up(plans: &[NodePlan], timeout: Duration) -> Vec<String> {
    let mut stuck: Vec<(&str, &str)> = Vec::new();
    let mut waiting: Vec<String> = Vec::new();
    for planned in plans.iter().flat_map(|plan| &plan.pods) {
        match &planned.verdict {
            PodVerdict::Evict(Budget::Blocked {
                name,
                cause: BlockCause::NoRoom,
                rule,
            }) => {
                if !stuck.iter().any(|(known, _)| known == name) {
                    stuck.push((name, rule));
                }
            }
            PodVerdict::Evict(Budget::Blocked { name, .. } | Budget::Waits { name, .. })
                if !waiting.contains(name) =>
            {
                waiting.push(name.clone());
            }
            _ => {}
        }
    }
    let timeout = timeout_text(timeout);
    let mut lines = Vec::new();
    match stuck.as_slice() {
        [] => {}
        [(name, rule)] => lines.push(format!(
            "{name} cannot lose a pod ({rule}); this drain will stop at the {timeout} timeout. Options: Skip PDBs, scale {name}, Cordon only"
        )),
        [(name, rule), more @ ..] => lines.push(format!(
            "{name} ({rule}) and {} more cannot lose a pod; this drain will stop at the {timeout} timeout. Options: Skip PDBs, scale the workloads, Cordon only",
            more.len()
        )),
    }
    if let Some((first, more)) = waiting.split_first() {
        let subject = match more.len() {
            0 => first.clone(),
            count => format!("{first} and {count} more"),
        };
        lines.push(format!(
            "Drain will wait on {subject} until a replacement pod is ready elsewhere, or stop at the {timeout} timeout."
        ));
    }
    lines
}

/// HEADS UP about pods whose volume lives on the node: the drain evicts them, and the replacement
/// stays Pending until the node is schedulable again. `None` when no pod is pinned.
pub(crate) fn pinned_note(plans: &[NodePlan]) -> Option<String> {
    let pinned: Vec<(&PlannedPod, &str)> = plans
        .iter()
        .flat_map(|plan| &plan.pods)
        .filter_map(|planned| Some((planned, planned.pinned_volume()?)))
        .collect();
    let tail = "so its replacement stays Pending until the node is back";
    Some(match pinned.as_slice() {
        [] => return None,
        [(planned, claim)] => format!(
            "{} cannot move: its volume {claim} lives on this node, {tail}.",
            planned.pod.name
        ),
        [(first, _), more @ ..] => format!(
            "{} and {} more cannot move: their volumes live on this node, so their replacements stay Pending until the node is back.",
            first.pod.name,
            more.len()
        ),
    })
}

/// The danger note of a drain that skips budgets: which budgets lose their say and how many pods
/// they protected. `None` when no pod of the plan is protected by one.
pub(crate) fn bypass_note(plans: &[NodePlan]) -> Option<String> {
    let mut names: Vec<&String> = Vec::new();
    let mut count = 0;
    for planned in plans.iter().flat_map(|plan| &plan.pods) {
        if let PodVerdict::Evict(Budget::Bypassed { names: bypassed }) = &planned.verdict {
            count += 1;
            for name in bypassed {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    if count == 0 {
        return None;
    }
    names.sort();
    let shown: Vec<&str> = names.iter().take(3).map(|name| name.as_str()).collect();
    let rest = names.len() - shown.len();
    let mut list = shown.join(", ");
    if rest > 0 {
        list.push_str(&format!(" and {rest} more"));
    }
    let verb = if count == 1 { "goes" } else { "go" };
    Some(format!(
        "PodDisruptionBudgets are not checked. {} protected by {list} {verb} down without waiting for replacements.",
        pod_count(count)
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

/// Whether the server refused every eviction it was asked about and accepted none: the drain
/// would only wait for the timeout, so the dry-run line is a warning, not a pass.
pub(crate) fn dry_run_refused_all(pods: &[PodCheck]) -> bool {
    let is_refused = |check: &PodCheck| matches!(check, PodCheck::Refused(_));
    pods.iter().any(is_refused) && !pods.contains(&PodCheck::Accepted)
}

/// The dry-run line of the dialog: `Server dry-run: cordon passed · 21 of 23 evictions accepted,
/// 2 refused by PDB`, or `… 23 of 24 deletes accepted` when the budgets are skipped.
pub(crate) fn dry_run_text(
    state: &DryRunState,
    cordons: &[CordonCheck],
    pods: &[PodCheck],
    budgets: BudgetPolicy,
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
        DryRunState::Failed(text) | DryRunState::Refused(text) => text.to_string(),
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
                let (noun, by) = match budgets {
                    BudgetPolicy::Respect => ("evictions", " by PDB"),
                    BudgetPolicy::Skip => ("deletes", ""),
                };
                let mut evictions = format!("{accepted} of {} {noun} accepted", pods.len());
                if refused > 0 {
                    evictions.push_str(&format!(", {refused} refused{by}"));
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
