//! The Issues engine: runs every rule over the live lists, groups pods that share a problem,
//! keeps one issue per object, holds back transient states, and orders the result. Pure: it takes
//! plain data and a clock, never a GPUI context. Event messages are arbitrary text, so nothing
//! here logs them.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use cluster::{NamespaceScope, NamespaceSummary, NodeSummary, PodSummary};
use jiff::{SignedDuration, Timestamp};

use crate::app_shell::Screen;
use crate::issue::{Finding, Issue, IssueKey, IssueObject, IssueRule, IssueSeverity};
use crate::issue_feeds::{Coverage, IssueFeed, WarningEvents};
use crate::issue_kind_rules::condition_findings;
use crate::issue_rules::{event_findings, node_finding, pod_finding, volume_findings};
use crate::kind_row::KindObject;
use crate::kubelet_history::KubeletHistory;
use crate::metrics_history::{NodeUsageHistory, PodUsageHistory};
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

/// How often the session looks whether the board needs a run.
pub(crate) const ISSUE_TICK: Duration = Duration::from_secs(1);
/// Rules that read the clock (ages, grace) run at least this often while anything is shown.
const TIME_REFRESH: SignedDuration = SignedDuration::from_secs(30);

/// What the rules read. `None` means the feed has not loaded, so its rules are skipped and the
/// coverage says so.
pub(crate) struct IssueInputs<'a> {
    pub(crate) pods: Option<&'a [PodSummary]>,
    pub(crate) nodes: Option<&'a [NodeSummary]>,
    /// The picked namespaces; a stuck namespace outside them is not listed.
    pub(crate) scope: &'a NamespaceScope,
    pub(crate) namespaces: Option<&'a [NamespaceSummary]>,
    pub(crate) events: Option<&'a WarningEvents>,
    /// The Ready condition feeds, as the 0012 summary objects; a feed that is not here has not
    /// loaded.
    pub(crate) objects: &'a [(ResourceKind, &'a [KindObject])],
    pub(crate) pod_usage: Option<&'a PodUsageHistory>,
    pub(crate) node_usage: Option<&'a NodeUsageHistory>,
    pub(crate) kubelet: Option<&'a KubeletHistory>,
    /// The Jobs feed is live, so the Job rules own the pods of a Job.
    pub(crate) is_job_feed_live: bool,
    pub(crate) now: Timestamp,
}

/// Every rule over every object, in rule order.
pub(crate) fn evaluate(inputs: &IssueInputs) -> Vec<Finding> {
    let mut findings: Vec<Finding> = inputs
        .pods
        .into_iter()
        .flatten()
        .filter_map(|pod| pod_finding(pod, inputs))
        .collect();
    findings.extend(
        inputs
            .nodes
            .into_iter()
            .flatten()
            .filter_map(|node| node_finding(node, inputs)),
    );
    findings.extend(condition_findings(inputs));
    findings.extend(volume_findings(inputs));
    findings.extend(event_findings(inputs));
    findings
}

/// One or more findings of the same rule and workload, as one issue.
struct Grouped {
    key: IssueKey,
    shown: IssueObject,
    /// The representative's finding, with the most severe severity of the group.
    finding: Finding,
    count: usize,
    /// Every object of the group; they never reach `Issue`, only the dedupe.
    members: Vec<IssueObject>,
}

/// The earlier onset first, an unknown onset last.
fn compare_onset(left: Option<Timestamp>, right: Option<Timestamp>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Pod findings with the same rule and workload become one issue with a count: the table lists
/// four rows, not forty pods, and the workload's drawer lists the rest.
fn group_findings(findings: Vec<Finding>) -> Vec<Grouped> {
    let mut groups: Vec<(IssueKey, Vec<Finding>)> = Vec::new();
    let mut positions: HashMap<IssueKey, usize> = HashMap::new();
    for finding in findings {
        let key = IssueKey {
            rule: finding.rule,
            object: finding
                .workload
                .clone()
                .unwrap_or_else(|| finding.object.clone()),
        };
        match positions.get(&key) {
            Some(position) => groups[*position].1.push(finding),
            None => {
                positions.insert(key.clone(), groups.len());
                groups.push((key, vec![finding]));
            }
        }
    }
    groups
        .into_iter()
        .filter_map(|(key, members)| grouped(key, members))
        .collect()
}

fn grouped(key: IssueKey, members: Vec<Finding>) -> Option<Grouped> {
    let count = members.len();
    let severity = members.iter().map(|finding| finding.severity).min()?;
    let member_objects: Vec<IssueObject> = members
        .iter()
        .map(|finding| finding.object.clone())
        .collect();
    let mut representative = members.into_iter().min_by(|left, right| {
        compare_onset(left.onset, right.onset).then_with(|| left.object.cmp(&right.object))
    })?;
    let workload = representative
        .workload
        .clone()
        .filter(|workload| count >= 2 && workload.target().is_some());
    let shown = workload.unwrap_or_else(|| representative.object.clone());
    representative.severity = severity;
    Some(Grouped {
        key,
        shown,
        finding: representative,
        count,
        members: member_objects,
    })
}

/// Walks the groups in rule order and drops one whose workload, shown object, or any member an
/// earlier group already claimed: one row per object, the most specific cause wins.
fn dedupe(mut groups: Vec<Grouped>) -> Vec<Grouped> {
    // Stable, so groups of one rule keep their order.
    groups.sort_by_key(|group| group.key.rule);
    let mut taken: HashSet<IssueObject> = HashSet::new();
    groups
        .into_iter()
        .filter(|group| {
            let is_taken = taken.contains(&group.key.object)
                || taken.contains(&group.shown)
                || group.members.iter().any(|member| taken.contains(member));
            if !is_taken {
                taken.insert(group.key.object.clone());
                taken.insert(group.shown.clone());
                taken.extend(group.members.iter().cloned());
            }
            !is_taken
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunReason {
    /// An input changed.
    Dirty,
    /// The clock moved: ages and grace periods.
    TimeRefresh,
}

/// What a run changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IssueChange {
    Unchanged,
    /// Only the cause texts differ, where an age moved (`stuck for 11m`): the sidebar and the
    /// title bar read none of it, so only the Issues screen needs a repaint.
    TextOnly,
    /// Issues came, went, or changed, or the coverage did.
    Shape,
}

/// Whether both lists hold the same issues but for their cause texts.
fn has_same_shape(left: &[Issue], right: &[Issue]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.key == right.key
                && left.severity == right.severity
                && left.reason == right.reason
                && left.shown == right.shown
                && left.subject == right.subject
                && left.container == right.container
                && left.count == right.count
                && left.since == right.since
                && left.target == right.target
                && left.action == right.action
        })
}

/// Whether a feed the rule of `key` reads has not loaded, so the run skipped the rule.
fn is_feed_missing(key: &IssueKey, inputs: &IssueInputs) -> bool {
    let has_objects = |kind: &str| {
        let kind = ResourceKind::from_object_kind(kind);
        inputs
            .objects
            .iter()
            .any(|(listed, _)| Some(*listed) == kind)
    };
    match key.rule {
        IssueRule::PodImage
        | IssueRule::PodCrash
        | IssueRule::PodWaiting
        | IssueRule::PodUnschedulable
        | IssueRule::PodFailed
        | IssueRule::PodExited
        | IssueRule::PodStartup
        | IssueRule::PodNotReady
        | IssueRule::PodStuck
        | IssueRule::PodRestarts
        | IssueRule::PodMemory
        | IssueRule::PodCpu => inputs.pods.is_none(),
        IssueRule::NodeNotReady
        | IssueRule::NodeNetwork
        | IssueRule::NodePressure
        | IssueRule::NodeCondition
        | IssueRule::NodeMemory
        | IssueRule::NodeCpu => inputs.nodes.is_none(),
        IssueRule::NamespaceStuck => inputs.namespaces.is_none(),
        // The object of the key names the kind of its feed.
        IssueRule::KindRollout
        | IssueRule::KindJob
        | IssueRule::KindClaim
        | IssueRule::KindAutoscaler
        | IssueRule::KindDisruptionBudget
        | IssueRule::KindQuota
        | IssueRule::QuotaNearLimit
        | IssueRule::CertExpired
        | IssueRule::CertExpiring => !has_objects(&key.object.kind),
        IssueRule::PvcPending => !has_objects(&key.object.kind) || inputs.events.is_none(),
        IssueRule::VolumeFull => inputs.kubelet.is_none(),
        IssueRule::EventFailedCreate | IssueRule::EventJobFailed | IssueRule::EventBurst => {
            inputs.events.is_none()
        }
    }
}

/// The numbers the title bar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IssueSummary {
    pub(crate) total: usize,
    pub(crate) critical: usize,
    pub(crate) is_partial: bool,
}
impl IssueSummary {
    /// The severity that tones the counts: Critical when any issue is.
    pub(crate) fn worst(self) -> IssueSeverity {
        if self.critical > 0 {
            IssueSeverity::Critical
        } else {
            IssueSeverity::Warning
        }
    }
}

/// The issues of one session. `ClusterSession` owns it, so a retry or a scope change keeps the
/// first-seen times and a context switch starts empty. First-seen times are memory only: they do
/// not survive an app restart.
pub(crate) struct IssueBoard {
    issues: Vec<Issue>,
    coverage: Coverage,
    /// The keys of the last run, held ones included, and those of a rule whose feed is reloading.
    first_seen: HashMap<IssueKey, Timestamp>,
    is_core_ready: bool,
    is_dirty: bool,
    last_run: Option<Timestamp>,
}

impl Default for IssueBoard {
    fn default() -> Self {
        Self {
            issues: Vec::new(),
            coverage: Coverage::default(),
            first_seen: HashMap::new(),
            is_core_ready: false,
            // Nothing has run yet.
            is_dirty: true,
            last_run: None,
        }
    }
}

impl IssueBoard {
    pub(crate) fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    /// Why the board should run now, if it should.
    pub(crate) fn run_due(&self, now: Timestamp) -> Option<RunReason> {
        if self.is_dirty {
            return Some(RunReason::Dirty);
        }
        let is_stale = self
            .last_run
            .is_none_or(|last| now.duration_since(last) >= TIME_REFRESH);
        is_stale.then_some(RunReason::TimeRefresh)
    }

    /// Runs the pipeline and says what changed.
    pub(crate) fn refresh(&mut self, inputs: &IssueInputs, coverage: Coverage) -> IssueChange {
        let now = inputs.now;
        let groups = dedupe(group_findings(evaluate(inputs)));
        let mut first_seen = HashMap::with_capacity(groups.len());
        let mut issues = Vec::with_capacity(groups.len());
        for group in groups {
            let seen = self.first_seen.get(&group.key).copied().unwrap_or(now);
            first_seen.insert(group.key.clone(), seen);
            let since = group.finding.onset.unwrap_or(seen);
            let is_held = group
                .finding
                .grace
                .is_some_and(|grace| now.duration_since(since) < grace);
            if !is_held {
                issues.push(issue_of(group, since));
            }
        }
        // A feed that reloads (a scope change, a retry) skips its rules for a while; what they
        // saw before keeps its first-seen time, or every age would start again.
        for (key, seen) in &self.first_seen {
            if is_feed_missing(key, inputs) {
                first_seen.entry(key.clone()).or_insert(*seen);
            }
        }
        issues.sort_by(|left, right| {
            (left.severity, left.since, &left.shown).cmp(&(
                right.severity,
                right.since,
                &right.shown,
            ))
        });
        let is_core_ready =
            coverage.is_settled(IssueFeed::Pods) && coverage.is_settled(IssueFeed::Nodes);
        let change = if coverage != self.coverage
            || is_core_ready != self.is_core_ready
            || !has_same_shape(&issues, &self.issues)
        {
            IssueChange::Shape
        } else if issues != self.issues {
            IssueChange::TextOnly
        } else {
            IssueChange::Unchanged
        };
        self.issues = issues;
        self.coverage = coverage;
        self.first_seen = first_seen;
        self.is_core_ready = is_core_ready;
        self.is_dirty = false;
        self.last_run = Some(now);
        change
    }

    /// Sorted by severity, then oldest first.
    pub(crate) fn issues(&self) -> &[Issue] {
        &self.issues
    }

    pub(crate) fn coverage(&self) -> &Coverage {
        &self.coverage
    }

    /// `None` until pods and nodes have loaded: a count of zero would be a guess.
    pub(crate) fn summary(&self) -> Option<IssueSummary> {
        self.is_core_ready.then(|| IssueSummary {
            total: self.issues.len(),
            critical: self
                .issues
                .iter()
                .filter(|issue| issue.severity == IssueSeverity::Critical)
                .count(),
            is_partial: self.coverage.is_partial(),
        })
    }

    /// The issues whose row `screen` lists, with the worst severity among them. `None` when there
    /// are none, or before the first run.
    pub(crate) fn count_for(&self, screen: Screen) -> Option<(usize, IssueSeverity)> {
        if !self.is_core_ready {
            return None;
        }
        let severities = self
            .issues
            .iter()
            .filter(|issue| issue.target.as_ref().map(ResourceKey::screen) == Some(screen))
            .map(|issue| issue.severity);
        let (count, worst) = severities.fold((0, None), |(count, worst), severity| {
            (
                count + 1,
                Some(worst.map_or(severity, |worst: IssueSeverity| worst.min(severity))),
            )
        });
        worst.map(|worst| (count, worst))
    }
}

fn issue_of(group: Grouped, since: Timestamp) -> Issue {
    let Grouped {
        key,
        shown,
        finding,
        count,
        ..
    } = group;
    Issue {
        key,
        severity: finding.severity,
        reason: finding.reason,
        cause: finding.cause,
        container: finding.container,
        subject: finding.object,
        count,
        since,
        target: shown.target(),
        shown,
        action: finding.action,
    }
}

#[cfg(test)]
#[path = "issue_board_tests.rs"]
mod issue_board_tests;
