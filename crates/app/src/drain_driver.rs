//! The driver of a drain run (spec 0034 step 3b): starts the tab, sends one request at a time for
//! the step the state machine asks for, writes the audit summary of each node, and stops the run
//! when its cluster is released. Every write goes through `checked_write`, so the lock, the
//! generation, and the audit line of each commit stay in one place; a read is a plain list.
//!
//! A child of `app_shell`, like `write_flow`: every step names the cluster of the drain and takes
//! its guard, connection, and lock from that cluster's own slot, never from the primary.

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use cluster::{ClusterConnection, NodeScheduling, WriteError};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, AsyncApp, Context, SharedString, WeakEntity, Window,
};

use super::AppShell;
use super::batch_write::BATCH_RUNNING_REASON;
use super::write_flow::{
    CheckedWriteError, CommitMode, Confirmed, WriteStep, append_in_background, checked_write,
    notify, notify_with,
};
use crate::audit_log::{AuditEntry, AuditIdentity, append_audit, drain_summary_entry};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::drain_plan::DrainOptions;
use crate::drain_run::{DrainRun, FollowStep, NextStep, NodeSummary, RunInput};
use crate::drain_tab::{DrainTab, DrainTabInputs};
use crate::drain_writes::{DrainScope, cordon_write, removal_write};
use crate::node_edits::{CordonMode, NodeScope, TickedNode, cordon_batch};
use crate::resource_actions::{ResourceAction, unavailable_text};
use crate::settings::AppSettings;

/// The longest the driver sleeps at once, so the countdown of the tab moves and a Cancel is seen.
const TICK: Duration = Duration::from_secs(1);

/// What the dialog hands over when Drain is pressed.
pub(crate) struct DrainStart {
    pub(crate) cluster: ClusterRef,
    /// In the order they drain.
    pub(crate) nodes: Vec<String>,
    /// The nodes that are not cordoned yet.
    pub(crate) to_cordon: Vec<String>,
    pub(crate) options: DrainOptions,
    pub(crate) confirmed: Confirmed,
    pub(crate) generation: u64,
    /// Uids whose eviction the dialog already dry-ran.
    pub(crate) checked: HashSet<String>,
    pub(crate) note: Option<String>,
}

impl AppShell {
    /// Whether a drain runs on `cluster` now: one at a time, so two runs never fight over a node.
    pub(crate) fn has_running_drain(&self, cluster: &ClusterRef, cx: &gpui_kit::App) -> bool {
        self.dock.read(cx).has_running_drain(cluster, cx)
    }

    /// Which of `nodes` the cluster reports schedulable now: what the drain tab reads to say a node
    /// is uncordoned again. A cluster that is not open reports none.
    pub(crate) fn schedulable_nodes(
        &self,
        cluster: &ClusterRef,
        nodes: &[String],
        cx: &gpui_kit::App,
    ) -> Vec<String> {
        let Some(live) = self.live_of(cluster, cx) else {
            return Vec::new();
        };
        live.nodes
            .items()
            .iter()
            .filter(|node| nodes.contains(&node.name))
            .filter(|node| node.status.scheduling == NodeScheduling::Enabled)
            .map(|node| node.name.clone())
            .collect()
    }

    /// Why `action` may not start on `cluster` now, `None` when it may: a cordon, an uncordon, or a
    /// taint edit while a drain runs there would put pods back (or keep them away) behind the run's
    /// back, and the run would still report the node drained.
    pub(crate) fn drain_conflict(
        &self,
        cluster: &ClusterRef,
        action: ResourceAction,
        cx: &gpui_kit::App,
    ) -> Option<String> {
        let is_node_change = matches!(
            action,
            ResourceAction::Cordon | ResourceAction::Uncordon | ResourceAction::EditTaints
        );
        if !is_node_change || !self.has_running_drain(cluster, cx) {
            return None;
        }
        let name = self.guard_for(cluster, cx).map_or_else(
            || cluster.context.clone(),
            |guard| guard.display_name().to_owned(),
        );
        Some(format!("A drain is running on {name}"))
    }

    /// Starts the run of a confirmed drain: opens its tab and sends the first request. The dialog
    /// is closed by the caller.
    pub(crate) fn start_drain_run(
        &mut self,
        start: DrainStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DrainStart {
            cluster,
            nodes,
            to_cordon,
            options,
            confirmed,
            generation,
            checked,
            note,
        } = start;
        let prepared = {
            let (Some(guard), Some(connection)) = (
                self.guard_for(&cluster, cx),
                self.connection_of(&cluster, cx),
            ) else {
                notify(window, cx, format!("{} is not open", cluster.context));
                return;
            };
            let name = guard.display_name().to_owned();
            if self.has_running_drain(&cluster, cx) {
                notify(window, cx, format!("A drain is already running on {name}"));
                return;
            }
            // A batch commits one object at a time on this cluster; the run would interleave with it.
            if self.running_batches.contains(&cluster) {
                notify(window, cx, format!("{BATCH_RUNNING_REASON} on {name}"));
                return;
            }
            (name, AuditIdentity::of(&guard), connection)
        };
        let (cluster_name, identity, connection) = prepared;
        let run = DrainRun::new(RunInput {
            nodes,
            to_cordon,
            options,
            confirmed,
            generation,
            checked,
        });
        let inputs = DrainTabInputs {
            shell: cx.weak_entity(),
            cluster,
            cluster_name: cluster_name.into(),
            run,
            identity,
            note,
        };
        let session = self.session_of(&inputs.cluster).cloned();
        let tab = cx.new(|cx| {
            let mut tab = DrainTab::new(inputs);
            if let Some(session) = &session {
                tab.watch_nodes(session, cx);
            }
            tab
        });
        self.dock
            .update(cx, |dock, cx| dock.open_drain(tab.clone(), cx));
        let runtime = cx.global::<ClusterRuntime>().clone();
        let (shell, handle, weak) = (cx.weak_entity(), self.window, tab.downgrade());
        cx.spawn(async move |_, cx| {
            drive(&shell, &weak, connection, runtime, handle, cx).await;
        })
        .detach();
        #[cfg(test)]
        {
            self.last_drain_tab = Some(tab.downgrade());
        }
        cx.notify();
    }

    /// Stops the running drains of `clusters` because the app is about to release them (a switch,
    /// "Remove from view"): the runs end `stopped`, one summary line per node reached is written,
    /// and the notification stays after the tab is gone (decision 45).
    pub(super) fn stop_drains_of(&mut self, clusters: &[ClusterRef], cx: &mut Context<Self>) {
        let running = self.dock.read(cx).running_drains_of(clusters, cx);
        for (tab, name) in running {
            let reason = format!("{name} is no longer open");
            let (entries, notice) = stop_tab(&tab, DrainStop::Released, &reason, cx);
            if let Some(dir) = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf) {
                cx.spawn(async move |this, cx| {
                    for entry in entries {
                        append_in_background(&this, Some(dir.clone()), entry, cx).await;
                    }
                })
                .detach();
            }
            if let Some(notice) = notice {
                self.notify_later(notice, cx);
            }
        }
    }

    /// Stops every running drain because the app is quitting. The lines are written at once: the
    /// process may end before a task would write them.
    pub(super) fn stop_all_drains_now(&mut self, cx: &mut Context<Self>) {
        let running = self.dock.read(cx).running_drains(cx);
        let dir = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf);
        for (tab, _) in running {
            // Read before the stop: the summary counts the request in the air as unknown.
            let in_flight = tab.read(cx).in_flight_entry();
            let (mut entries, _) = stop_tab(&tab, DrainStop::Quit, "k8sBoard is closing", cx);
            entries.extend(in_flight);
            let Some(dir) = &dir else {
                continue;
            };
            for entry in entries {
                if let Err(error) = append_audit(dir, &entry) {
                    tracing::warn!(kind = ?error.kind(), "could not append to the audit log");
                }
            }
        }
    }

    /// The names of the clusters with a running drain, whichever they are, for the quit dialog.
    pub(super) fn running_drain_names(&self, cx: &gpui_kit::App) -> Vec<SharedString> {
        self.dock
            .read(cx)
            .running_drains(cx)
            .into_iter()
            .map(|(_, name)| name)
            .collect()
    }

    /// The names of the clusters with a running drain among `clusters`, for the "work will close"
    /// dialog.
    pub(super) fn running_drain_names_of(
        &self,
        clusters: &[ClusterRef],
        cx: &gpui_kit::App,
    ) -> Vec<SharedString> {
        self.dock
            .read(cx)
            .running_drains_of(clusters, cx)
            .into_iter()
            .map(|(_, name)| name)
            .collect()
    }

    /// Closes the tab of a finished drain.
    pub(crate) fn close_drain_tab(
        &mut self,
        tab: &gpui_kit::Entity<DrainTab>,
        cx: &mut Context<Self>,
    ) {
        self.dock.update(cx, |dock, cx| dock.close_drain(tab, cx));
    }

    /// The Uncordon button of a finished drain: a 0032 batch with its own dialog over the nodes the
    /// drain cordoned, as the cluster reports them now.
    pub(crate) fn uncordon_drained(
        &mut self,
        cluster: &ClusterRef,
        nodes: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = "Uncordon";
        let intent = {
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
            let listed: Vec<TickedNode> = nodes
                .iter()
                .filter_map(|name| {
                    let summary = live.nodes.items().iter().find(|node| node.name == *name)?;
                    Some(TickedNode {
                        name: name.clone(),
                        scheduling: summary.status.scheduling,
                        labels: summary.labels.clone(),
                    })
                })
                .collect();
            let scope = NodeScope {
                cluster,
                cluster_name: guard.display_name(),
            };
            cordon_batch(&scope, CordonMode::Uncordon, &listed)
        };
        match intent {
            Ok(intent) => self.start_batch(intent, window, cx),
            Err(reason) => notify(window, cx, unavailable_text(label, &reason)),
        }
    }
}

/// `--screen drain-progress`: the tab of a drain of one node, 12 of 23 pods gone and one refused by
/// its budget, drawn from fixed data. It starts no driver and sends nothing.
#[cfg(feature = "screenshot")]
impl AppShell {
    pub(super) fn open_drain_progress_fixture(
        &mut self,
        launch: crate::launch_options::LaunchScreen,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use cluster::{ControllerRef, DrainPod, PendingPod, WriteEffect, WriteMode, WriteOutcome};

        use crate::app_shell::write_flow::{DryRunState, TypedMatch, confirmed};
        use crate::dock::DockMode;
        use crate::drain_plan::PodKey;
        use crate::drain_run::NodeOutcome;

        self.pending_launch_screen = None;
        let secs = Duration::from_secs;
        let ok = || {
            Ok(WriteOutcome {
                mode: WriteMode::Commit,
                elapsed: Duration::from_millis(40),
                effect: WriteEffect::Created,
                created_name: None,
                uid: None,
                dropped_fields: Vec::new(),
            })
        };
        let Some(proof) = confirmed(&DryRunState::NotSupported, TypedMatch::NotNeeded, 0) else {
            return;
        };
        let pod = |namespace: &str, name: &str, kind: &str| DrainPod {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            uid: format!("fixture-{name}"),
            labels: Vec::new(),
            controller: Some(ControllerRef {
                kind: kind.to_owned(),
                name: "owner".to_owned(),
            }),
            is_mirror: false,
            has_empty_dir: false,
            is_finished: false,
            is_pending: false,
            is_terminating: false,
            claims: Vec::new(),
            pinned_volume: None,
        };
        let mut pods = vec![pod("payments", "api-7d9f8c-m8n2p", "ReplicaSet")];
        pods.extend(
            (0..22).map(|index| pod("web", &format!("frontend-6b8d7-p{index:02}"), "ReplicaSet")),
        );
        pods.extend(
            (0..6).map(|index| pod("kube-system", &format!("node-agent-{index}"), "DaemonSet")),
        );
        let mut run = DrainRun::new(RunInput {
            nodes: vec!["wk-04".to_owned()],
            to_cordon: vec!["wk-04".to_owned()],
            options: DrainOptions::default(),
            confirmed: proof,
            generation: 0,
            checked: pods.iter().map(|pod| pod.uid.clone()).collect(),
        });
        run.on_write(&NextStep::Cordon("wk-04".to_owned()), ok(), secs(2));
        run.on_read(Ok(pods.clone()), secs(5));
        let evict = |run: &mut DrainRun, pod: &DrainPod, at: u64| {
            run.on_write(&NextStep::Evict(PodKey::of(pod)), ok(), secs(at));
        };
        // Twelve pods were evicted and have gone; three more are being deleted.
        let web: Vec<&DrainPod> = pods.iter().filter(|pod| pod.namespace == "web").collect();
        for pod in &web[..15] {
            evict(&mut run, pod, 30);
        }
        let present: Vec<DrainPod> = pods
            .iter()
            .filter(|pod| !web[..12].iter().any(|gone| gone.uid == pod.uid))
            .cloned()
            .collect();
        run.on_poll(Ok(present), secs(99));
        // The budget of api-pdb refused its pod three times.
        let refused = PodKey::of(&pods[0]);
        for at in [86, 91, 101] {
            let message = "The disruption budget api-pdb needs 2 healthy pods and has 2 currently";
            let refusal = Err(CheckedWriteError::Write(WriteError::TooManyRequests {
                message: message.to_owned(),
                retry_after: None,
            }));
            run.on_write(&NextStep::Evict(refused.clone()), refusal, secs(at));
        }
        if launch == crate::launch_options::LaunchScreen::DrainProgressStuck {
            run.on_node_done(NodeOutcome::Stuck {
                reason: "Timed out after 20 min: 11 pods left".into(),
            });
        }
        if launch == crate::launch_options::LaunchScreen::DrainProgressPending {
            // The budget let its pod go in the end, every pod left, and the controller's
            // replacement of the first one cannot be placed.
            evict(&mut run, &pods[0], 102);
            for pod in &web[15..] {
                evict(&mut run, pod, 102);
            }
            let staying: Vec<DrainPod> = pods
                .iter()
                .filter(|pod| {
                    pod.controller
                        .as_ref()
                        .is_some_and(|c| c.kind == "DaemonSet")
                })
                .cloned()
                .collect();
            run.on_poll(Ok(staying), secs(106));
            run.on_node_done(NodeOutcome::Drained);
            run.start_follow(secs(107));
            let unplaced = PendingPod {
                namespace: "payments".to_owned(),
                name: "api-7d9f8c-x2k8d".to_owned(),
                uid: "fixture-api-replacement".to_owned(),
                controller: pods[0].controller.clone(),
                reason: Some(
                    "0/3 nodes are available: 1 node(s) had volume node affinity conflict, 2 node(s) had untolerated taint {workload: data}."
                        .to_owned(),
                ),
            };
            run.on_follow(Ok(vec![unplaced]), secs(110));
        }
        let shell = cx.weak_entity();
        let tab = cx.new(|_| {
            DrainTab::new(DrainTabInputs {
                shell,
                cluster: ClusterRef {
                    kubeconfig: std::path::PathBuf::from("fixture.yaml"),
                    context: "onprem-hn-1".to_owned(),
                },
                cluster_name: "onprem-hn-1".into(),
                run,
                identity: AuditIdentity::fixture("onprem-hn-1"),
                note: None,
            })
            .frozen_at(secs(113))
        });
        self.dock.update(cx, |dock, cx| {
            dock.open_drain(tab, cx);
            dock.set_mode(DockMode::Zoomed, cx);
        });
    }
}

/// Why a run is stopped from outside: its cluster is released, or the app quits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DrainStop {
    Released,
    Quit,
}

/// Stops the run of `tab` and takes what it owes: the summary lines of the nodes reached and the
/// notification, both once.
fn stop_tab(
    tab: &gpui_kit::Entity<DrainTab>,
    stop: DrainStop,
    reason: &str,
    cx: &mut Context<AppShell>,
) -> (Vec<AuditEntry>, Option<String>) {
    tab.update(cx, |tab, cx| {
        match stop {
            DrainStop::Released => tab.run_mut().stop(reason.to_owned()),
            DrainStop::Quit => tab.run_mut().abandon(reason.to_owned()),
        }
        let lines = tab.run_mut().take_summaries();
        let entries = summary_entries(tab, &lines);
        let notice = tab.run_mut().take_end_notice();
        cx.notify();
        (entries, notice)
    })
}

fn summary_entries(tab: &DrainTab, lines: &[NodeSummary]) -> Vec<AuditEntry> {
    lines
        .iter()
        .map(|line| {
            let budgets = tab.run().options().budgets;
            drain_summary_entry(tab.identity(), line, budgets, tab.note())
        })
        .collect()
}

/// What one request of the run needs from the tab, read in one go.
struct Sending {
    options: DrainOptions,
    confirmed: Confirmed,
    generation: u64,
    note: Option<String>,
    cluster: ClusterRef,
    cluster_name: SharedString,
}

/// The loop: ask the state machine for the next step, do it, tell it the result. One request is in
/// flight at a time; a dropped tab (its cluster released) ends the loop.
async fn drive(
    shell: &WeakEntity<AppShell>,
    tab: &WeakEntity<DrainTab>,
    connection: ClusterConnection,
    runtime: ClusterRuntime,
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
) {
    loop {
        let Ok(step) = tab.read_with(cx, |tab, _| tab.run().next_step(tab.now())) else {
            return;
        };
        match step {
            NextStep::Cordon(_) | NextStep::DryRun(_) | NextStep::Evict(_) => {
                let Ok(sending) = tab.read_with(cx, |tab, _| Sending {
                    options: *tab.run().options(),
                    confirmed: tab.run().confirmed(),
                    generation: tab.run().generation(),
                    note: tab.note().map(str::to_owned),
                    cluster: tab.cluster().clone(),
                    cluster_name: tab.cluster_name().clone(),
                }) else {
                    return;
                };
                if tab
                    .update(cx, |tab, _| tab.run_mut().begin_write(&step))
                    .is_err()
                {
                    return;
                }
                let result = send(shell, &step, sending, cx).await;
                let updated = tab.update(cx, |tab, cx| {
                    let now = tab.now();
                    tab.run_mut().on_write(&step, result, now);
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
            }
            NextStep::Read(_) | NextStep::Poll => {
                let node = match &step {
                    NextStep::Read(node) => Some(node.clone()),
                    _ => tab
                        .read_with(cx, |tab, _| tab.run().current_node().map(str::to_owned))
                        .ok()
                        .flatten(),
                };
                let Some(node) = node else {
                    return;
                };
                let read = list_pods(&connection, &runtime, &node).await;
                let is_start = matches!(step, NextStep::Read(_));
                let updated = tab.update(cx, |tab, cx| {
                    let now = tab.now();
                    if is_start {
                        tab.run_mut().on_read(read, now);
                    } else {
                        tab.run_mut().on_poll(read, now);
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
            }
            NextStep::Sleep(wait) => {
                cx.background_executor().timer(wait.min(TICK)).await;
                if tab.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
            NextStep::NodeDone(outcome) => {
                if tab
                    .update(cx, |tab, cx| {
                        tab.run_mut().on_node_done(outcome);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                write_summaries(shell, tab, cx).await;
            }
            NextStep::Finished => {
                write_summaries(shell, tab, cx).await;
                let ended = tab.update(cx, |tab, cx| {
                    cx.notify();
                    (tab.run_mut().take_end_notice(), tab.run().is_drained())
                });
                if let Ok((Some(notice), is_success)) = ended {
                    let _ = cx.update_window(window, |_, window, cx| {
                        notify_with(window, cx, notice, is_success);
                    });
                }
                follow_replacements(tab, &connection, &runtime, window, cx).await;
                return;
            }
        }
    }
}

/// Sends the write of `step` through `checked_write`: a dry-run for `DryRun`, a commit with the
/// dialog's proof for the others. A name the write path refuses is a failed pod, not a request.
async fn send(
    shell: &WeakEntity<AppShell>,
    step: &NextStep,
    sending: Sending,
    cx: &mut AsyncApp,
) -> Result<cluster::WriteOutcome, CheckedWriteError> {
    let scope = DrainScope {
        cluster: &sending.cluster,
        cluster_name: &sending.cluster_name,
    };
    let (intent, mode) = match step {
        NextStep::Cordon(node) => (
            cordon_write(scope, node),
            CommitMode::Commit {
                confirmed: sending.confirmed,
            },
        ),
        NextStep::DryRun(key) => (
            removal_write(scope, key, &sending.options),
            CommitMode::DryRun,
        ),
        NextStep::Evict(key) => (
            removal_write(scope, key, &sending.options),
            CommitMode::Commit {
                confirmed: sending.confirmed,
            },
        ),
        _ => (None, CommitMode::DryRun),
    };
    let Some(intent) = intent else {
        return Err(CheckedWriteError::Write(WriteError::Invalid {
            message: "the name is not valid for Kubernetes".to_owned(),
            fields: Vec::new(),
        }));
    };
    let step = WriteStep {
        intent: Rc::new(intent),
        generation: sending.generation,
        mode,
        note: sending.note,
    };
    checked_write(shell, step, cx).await
}

/// The pods of `node`, or the text that says why they could not be listed.
async fn list_pods(
    connection: &ClusterConnection,
    runtime: &ClusterRuntime,
    node: &str,
) -> Result<Vec<cluster::DrainPod>, SharedString> {
    let (connection, name) = (connection.clone(), node.to_owned());
    let read = runtime
        .spawn(async move { connection.drain_pods(&name).await })
        .await;
    match read {
        Ok(Ok(pods)) => Ok(pods),
        Ok(Err(error)) => Err(format!("Could not list pods on {node}: {error}").into()),
        Err(_) => Err("The request task stopped".into()),
    }
}

/// After a clean end, looks for the replacements of the evicted pods until the tab says it is done:
/// a pod whose controller recreated it Pending is not gone for good. A closed tab ends the loop.
async fn follow_replacements(
    tab: &WeakEntity<DrainTab>,
    connection: &ClusterConnection,
    runtime: &ClusterRuntime,
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
) {
    if tab
        .update(cx, |tab, _| {
            let now = tab.now();
            tab.run_mut().start_follow(now);
        })
        .is_err()
    {
        return;
    }
    loop {
        let Ok(step) = tab.read_with(cx, |tab, _| tab.run().next_follow(tab.now())) else {
            return;
        };
        match step {
            FollowStep::Poll => {
                let read = list_pending_pods(connection, runtime).await;
                let updated = tab.update(cx, |tab, cx| {
                    let now = tab.now();
                    tab.run_mut().on_follow(read, now);
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
            }
            FollowStep::Sleep(wait) => {
                cx.background_executor().timer(wait.min(TICK)).await;
                if tab.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
            FollowStep::Done => {
                let notice = tab.update(cx, |tab, cx| {
                    cx.notify();
                    tab.run().follow_notice()
                });
                if let Ok(Some(notice)) = notice {
                    let _ = cx.update_window(window, |_, window, cx| {
                        notify_with(window, cx, notice, false);
                    });
                }
                return;
            }
        }
    }
}

/// The Pending pods of the cluster, or the text that says why they could not be listed.
async fn list_pending_pods(
    connection: &ClusterConnection,
    runtime: &ClusterRuntime,
) -> Result<Vec<cluster::PendingPod>, SharedString> {
    let connection = connection.clone();
    let read = runtime
        .spawn(async move { connection.pending_pods().await })
        .await;
    match read {
        Ok(Ok(pods)) => Ok(pods),
        Ok(Err(error)) => Err(format!("Could not list pending pods: {error}").into()),
        Err(_) => Err("The request task stopped".into()),
    }
}

/// Writes the summary line of every node that ended and has none yet, off the main thread.
async fn write_summaries(
    shell: &WeakEntity<AppShell>,
    tab: &WeakEntity<DrainTab>,
    cx: &mut AsyncApp,
) {
    let Ok(entries) = tab.update(cx, |tab, _| {
        let lines = tab.run_mut().take_summaries();
        summary_entries(tab, &lines)
    }) else {
        return;
    };
    let dir: Option<PathBuf> = shell
        .read_with(cx, |_, cx| {
            AppSettings::config_dir(cx).map(std::path::Path::to_path_buf)
        })
        .ok()
        .flatten();
    for entry in entries {
        append_in_background(shell, dir.clone(), entry, cx).await;
    }
}
