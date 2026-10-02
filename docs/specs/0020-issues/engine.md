# 0020 · Engine: model, pipeline, board

[Back to index](README.md) · Step 1a (step 2 adds `objects`) · Modules: `issue.rs` (types), `issue_board.rs` (+ `issue_board_tests.rs`). Pure: no GPUI type except `SharedString` and `StatusTone`, no `tracing::` (event messages are arbitrary text).

## Model (`issue.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum IssueSeverity { Critical, Warning }           // Ord: Critical first
impl IssueSeverity { pub(crate) fn tone(self) -> StatusTone; pub(crate) fn label(self) -> &'static str; }

/// Rule ids, declared in evaluation order (kind-rules.md "Order"); `Ord` is the dedupe order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum IssueRule { /* PodImage, PodCrash, …, EventBurst */ }

/// An object as the engine names it. Kind is the API kind ("Pod", "Deployment"); it is text, not a
/// `ResourceKind`, because an owner or an event can name a kind without a screen (`target` is then `None`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct IssueObject { pub(crate) kind: String, pub(crate) namespace: Option<String>, pub(crate) name: String }

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct IssueKey { pub(crate) rule: IssueRule, pub(crate) object: IssueObject } // group: object = workload

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IssueAction { Open, ViewLogs { container: Option<String> } }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Issue {
    pub(crate) key: IssueKey,
    pub(crate) severity: IssueSeverity,
    pub(crate) reason: SharedString,          // pill text: "CrashLoopBackOff", "Cert expiring"
    pub(crate) cause: String,                 // from the WHY rules
    pub(crate) shown: IssueObject,            // Object column: the pod, or the workload of a group of 2+
    pub(crate) container: Option<String>,     // " · container api"
    pub(crate) count: usize,                  // ≥ 1; pods in a group
    pub(crate) since: jiff::Timestamp,        // decision 21
    pub(crate) target: Option<ResourceKey>,   // reveal target; None when k8sBoard has no screen
    pub(crate) action: IssueAction,           // ViewLogs reads the pod of `shown` (a group: its representative)
}
```

`target` = `ResourceKey::of_object(kind, namespace, name)` (0006) of `shown`.

## Rule output (`issue_rules.rs`, `issue_kind_rules.rs`)

```rust
pub(crate) struct Finding { pub(crate) rule: IssueRule, pub(crate) severity: IssueSeverity,
    pub(crate) object: IssueObject, pub(crate) reason: SharedString, pub(crate) cause: String,
    pub(crate) container: Option<String>,
    pub(crate) onset: Option<jiff::Timestamp>,           // None → first-seen
    pub(crate) grace: Option<jiff::SignedDuration>,      // shown only once now − since ≥ grace
    pub(crate) action: IssueAction, pub(crate) workload: Option<IssueObject> /* pods only */ }
```

## Inputs and pipeline (`issue_board.rs`)

```rust
pub(crate) struct IssueInputs<'a> {
    pub(crate) pods: Option<&'a [PodSummary]>, pub(crate) nodes: Option<&'a [NodeSummary]>,
    pub(crate) namespaces: Option<&'a [NamespaceSummary]>,
    pub(crate) events: Option<&'a WarningEvents>,                  // feeds.md
    pub(crate) objects: &'a [(ResourceKind, &'a [KindObject])],   // Ready condition feeds (step 2)
    pub(crate) pod_usage: Option<&'a PodUsageHistory>, pub(crate) node_usage: Option<&'a NodeUsageHistory>,
    pub(crate) kubelet: Option<&'a KubeletHistory>,
    pub(crate) is_job_feed_live: bool,                             // rules.md "Pods"
    pub(crate) now: jiff::Timestamp,
}
pub(crate) fn evaluate(inputs: &IssueInputs) -> Vec<Finding>;         // every rule, rule order
struct Grouped { finding: Finding, count: usize, members: Vec<IssueObject> } // members never reach `Issue`
fn group_findings(findings: Vec<Finding>) -> Vec<Grouped>;             // decision 17
fn dedupe(groups: Vec<Grouped>) -> Vec<Grouped>;                       // decisions 16, 18
```

1. **evaluate**: per pod the first matching pod rule ([rules.md](rules.md)); per node; per Ready object ([kind-rules.md](kind-rules.md)); per usage sample; then events. `None` input = feed not Ready → its rules are skipped (coverage, [feeds.md](feeds.md)).
2. **group**: key `(rule, workload)` for findings with `workload: Some`. Representative = earliest `onset` (None last), then name. `shown` = the workload when the group has 2+ members **and** its kind has a screen, else the representative pod. `count` = members; severity = the most severe (smallest `Ord`); cause, container, action, grace = the representative's.
3. **dedupe**: walk in rule order with a `HashSet<IssueObject>` of taken objects (group object, `shown`, every member). A later finding whose normalized object (ReplicaSet `{d}-{hash}` → Deployment `d`) is taken is dropped. Event findings for a Pod/Node missing from the Ready lists are dropped. A held (grace) finding still takes its slot.
4. **since and grace**: `since = onset.unwrap_or(first_seen)`; first-seen is recorded for **every** grouped finding, held or not; findings with `now − since < grace` are held back.
5. **sort**: severity, then `since` oldest first, then `shown`.

## Board (`issue_board.rs`; owned by `ClusterSession`, decision 22)

```rust
pub(crate) struct IssueBoard {
    issues: Vec<Issue>, coverage: Coverage,
    first_seen: HashMap<IssueKey, jiff::Timestamp>,   // the current keys (held ones included), plus those of a rule whose feed input is `None` this run (a reload keeps its ages)
    is_dirty: bool, last_run: Option<jiff::Timestamp>,
}
impl IssueBoard {
    pub(crate) fn mark_dirty(&mut self);
    pub(crate) fn run_due(&self, now: jiff::Timestamp) -> Option<RunReason>;   // Dirty, or TimeRefresh after 30 s
    /// Runs the pipeline; `IssueChange` is `Unchanged`, `TextOnly` (only cause texts, where an age moved: repaint only while Issues is visible), or `Shape`.
    pub(crate) fn refresh(&mut self, inputs: &IssueInputs, coverage: Coverage) -> bool;
    pub(crate) fn issues(&self) -> &[Issue];                     // 0021 reads the top ones
    pub(crate) fn coverage(&self) -> &Coverage;
    pub(crate) fn summary(&self) -> Option<IssueSummary>;        // None until pods and nodes Ready
    pub(crate) fn count_for(&self, screen: Screen) -> Option<(usize, IssueSeverity)>; // sidebar
}
pub(crate) enum RunReason { Dirty, TimeRefresh }
pub(crate) struct IssueSummary { pub(crate) total: usize, pub(crate) critical: usize, pub(crate) is_partial: bool }
```

`ClusterSession` holds `issues: IssueBoard` and the tick task; a context switch builds a new session, so the board starts empty. `count_for` counts issues whose `target.screen()` equals `screen`, with the worst severity.

## Constants (`issue_rules.rs` unless noted)

| Name | Value | Use |
|---|---|---|
| `ISSUE_TICK`, `TIME_REFRESH` (`issue_board.rs`) | 1 s, 30 s | evaluation throttle, time-based rules |
| `UNSCHEDULABLE_GRACE` | 2 min | PodUnschedulable |
| `NOT_READY_GRACE`, `ROLLOUT_GRACE` | 5 min | PodNotReady; DaemonSet and J4 kind findings |
| `STARTUP_FALLBACK_GRACE`, `STUCK_STARTING_AFTER` | 10 min | PodStartup without a probe period; PodStuck |
| `NODE_UNKNOWN_GRACE` | 1 min | NodeNotReady with status Unknown |
| `RESTART_WINDOW`, `RESTART_WARN` | 1 h, 3 | PodRestarts |
| `EVICTION_WINDOW` | 24 h | PodFailed (Evicted) |
| `EVENT_WINDOW`, `WARNING_BURST` | 15 min, 10 | event rules |
| `PVC_PENDING_GRACE` | 5 min | PvcPending |
| `USAGE_WARN_RATIO` | 0.9 (= `usage_tone` Bad) | usage rules |

Reused: `EXPIRY_WARNING` 14 d (0016), `STUCK_AFTER` 5 min (0018), `quota_tone` 90 % (0013).
