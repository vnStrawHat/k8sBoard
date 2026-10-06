//! The end of a rollout the app started (spec 0032).
//!
//! A Restart, a Roll back, and a batch Restart return once the patch is accepted, so their toast
//! only says the rollout is under way. This follows the Deployment status the session already
//! watches and replaces that toast with how the rollout ended.

use std::time::{Duration, Instant};

use cluster::DeploymentSummary;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Context, SharedString};

use super::AppShell;
use crate::cluster_registry::ClusterRef;
use crate::workload_rows::{DEADLINE_EXCEEDED, PROGRESSING};

const POLL: Duration = Duration::from_secs(1);
/// A patch reaches the status watch within moments. A rollout whose generation did not move in
/// this long was a no-op (a Roll back to the template that already runs), and its status is final.
const BEGIN_GRACE: Duration = Duration::from_secs(3);
/// How long a rollout is followed; a slower one ends as not progressing, with its counts.
const WATCH_LIMIT: Duration = Duration::from_secs(120);

/// Marks the toasts of one rollout watch: the end toast has the same id as the start toast, so it
/// replaces it.
pub(crate) struct RolloutToast;

/// How a followed rollout ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RolloutOutcome {
    Complete {
        ready: u32,
        desired: u32,
    },
    /// The controller's reason, or the counts when the limit ran out.
    Stalled(String),
    /// The Deployment never showed up in a list the session holds.
    Unseen,
}

/// The outcome of the rollout, or `None` while it goes on. `baseline_generation` is the
/// generation the watch first saw: a rollout began once it grew (or the grace passed), and its
/// status counts only after the controller observed that generation, so the status of the state
/// before the patch is never read as the end.
pub(crate) fn rollout_outcome(
    deployment: &DeploymentSummary,
    baseline_generation: i64,
    elapsed: Duration,
) -> Option<RolloutOutcome> {
    let is_begun = deployment.generation > baseline_generation || elapsed >= BEGIN_GRACE;
    let is_observed = deployment.observed_generation >= deployment.generation;
    if is_begun && is_observed {
        if let Some(reason) = stall_reason(deployment) {
            return Some(RolloutOutcome::Stalled(reason));
        }
        let is_complete = deployment.up_to_date >= deployment.desired
            && deployment.ready >= deployment.desired
            && deployment.available >= deployment.desired;
        if is_complete {
            return Some(RolloutOutcome::Complete {
                ready: deployment.ready,
                desired: deployment.desired,
            });
        }
    }
    (elapsed >= WATCH_LIMIT).then(|| RolloutOutcome::Stalled(slow_text(deployment)))
}

/// The reason of a condition that says the rollout cannot go on: the progress deadline passed, or
/// the ReplicaSet cannot create pods (a quota, an admission refusal).
fn stall_reason(deployment: &DeploymentSummary) -> Option<String> {
    deployment.conditions.iter().find_map(|condition| {
        let is_deadline = condition.name == PROGRESSING
            && !condition.is_true
            && condition.reason.as_deref() == Some(DEADLINE_EXCEEDED);
        let is_failure = condition.name == "ReplicaFailure" && condition.is_true;
        (is_deadline || is_failure).then(|| {
            condition
                .reason
                .clone()
                .unwrap_or_else(|| condition.name.clone())
        })
    })
}

/// `2/3 ready after 2 min`, with the reason of an Available condition that is false.
fn slow_text(deployment: &DeploymentSummary) -> String {
    let unavailable = deployment
        .conditions
        .iter()
        .find(|condition| condition.name == "Available" && !condition.is_true)
        .and_then(|condition| condition.reason.as_deref());
    let counts = format!(
        "{}/{} ready after {} min",
        deployment.ready,
        deployment.desired,
        WATCH_LIMIT.as_secs() / 60
    );
    match unavailable {
        Some(reason) => format!("{counts}, {reason}"),
        None => counts,
    }
}

/// The toast text of the end of the watch, and whether every rollout completed. `outcomes` pairs a
/// workload name with its outcome.
pub(crate) fn rollout_summary(outcomes: &[(String, RolloutOutcome)]) -> (String, bool) {
    let is_success = outcomes
        .iter()
        .all(|(_, outcome)| matches!(outcome, RolloutOutcome::Complete { .. }));
    let text = match outcomes {
        [(name, RolloutOutcome::Complete { ready, desired })] => {
            format!("Rollout complete: {name} ({ready}/{desired} ready)")
        }
        [(name, RolloutOutcome::Stalled(reason))] => {
            format!("Rollout not progressing: {name} ({reason})")
        }
        [(name, RolloutOutcome::Unseen)] => format!("Rollout of {name}: no status available"),
        _ => {
            let parts: Vec<String> = outcomes
                .iter()
                .map(|(name, outcome)| match outcome {
                    RolloutOutcome::Complete { ready, desired } => {
                        format!("{name} complete ({ready}/{desired})")
                    }
                    RolloutOutcome::Stalled(reason) => {
                        format!("{name} not progressing ({reason})")
                    }
                    RolloutOutcome::Unseen => format!("{name} no status"),
                })
                .collect();
            format!("Rollouts: {}", parts.join(", "))
        }
    };
    (text, is_success)
}

/// The id of the toast of a watch over `workloads` (`namespace`, `name`).
pub(crate) fn rollout_toast_id(workloads: &[(String, String)]) -> SharedString {
    workloads
        .iter()
        .map(|(namespace, name)| format!("{namespace}/{name}"))
        .collect::<Vec<_>>()
        .join(",")
        .into()
}

/// A start or end toast of a watch; one id per watch, so the end replaces the start.
pub(crate) fn notify_rollout(
    window: &mut gpui_kit::Window,
    cx: &mut App,
    text: String,
    is_success: bool,
    id: SharedString,
) {
    let notification = if is_success {
        Notification::success(text)
    } else {
        Notification::warning(text)
    };
    window.push_notification(notification.id1::<RolloutToast>(id), cx);
}

/// One Deployment under watch.
struct Watched {
    namespace: String,
    name: String,
    /// The generation of the first status seen.
    baseline: Option<i64>,
    outcome: Option<RolloutOutcome>,
}

impl AppShell {
    /// Follows the rollouts of `workloads` (`namespace`, `name` of Deployments of `cluster`) and
    /// ends their start toast, `rollout_toast_id(workloads)`, with how they went. Silent when the
    /// cluster closes first.
    pub(crate) fn watch_rollouts(
        &mut self,
        cluster: ClusterRef,
        workloads: Vec<(String, String)>,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if workloads.is_empty() {
            return;
        }
        let id = rollout_toast_id(&workloads);
        let mut watched: Vec<Watched> = workloads
            .into_iter()
            .map(|(namespace, name)| Watched {
                namespace,
                name,
                baseline: None,
                outcome: None,
            })
            .collect();
        cx.spawn(async move |shell, cx| {
            let started = Instant::now();
            loop {
                let elapsed = started.elapsed();
                let is_open = shell.update(cx, |shell, cx| {
                    shell.poll_rollouts(&cluster, &mut watched, elapsed, cx)
                });
                if !is_open.unwrap_or(false) {
                    return;
                }
                if watched.iter().all(|watched| watched.outcome.is_some()) {
                    break;
                }
                cx.background_executor().timer(POLL).await;
            }
            let outcomes: Vec<(String, RolloutOutcome)> = watched
                .into_iter()
                .filter_map(|watched| Some((watched.name, watched.outcome?)))
                .collect();
            let (text, is_success) = rollout_summary(&outcomes);
            let _ = cx.update_window(window, |_, window, cx| {
                notify_rollout(window, cx, text, is_success, id);
            });
        })
        .detach();
    }

    /// One look at the Deployments under watch. `false` when the cluster is no longer open.
    fn poll_rollouts(
        &self,
        cluster: &ClusterRef,
        watched: &mut [Watched],
        elapsed: Duration,
        cx: &App,
    ) -> bool {
        let Some(live) = self.live_of(cluster, cx) else {
            return false;
        };
        for item in watched.iter_mut().filter(|item| item.outcome.is_none()) {
            match live.deployment_of(&item.namespace, &item.name) {
                Some(deployment) => {
                    let baseline = *item.baseline.get_or_insert(deployment.generation);
                    item.outcome = rollout_outcome(deployment, baseline, elapsed);
                }
                None if elapsed >= WATCH_LIMIT => item.outcome = Some(RolloutOutcome::Unseen),
                None => {}
            }
        }
        true
    }
}

#[cfg(test)]
#[path = "rollout_watch_tests.rs"]
mod rollout_watch_tests;
