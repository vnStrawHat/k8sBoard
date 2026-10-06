//! An HPA undoing a manual scale (spec 0013).
//!
//! A Scale of a workload that an HPA targets goes through, and some seconds later the HPA sets the
//! replicas back into its range. Nothing in the app said so, so for a short while after the scale
//! the app follows `spec.replicas` of the row it already watches and announces the change.

use std::time::{Duration, Instant};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{AnyWindowHandle, AppContext as _, Context};

use super::AppShell;
use crate::cluster_registry::ClusterRef;
use crate::table_selection::ClusterObject;
use crate::workload_actions::{ManagingHpa, ScaleTarget};

const POLL: Duration = Duration::from_secs(2);
/// The HPA syncs every 15 seconds by default, so this sees at least one sync after the scale.
const WATCH_LIMIT: Duration = Duration::from_secs(30);

/// What the replicas of the workload did since the scale.
#[derive(Debug)]
struct ScaleFollow {
    requested: u32,
    /// The first replicas count seen: a watch that is still catching up shows the old one.
    first_seen: Option<u32>,
    is_requested_seen: bool,
}

impl ScaleFollow {
    fn new(requested: u32) -> Self {
        Self {
            requested,
            first_seen: None,
            is_requested_seen: false,
        }
    }

    /// The count the replicas moved to when something other than the scale changed them, or `None`
    /// while they are what the scale asked for, or still what they were before it.
    fn moved_to(&mut self, replicas: u32) -> Option<u32> {
        let first_seen = *self.first_seen.get_or_insert(replicas);
        if replicas == self.requested {
            self.is_requested_seen = true;
            return None;
        }
        (self.is_requested_seen || replicas != first_seen).then_some(replicas)
    }
}

/// `HPA web set replicas back to 3 (max 3)`; the bound is named when the HPA stopped at it.
pub(crate) fn hpa_revert_text(hpa: &ManagingHpa, replicas: u32) -> String {
    let bound = if replicas == hpa.max {
        format!(" (max {})", hpa.max)
    } else if replicas == hpa.min {
        format!(" (min {})", hpa.min)
    } else {
        String::new()
    };
    format!("HPA {} set replicas back to {replicas}{bound}", hpa.name)
}

impl AppShell {
    /// Follows the replicas of `subject` for a short while after a Scale to `requested` and warns
    /// when the HPA `hpa` moves them. Silent when nothing moves them or the cluster closes first.
    pub(crate) fn watch_hpa_scale(
        &mut self,
        subject: ClusterObject,
        requested: u32,
        hpa: ManagingHpa,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let mut follow = ScaleFollow::new(requested);
        cx.spawn(async move |shell, cx| {
            let started = Instant::now();
            while started.elapsed() < WATCH_LIMIT {
                let replicas = shell.read_with(cx, |shell, cx| {
                    shell.replicas_of(&subject.cluster, &subject, cx)
                });
                let Ok(replicas) = replicas else {
                    return;
                };
                if let Some(back) = replicas.and_then(|replicas| follow.moved_to(replicas)) {
                    let text = hpa_revert_text(&hpa, back);
                    let _ = cx.update_window(window, |_, window, cx| {
                        window.push_notification(Notification::warning(text), cx);
                    });
                    return;
                }
                cx.background_executor().timer(POLL).await;
            }
        })
        .detach();
    }

    /// `spec.replicas` of the listed workload `subject`, `None` while it is not listed.
    fn replicas_of(
        &self,
        cluster: &ClusterRef,
        subject: &ClusterObject,
        cx: &gpui_kit::App,
    ) -> Option<u32> {
        let row = self.live_of(cluster, cx)?.row_of(&subject.key)?;
        Some(ScaleTarget::of(&row.object, &[])?.desired)
    }
}

#[cfg(test)]
#[path = "hpa_watch_tests.rs"]
mod hpa_watch_tests;
