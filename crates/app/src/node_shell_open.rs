//! Opening a node shell (spec 0037): the options dialog, the guarded start of the node's own
//! cluster (always typing the node name), the privileged pod, and the tab. The cleanup of that pod
//! is `node_shell_cleanup`; the tab itself is the debug tab of `debug_open`.
//!
//! A child of `app_shell`, like `shell_open`: every step names the cluster of the node and takes its
//! guard, permit, connection, and tier from that cluster's own slot, never from the primary.

use std::rc::Rc;

use cluster::{
    ObjectKind, ObjectRef, WriteOperation, WriteRequest, node_shell_pod_name, random_suffix,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{Context, Window};

use super::AppShell;
use super::debug_open::TabPlan;
use super::write_flow::{CleanupAudit, ConnectIntent, ConnectOpen, CreateThenAttach, WriteIntent};
use crate::audit_log::{AuditField, AuditObject};
use crate::cluster_registry::ClusterRef;
use crate::debug_dialogs::{NodeShellChosen, NodeShellForm, node_shell_warning};
use crate::dock::shell_cap_text;
use crate::resource_actions::{
    ActionAvailability, ResourceAction, action_availability, action_label, action_risk,
    node_shell_block, unavailable_text,
};
use crate::shell_tab::{ShellKind, ShellTarget};

/// The name of the container in every node shell pod (`cluster::debug_pod_bodies`).
const CONTAINER: &str = "shell";
/// The text of the confirm button.
const BUTTON: &str = "Open node shell";
/// The audit line of the create names what it does, not the button.
const CREATE_ACTION: &str = "Create node shell pod";

/// The fields the node shell confirm lists: the namespace the pod is created in (the request's own
/// diff leaves the pod's metadata out), then what the request sets.
fn node_shell_fields(namespace: &str, request: &WriteRequest) -> Vec<AuditField> {
    std::iter::once(AuditField {
        path: "metadata.namespace".to_owned(),
        value: Some(namespace.to_owned()),
    })
    .chain(
        request
            .changed_fields()
            .into_iter()
            .map(|field| AuditField {
                path: field.path.into_owned(),
                value: field.value,
            }),
    )
    .collect()
}

impl AppShell {
    /// Opens the Open node shell dialog for a node: the one entry of S on a node, the node menu,
    /// and the palette. The node is read again from its own cluster, so the gate, the setting, the
    /// operating system, and the defaults are what they are now.
    pub(crate) fn open_node_shell_options(
        &mut self,
        cluster: &ClusterRef,
        node: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::OpenNodeShell);
        let form = {
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
            let Some(summary) = live
                .nodes
                .items()
                .iter()
                .find(|summary| summary.name == node)
            else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the node is no longer listed"),
                );
                return;
            };
            // A stale menu or a key pressed in a gap cannot bypass the gate.
            let reason = match action_availability(ResourceAction::OpenNodeShell, &guard) {
                ActionAvailability::Disabled { reason } => Some(reason),
                ActionAvailability::Enabled => node_shell_block(summary),
            };
            if let Some(reason) = reason {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            NodeShellForm {
                node: summary.name.clone(),
                namespace: guard.profile.node_shell_namespace.clone(),
                image: guard.profile.debug_image.clone(),
            }
        };
        let (cluster, node) = (cluster.clone(), node.to_owned());
        self.show_node_shell_form(
            form,
            move |shell, chosen, window, cx| {
                shell.start_node_shell(&cluster, &node, chosen, window, cx);
            },
            window,
            cx,
        );
    }

    /// Continue of the Open node shell dialog: the guarded start of the node's own cluster. The
    /// pod is checked on the server first, the node name is typed in every environment, and the tab
    /// opens only after the create went through.
    fn start_node_shell(
        &mut self,
        cluster: &ClusterRef,
        node: &str,
        chosen: NodeShellChosen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::OpenNodeShell);
        if !self.dock.read(cx).has_room_for_shell(cx) {
            notify(window, cx, shell_cap_text().to_owned());
            return;
        }
        let (Some(cluster_label), Some((cluster_name, user, audit, generation))) = (
            self.label_of(cluster),
            self.guard_for(cluster, cx).map(|guard| {
                (
                    guard.display_name().to_owned(),
                    guard.summary.user.clone(),
                    CleanupAudit::of(&guard),
                    guard.generation,
                )
            }),
        ) else {
            notify(window, cx, format!("{} is not open", cluster.context));
            return;
        };
        self.record_run_started(cx);
        let name = node_shell_pod_name(node, &random_suffix());
        let request = ObjectRef::new(
            ObjectKind::Pod,
            Some(chosen.namespace.clone()),
            name.clone(),
        )
        .and_then(|target| {
            WriteRequest::new(
                target,
                WriteOperation::CreateNodeShellPod {
                    node: node.to_owned(),
                    image: chosen.image.clone(),
                    user,
                    instance: self.run_id.clone(),
                },
            )
        });
        let Some(request) = request else {
            notify(
                window,
                cx,
                unavailable_text(label, "the node name, namespace, or image is not valid"),
            );
            return;
        };
        let intent_label = format!("Open node shell for {node}");
        let fields = node_shell_fields(&chosen.namespace, &request);
        let risk = action_risk(ResourceAction::OpenNodeShell);
        let create = Rc::new(WriteIntent {
            cluster: cluster.clone(),
            cluster_name: cluster_name.clone().into(),
            action: ResourceAction::OpenNodeShell,
            label: intent_label.clone().into(),
            button: CREATE_ACTION.into(),
            request,
            risk,
            warnings: Vec::new(),
        });
        let plan = Rc::new(TabPlan {
            cluster: cluster.clone(),
            target: ShellTarget {
                cluster: cluster.clone(),
                namespace: chosen.namespace.clone(),
                pod: name,
                short_pod: node.to_owned(),
                container: CONTAINER.to_owned(),
            },
            kind: ShellKind::NodeShell {
                node: node.to_owned(),
                image: chosen.image.clone(),
            },
            cluster_label,
            image: chosen.image,
            namespace: Some(chosen.namespace),
            cleanup_audit: Some(audit),
            generation,
        });
        let intent = ConnectIntent {
            cluster: cluster.clone(),
            cluster_name: cluster_name.into(),
            action: ResourceAction::OpenNodeShell,
            label: intent_label.into(),
            button: BUTTON.into(),
            risk,
            warnings: vec![node_shell_warning(node)],
            object: AuditObject {
                kind: "Node".to_owned(),
                namespace: None,
                name: node.to_owned(),
            },
            fields,
            open: ConnectOpen::CreateThenAttach(CreateThenAttach {
                create,
                open: Rc::new({
                    let plan = Rc::clone(&plan);
                    move |shell, permit, connection, outcome, window, cx| {
                        shell.open_debug_tab(&plan, permit, connection, outcome, window, cx);
                    }
                }),
                discard: Rc::new(move |shell, connection, outcome, cx| {
                    shell.discard_debug_start(&plan, &connection, &outcome, cx);
                }),
            }),
        };
        self.start_connect(intent, window, cx);
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen node-shell-confirm`: the Open node shell dialog of a fixed node of a fixed
    /// Production cluster (or, for `node-shell-confirm-staging`, a Staging one: the node name is
    /// typed there too), the dry-run passed in 388 ms and the typed-name field empty. It needs no
    /// cluster at all, skips the gate, and its confirm button and Enter do nothing
    /// (`ConfirmDialog::show_fixture_after`), so it can never create a pod.
    pub(super) fn open_node_shell_confirm_fixture(
        &mut self,
        environment: crate::environment::Environment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use std::time::Duration;

        use gpui_kit::AppContext as _;

        use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
        use crate::environment::EnvironmentTier;
        use crate::write_guard::{ConfirmMode, confirm_step};

        const NODE: &str = "wk-03";
        let fixture_cluster = match environment.tier() {
            EnvironmentTier::Production => crate::screenshot::SHELL_FIXTURE_CLUSTER,
            _ => "stg-eu-1",
        };
        let cluster = ClusterRef {
            kubeconfig: std::path::PathBuf::from("fixture.yaml"),
            context: fixture_cluster.to_owned(),
        };
        let pod = node_shell_pod_name(NODE, "x7k2q");
        let request = ObjectRef::new(ObjectKind::Pod, Some("kube-system".to_owned()), pod)
            .and_then(|target| {
                WriteRequest::new(
                    target,
                    WriteOperation::CreateNodeShellPod {
                        node: NODE.to_owned(),
                        image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
                        user: None,
                        instance: "fixture".to_owned(),
                    },
                )
            });
        let Some(request) = request else {
            return;
        };
        let risk = action_risk(ResourceAction::OpenNodeShell);
        let label = format!("Open node shell for {NODE}");
        let fields = node_shell_fields("kube-system", &request);
        let create = Rc::new(WriteIntent {
            cluster: cluster.clone(),
            cluster_name: fixture_cluster.into(),
            action: ResourceAction::OpenNodeShell,
            label: label.clone().into(),
            button: CREATE_ACTION.into(),
            request,
            risk,
            warnings: Vec::new(),
        });
        let intent = ConnectIntent {
            cluster,
            cluster_name: fixture_cluster.into(),
            action: ResourceAction::OpenNodeShell,
            label: label.into(),
            button: BUTTON.into(),
            risk,
            warnings: vec![node_shell_warning(NODE)],
            object: AuditObject {
                kind: "Node".to_owned(),
                namespace: None,
                name: NODE.to_owned(),
            },
            fields,
            open: ConnectOpen::CreateThenAttach(CreateThenAttach {
                create,
                open: Rc::new(|_, _, _, _, _, _| {}),
                discard: Rc::new(|_, _, _, _| {}),
            }),
        };
        let confirm = confirm_step(
            ConfirmMode::for_tier(environment.tier()),
            risk,
            intent.expected(),
        );
        let inputs = DialogInputs {
            shell: cx.weak_entity(),
            kind: DialogKind::Connect(Rc::new(intent)),
            confirm,
            environment,
            generation: 0,
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        dialog.update(cx, |dialog, _| {
            dialog.show_fixture_after(Duration::from_millis(388));
        });
        ConfirmDialog::open(&dialog, window, cx);
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen node-shell-options` and `--screen debug-container-options`: the options dialogs
    /// over fixed data. Continue does nothing: the start they would run is an empty closure.
    pub(super) fn open_options_fixture(
        &mut self,
        launch: crate::launch_options::LaunchScreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::debug_dialogs::{DebugChoice, DebugForm};
        use crate::launch_options::LaunchScreen;
        match launch {
            LaunchScreen::NodeShellOptions => self.show_node_shell_form(
                NodeShellForm {
                    node: "wk-03".to_owned(),
                    namespace: "kube-system".to_owned(),
                    image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
                },
                |_, _, _, _| {},
                window,
                cx,
            ),
            LaunchScreen::DebugContainerOptions => self.show_debug_form(
                DebugForm {
                    pod: "api-7d9f8c-x2k4q".to_owned(),
                    choices: ["api", "worker", "otel-agent", "istio-proxy"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, name)| DebugChoice {
                            name: name.to_owned(),
                            tag: if index == 3 { "SIDECAR" } else { "MAIN" },
                        })
                        .collect(),
                    preselected: None,
                    image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
                },
                |_, _, _, _| {},
                window,
                cx,
            ),
            _ => {}
        }
    }
}

fn notify(window: &mut Window, cx: &mut gpui_kit::App, text: String) {
    window.push_notification(Notification::warning(text), cx);
}

#[cfg(test)]
#[path = "node_shell_open_tests.rs"]
pub(super) mod node_shell_open_tests;
