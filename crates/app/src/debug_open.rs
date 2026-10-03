//! Debug container starts (spec 0037): the app-side glue between the entry points (the pod menu,
//! the drawer menu, a shell tab that found no shell), the options dialog, the guarded flow of
//! `write_flow`, the dock, and the audit line.
//!
//! A child of `app_shell`, like `shell_open`: every step names the cluster of the pod and takes its
//! guard, permit, and connection from that cluster's own slot, never from the primary.

use std::rc::Rc;

use cluster::{
    AttachPermit, ObjectKind, ObjectRef, ShellCommand, WriteOperation, WriteOutcome, WriteRequest,
    debug_container_name,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{Context, WeakEntity, Window};

use super::AppShell;
use super::write_flow::{
    CleanupAudit, ConnectIntent, ConnectOpen, CreateThenAttach, NodeShellCleanup, WriteIntent,
};
use crate::audit_log::{AuditField, AuditObject};
use crate::cluster_form::edit_entry;
use crate::cluster_registry::ClusterRef;
use crate::debug_dialogs::{DEBUG_WARNING, DebugChoice, DebugChosen, DebugForm};
use crate::dock::shell_cap_text;
use crate::resource_actions::{
    ActionAvailability, DebugPod, ResourceAction, action_availability, action_label, action_risk,
    debug_container_block, debug_targets, unavailable_text,
};
use crate::settings::AppSettings;
use crate::shell_tab::{AttachGrant, ShellKind, ShellTab, ShellTarget, short_pod_name};
use crate::write_guard::ActionRisk;

/// The text of the confirm button and the audit line of the patch.
const BUTTON: &str = "Add debug container";

/// What a started debug session needs to open its tab and to remember what the user chose.
pub(super) struct TabPlan {
    pub(super) cluster: ClusterRef,
    pub(super) target: ShellTarget,
    pub(super) kind: ShellKind,
    /// The switcher text of the cluster, for the banner and the title while several are viewed.
    pub(super) cluster_label: String,
    /// The image the user chose; stored per cluster after a successful start.
    pub(super) image: String,
    /// The namespace of a node shell pod, stored the same way; `None` for a debug container.
    pub(super) namespace: Option<String>,
    /// Set for a node shell: how to describe the cluster in the audit line of its delete.
    pub(super) cleanup_audit: Option<CleanupAudit>,
}

impl AppShell {
    /// Opens the Debug container dialog for a pod: the one entry of the pod menu, the drawer menu,
    /// and the Debug container… button of a shell tab. The pod is read again from its own cluster,
    /// so the gate, the pod's running containers, and the default image are what they are now.
    pub(crate) fn open_debug_options(
        &mut self,
        pod: DebugPod,
        preselected: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::DebugContainer);
        let form = {
            let (Some(guard), Some(live)) = (
                self.guard_for(&pod.cluster, cx),
                self.slot_live(&pod.cluster, cx),
            ) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            let found = live
                .pods
                .items()
                .iter()
                .find(|summary| summary.namespace == pod.namespace && summary.name == pod.pod);
            let Some(summary) = found else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the pod is no longer listed"),
                );
                return;
            };
            // A stale menu or a key pressed in a gap cannot bypass the gate.
            let reason = match action_availability(ResourceAction::DebugContainer, &guard) {
                ActionAvailability::Disabled { reason } => Some(reason),
                ActionAvailability::Enabled => debug_container_block(summary),
            };
            if let Some(reason) = reason {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            DebugForm {
                pod: summary.name.clone(),
                choices: debug_targets(summary)
                    .map(|container| DebugChoice {
                        name: container.name.clone(),
                        tag: crate::pod_drawer::kind_tag_text(container.kind),
                    })
                    .collect(),
                preselected,
                image: guard.profile.debug_image.clone(),
            }
        };
        self.show_debug_form(
            form,
            move |shell, chosen, window, cx| shell.start_debug_container(&pod, chosen, window, cx),
            window,
            cx,
        );
    }

    /// Continue of the Debug container dialog: the guarded start of the pod's own cluster. The
    /// patch is checked on the server first, and the tab opens only after it went through.
    fn start_debug_container(
        &mut self,
        pod: &DebugPod,
        chosen: DebugChosen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::DebugContainer);
        if !self.dock.read(cx).has_room_for_shell(cx) {
            notify(window, cx, shell_cap_text().to_owned());
            return;
        }
        let (Some(cluster_name), Some(cluster_label)) = (
            self.guard_for(&pod.cluster, cx)
                .map(|guard| guard.display_name().to_owned()),
            self.view
                .slot_of(&pod.cluster)
                .map(|index| self.view.slots()[index].label.clone()),
        ) else {
            notify(window, cx, format!("{} is not open", pod.cluster.context));
            return;
        };
        let short_pod = self
            .slot_live(&pod.cluster, cx)
            .and_then(|live| {
                live.pods
                    .items()
                    .iter()
                    .find(|summary| summary.namespace == pod.namespace && summary.name == pod.pod)
                    .map(short_pod_name)
            })
            .unwrap_or_else(|| pod.pod.clone());
        let name = debug_container_name();
        let request = ObjectRef::new(
            ObjectKind::Pod,
            Some(pod.namespace.clone()),
            pod.pod.clone(),
        )
        .and_then(|target| {
            WriteRequest::new(
                target,
                WriteOperation::AddDebugContainer {
                    name: name.clone(),
                    image: chosen.image.clone(),
                    target_container: chosen.target_container.clone(),
                },
            )
        });
        let Some(request) = request else {
            notify(
                window,
                cx,
                unavailable_text(label, "the pod name or the image is not valid"),
            );
            return;
        };
        let intent_label = format!("Add debug container to {}", pod.pod);
        let fields = request
            .changed_fields()
            .into_iter()
            .map(|field| AuditField {
                path: field.path.into_owned(),
                value: field.value,
            })
            .collect();
        let create = Rc::new(WriteIntent {
            cluster: pod.cluster.clone(),
            cluster_name: cluster_name.clone().into(),
            action: ResourceAction::DebugContainer,
            label: intent_label.clone().into(),
            button: BUTTON.into(),
            request,
            risk: action_risk(ResourceAction::DebugContainer),
            expected_name: None,
            warnings: Vec::new(),
        });
        let plan = Rc::new(TabPlan {
            cluster: pod.cluster.clone(),
            target: ShellTarget {
                cluster: pod.cluster.clone(),
                namespace: pod.namespace.clone(),
                pod: pod.pod.clone(),
                short_pod,
                container: name,
            },
            kind: ShellKind::Debug {
                target_container: chosen.target_container,
                image: chosen.image.clone(),
            },
            cluster_label,
            image: chosen.image,
            namespace: None,
            cleanup_audit: None,
        });
        let intent = ConnectIntent {
            cluster: pod.cluster.clone(),
            cluster_name: cluster_name.into(),
            action: ResourceAction::DebugContainer,
            label: intent_label.into(),
            button: BUTTON.into(),
            risk: ActionRisk::Change,
            warnings: vec![DEBUG_WARNING.into()],
            object: AuditObject {
                kind: "Pod".to_owned(),
                namespace: Some(pod.namespace.clone()),
                name: pod.pod.clone(),
            },
            fields,
            expected_name: None,
            open: ConnectOpen::CreateThenAttach(CreateThenAttach {
                create,
                open: Rc::new(move |shell, permit, connection, outcome, window, cx| {
                    shell.open_debug_tab(&plan, permit, connection, outcome, window, cx);
                }),
            }),
        };
        self.start_connect(intent, window, cx);
    }

    /// The tab of a started debug session, after the write went through: the attach waits for the
    /// container to run, and the audit line of the start follows the tab. What the user chose is
    /// stored once the tab is open.
    ///
    /// A node shell's pod exists now, so every path out of here that opens no tab deletes it: the
    /// uid the server reported is what makes the delete exact, a window already closing or a tab
    /// past the cap has no one to own the pod, and the registered cleanup follows the tab from then
    /// on (shell end, tab close, switch, window close, quit).
    pub(super) fn open_debug_tab(
        &mut self,
        plan: &TabPlan,
        permit: AttachPermit,
        connection: cluster::ClusterConnection,
        outcome: WriteOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cleanup = match &plan.cleanup_audit {
            None => None,
            Some(audit) => match node_shell_cleanup(plan, audit, &connection, &outcome) {
                Some(cleanup) => Some(cleanup),
                None => {
                    // Nothing can delete a pod the server did not identify; the user is told which.
                    let text = format!(
                        "The node shell pod {}/{} was created but could not be tracked; delete it by hand",
                        plan.target.namespace, plan.target.pod
                    );
                    notify(window, cx, text);
                    return;
                }
            },
        };
        let is_closing = self.node_shell_runs.is_closing();
        let tab = (!is_closing)
            .then(|| {
                let grant = AttachGrant { connection, permit };
                self.dock.update(cx, |dock, cx| {
                    dock.open_attach(
                        plan.target.clone(),
                        plan.kind.clone(),
                        plan.cluster_label.clone(),
                        grant,
                        window,
                        cx,
                    )
                })
            })
            .flatten();
        let Some(tab) = tab else {
            if let Some(cleanup) = cleanup {
                self.begin_cleanup(cleanup, cx);
            }
            return;
        };
        self.watch_shell(&tab, cx);
        self.begin_shell_start(&tab, ShellCommand::Auto, cx);
        if let Some(cleanup) = cleanup {
            self.register_cleanup(tab.entity_id(), cleanup, cx);
        }
        self.store_debug_choice(&plan.cluster, &plan.image, plan.namespace.as_deref(), cx);
    }

    /// Writes the image (and namespace) the user chose to the cluster's registry entry, when they
    /// differ from what it resolves to now: the next dialog opens with them.
    fn store_debug_choice(
        &mut self,
        cluster: &ClusterRef,
        image: &str,
        namespace: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some((stored_image, stored_namespace)) = self.guard_for(cluster, cx).map(|guard| {
            (
                guard.profile.debug_image.clone(),
                guard.profile.node_shell_namespace.clone(),
            )
        }) else {
            return;
        };
        let is_image_new = image != stored_image;
        let new_namespace = namespace.filter(|namespace| *namespace != stored_namespace);
        if !is_image_new && new_namespace.is_none() {
            return;
        }
        let (image, new_namespace) = (image.to_owned(), new_namespace.map(str::to_owned));
        AppSettings::update(cx, |settings| {
            edit_entry(&mut settings.registry, cluster, |entry| {
                if is_image_new {
                    entry.debug_image = Some(image);
                }
                if let Some(namespace) = new_namespace {
                    entry.node_shell_namespace = Some(namespace);
                }
            });
        });
    }

    /// Whether `tab` is a debug tab (its Reconnect opens the options dialog again).
    pub(crate) fn is_debug_tab(&self, tab: &WeakEntity<ShellTab>, cx: &gpui_kit::App) -> bool {
        tab.upgrade()
            .is_some_and(|tab| !tab.read(cx).kind().is_exec())
    }

    /// Reconnect of a debug tab: the options dialog again, for a new container or pod. Nothing is
    /// reused.
    pub(crate) fn reopen_debug_options(
        &mut self,
        tab: &WeakEntity<ShellTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entity) = tab.upgrade() else {
            return;
        };
        let (target, kind) = {
            let tab = entity.read(cx);
            (tab.target().clone(), tab.kind().clone())
        };
        match kind {
            ShellKind::Debug {
                target_container, ..
            } => {
                let pod = DebugPod {
                    cluster: target.cluster,
                    namespace: target.namespace,
                    pod: target.pod,
                };
                self.open_debug_options(pod, Some(target_container), window, cx);
            }
            ShellKind::NodeShell { node, .. } => {
                self.open_node_shell_options(&target.cluster, &node, window, cx);
            }
            ShellKind::Exec => {}
        }
    }

    /// The Debug container… button of a shell tab that found no shell: the options dialog for its
    /// pod, with that container selected.
    pub(crate) fn offer_debug_container(
        &mut self,
        tab: &WeakEntity<ShellTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entity) = tab.upgrade() else {
            return;
        };
        let target = entity.read(cx).target().clone();
        let pod = DebugPod {
            cluster: target.cluster,
            namespace: target.namespace,
            pod: target.pod,
        };
        self.open_debug_options(pod, Some(target.container), window, cx);
    }
}

/// The cleanup of the pod a node shell start created, from the uid the server reported.
fn node_shell_cleanup(
    plan: &TabPlan,
    audit: &CleanupAudit,
    connection: &cluster::ClusterConnection,
    outcome: &WriteOutcome,
) -> Option<NodeShellCleanup> {
    let uid = outcome.uid.clone()?;
    let target = ObjectRef::new(
        ObjectKind::Pod,
        Some(plan.target.namespace.clone()),
        plan.target.pod.clone(),
    )?;
    let request = WriteRequest::new(target, WriteOperation::DeleteNodeShellPod { uid })?;
    NodeShellCleanup::new(connection.clone(), request, audit.clone())
}

fn notify(window: &mut Window, cx: &mut gpui_kit::App, text: String) {
    window.push_notification(Notification::warning(text), cx);
}

#[cfg(test)]
#[path = "debug_open_tests.rs"]
pub(super) mod debug_open_tests;
