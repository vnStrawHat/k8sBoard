//! The replacement of a pod the app evicted (round 3, Q10). An Evict returns once the API accepts
//! it, so its toast only says the pod is terminating. This follows the pod list the session already
//! watches and says where the controller put the replacement, and when it landed on the same node.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use cluster::PodSummary;
use gpui_kit::{AnyWindowHandle, AppContext as _, Context};

use super::AppShell;
use super::write_flow::notify_with;
use crate::cluster_registry::ClusterRef;
use crate::issue_rules::cpu_limit_hint;

const POLL: Duration = Duration::from_secs(1);
/// A controller recreates a pod within seconds and the scheduler places it within seconds; with no
/// node after this long the toast says the replacement is Pending.
const WATCH_LIMIT: Duration = Duration::from_secs(45);

/// An evicted pod under watch.
struct Watched {
    pod: PodSummary,
    /// The pods of its controller when the watch began: a replacement is none of them, unless it
    /// has the same name (a StatefulSet's).
    known: HashSet<String>,
    /// What its CPU usage says of the replacement (`300m = limit: throttled`).
    hint: Option<String>,
    replacement: Option<PodSummary>,
}

/// The pod of `evicted`'s controller made after it, which is its replacement. A new pod with a name
/// of the evicted one (StatefulSet) counts, an old sibling does not.
pub(crate) fn replacement_of<'a>(
    evicted: &PodSummary,
    known: &HashSet<String>,
    pods: &'a [PodSummary],
) -> Option<&'a PodSummary> {
    pods.iter().find(|pod| {
        pod.namespace == evicted.namespace
            && pod.controller == evicted.controller
            && pod.created_at > evicted.created_at
            && (pod.name == evicted.name || !known.contains(&pod.name))
    })
}

/// The toast for `watched` pods: the replacement and its node, with the throttling hint.
fn watch_text(watched: &[Watched]) -> (String, bool) {
    let placed = |item: &Watched| {
        item.replacement
            .as_ref()
            .is_some_and(|r| r.node_name.is_some())
    };
    let same_node = |item: &Watched| {
        item.replacement
            .as_ref()
            .is_some_and(|r| r.node_name.is_some() && r.node_name == item.pod.node_name)
    };
    let is_success = watched.iter().all(placed);
    let [item] = watched else {
        let text = format!(
            "{} of {} replacements placed · {} on the same node",
            watched.iter().filter(|item| placed(item)).count(),
            watched.len(),
            watched.iter().filter(|item| same_node(item)).count(),
        );
        return (text, is_success);
    };
    let evicted = &item.pod.name;
    let mut text = match (&item.replacement, same_node(item)) {
        (Some(replacement), true) => format!(
            "Evicted {evicted}: replacement {} → {} (same node)",
            replacement.name,
            replacement.node_name.as_deref().unwrap_or_default()
        ),
        (Some(replacement), false) => match &replacement.node_name {
            Some(node) => format!(
                "Evicted {evicted}: replacement {} → {node}",
                replacement.name
            ),
            None => format!(
                "Evicted {evicted}: replacement {} is Pending, no node yet",
                replacement.name
            ),
        },
        (None, _) => format!("Evicted {evicted}: no replacement seen yet"),
    };
    if let Some(hint) = item.hint.as_deref().filter(|_| same_node(item)) {
        text.push_str(&format!(" · {hint}"));
    }
    (text, is_success)
}

impl AppShell {
    /// Follows the replacements of the pods (`namespace`, `name`) just evicted on `cluster`, and
    /// ends with a toast that says where they went. A pod with no controller has none; one the
    /// session does not list cannot be followed. Silent when the cluster closes first.
    pub(crate) fn watch_replacements(
        &mut self,
        cluster: ClusterRef,
        evicted: &[(String, String)],
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_of(&cluster, cx) else {
            return;
        };
        let pods = live.pods.items();
        let mut watched: Vec<Watched> = evicted
            .iter()
            .filter_map(|(namespace, name)| {
                let pod = pods
                    .iter()
                    .find(|pod| pod.namespace == *namespace && pod.name == *name)?;
                pod.controller.as_ref()?;
                Some(Watched {
                    known: pods
                        .iter()
                        .filter(|other| {
                            other.namespace == pod.namespace && other.controller == pod.controller
                        })
                        .map(|other| other.name.clone())
                        .collect(),
                    hint: cpu_limit_hint(pod, &live.metrics.pods.history),
                    pod: pod.clone(),
                    replacement: None,
                })
            })
            .collect();
        if watched.is_empty() {
            return;
        }
        cx.spawn(async move |shell, cx| {
            let started = Instant::now();
            loop {
                let is_open = shell.update(cx, |shell, cx| {
                    shell.poll_replacements(&cluster, &mut watched, cx)
                });
                if !is_open.unwrap_or(false) {
                    return;
                }
                let is_placed = watched.iter().all(|item| {
                    item.replacement
                        .as_ref()
                        .is_some_and(|r| r.node_name.is_some())
                });
                if is_placed || started.elapsed() >= WATCH_LIMIT {
                    break;
                }
                cx.background_executor().timer(POLL).await;
            }
            // Nothing was made within the limit: the controller may be slow or gone, and a toast
            // that says so would claim more than the app knows.
            if watched.iter().all(|item| item.replacement.is_none()) {
                return;
            }
            let (text, is_success) = watch_text(&watched);
            let _ = cx.update_window(window, |_, window, cx| {
                notify_with(window, cx, text, is_success);
            });
        })
        .detach();
    }

    /// One look at the pods under watch. `false` when the cluster is no longer open.
    fn poll_replacements(
        &self,
        cluster: &ClusterRef,
        watched: &mut [Watched],
        cx: &gpui_kit::App,
    ) -> bool {
        let Some(live) = self.live_of(cluster, cx) else {
            return false;
        };
        for item in watched.iter_mut() {
            let found = replacement_of(&item.pod, &item.known, live.pods.items());
            item.replacement = found.cloned();
        }
        true
    }
}

#[cfg(test)]
#[path = "replacement_watch_tests.rs"]
mod replacement_watch_tests;
