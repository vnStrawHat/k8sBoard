//! Opening and reopening shell tabs (spec 0036): the app-side glue between the entry points (S, the
//! menus, the palette, the dock, Reconnect), the guarded flow of `write_flow`, the dock, and the
//! audit line.
//!
//! A child of `app_shell`, like `write_flow`: every step names the cluster of the pod and takes
//! its guard, permit, and connection from that cluster's own slot, never from the primary.

use std::rc::Rc;

use cluster::ShellCommand;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{Context, Entity, WeakEntity, Window};

use super::AppShell;
use super::write_flow::{ConnectIntent, append_in_background};
use crate::audit_log::{AuditField, AuditObject, AuditOutcome, connect_entry};
use crate::cluster_registry::ClusterRef;
use crate::dock::shell_cap_text;
use crate::resource_actions::{ResourceAction, action_label, action_risk, default_shell_container};
use crate::settings::AppSettings;
use crate::shell_tab::{ShellEvent, ShellGrant, ShellTab, ShellTarget};
use crate::table_selection::{ClusterObject, ResourceKey};

/// The container to open a shell in, and the cluster of its pod.
#[derive(Clone)]
pub(crate) struct ShellOpen {
    pub(crate) cluster: ClusterRef,
    pub(crate) namespace: String,
    pub(crate) pod: String,
    pub(crate) container: String,
}

/// The name an audit line gives every session start, a reconnect included: a new exec is a new
/// shell.
const AUDIT_ACTION: &str = "Open shell";

/// What the dialog names and the audit line records of a shell start: the pod, and the container
/// and the shell it runs.
fn shell_audit(target: &ShellTarget, command: ShellCommand) -> (AuditObject, Vec<AuditField>) {
    let command = match command {
        ShellCommand::Auto => "auto",
        ShellCommand::Bash => "bash",
        ShellCommand::Sh => "sh",
    };
    (
        AuditObject {
            kind: "Pod".to_owned(),
            namespace: Some(target.namespace.clone()),
            name: target.pod.clone(),
        },
        vec![
            AuditField {
                path: "container".to_owned(),
                value: Some(target.container.clone()),
            },
            AuditField {
                path: "command".to_owned(),
                value: Some(command.to_owned()),
            },
        ],
    )
}

impl AppShell {
    /// Opens a shell in `open.container` of the pod: the one entry of S, the menus, the palette, and
    /// the dock. The guarded flow of the pod's own cluster asks first.
    pub(crate) fn start_shell(
        &mut self,
        open: ShellOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dock.read(cx).has_room_for_shell(cx) {
            window.push_notification(Notification::warning(shell_cap_text()), cx);
            return;
        }
        let (Some(cluster_name), Some(tab_label)) = (
            self.guard_for(&open.cluster, cx)
                .map(|guard| guard.display_name().to_owned()),
            self.view
                .slot_of(&open.cluster)
                .map(|index| self.view.slots()[index].label.clone()),
        ) else {
            let text = format!("{} is not open", open.cluster.context);
            window.push_notification(Notification::warning(text), cx);
            return;
        };
        let target = ShellTarget {
            cluster: open.cluster,
            namespace: open.namespace,
            pod: open.pod,
            container: open.container,
        };
        let dock = self.dock.clone();
        let opened = target.clone();
        let intent = shell_intent(
            &target,
            ShellCommand::Auto,
            cluster_name,
            "Open shell",
            "Open shell",
            Rc::new(move |shell, permit, connection, window, cx| {
                let grant = ShellGrant {
                    connection,
                    permit,
                    command: ShellCommand::Auto,
                };
                let tab = dock.update(cx, |dock, cx| {
                    dock.open_shell(opened.clone(), tab_label.clone(), grant, window, cx)
                });
                if let Some(tab) = tab {
                    shell.watch_shell(&tab, cx);
                }
            }),
        );
        self.start_connect(intent, window, cx);
    }

    /// S on a pod: its default container (the first running main one, else the first running one),
    /// in the cluster of the cursor row.
    pub(crate) fn open_default_shell(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ResourceKey::Pod { .. } = &subject.key else {
            return;
        };
        let found = self.slot_live(&subject.cluster, cx).and_then(|live| {
            let pod = live
                .pods
                .items()
                .iter()
                .find(|pod| subject.key.is_pod(pod))?;
            let container = default_shell_container(pod)?;
            Some((
                pod.namespace.clone(),
                pod.name.clone(),
                container.name.clone(),
            ))
        });
        let Some((namespace, pod, container)) = found else {
            let text = format!(
                "{} is unavailable: No running container",
                action_label(ResourceAction::OpenShell)
            );
            window.push_notification(Notification::warning(text), cx);
            return;
        };
        let open = ShellOpen {
            cluster: subject.cluster.clone(),
            namespace,
            pod,
            container,
        };
        self.start_shell(open, window, cx);
    }

    /// Reconnect, and a change of the shell: a new exec in the same tab, through the same guarded
    /// flow as the first one. The tab keeps its scrollback; the old session ends when the new one
    /// starts.
    pub(crate) fn reconnect_shell(
        &mut self,
        tab: &WeakEntity<ShellTab>,
        command: ShellCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entity) = tab.upgrade() else {
            return;
        };
        let target = entity.read(cx).target().clone();
        let Some(cluster_name) = self
            .guard_for(&target.cluster, cx)
            .map(|guard| guard.display_name().to_owned())
        else {
            let text = format!("{} is not open", target.cluster.context);
            window.push_notification(Notification::warning(text), cx);
            return;
        };
        let tab = tab.clone();
        let intent = shell_intent(
            &target,
            command,
            cluster_name,
            "Reconnect shell",
            "Reconnect",
            Rc::new(move |_, permit, connection, _, cx| {
                let grant = ShellGrant {
                    connection,
                    permit,
                    command,
                };
                let _ = tab.update(cx, |tab, cx| tab.connect(grant, cx));
            }),
        );
        self.start_connect(intent, window, cx);
    }

    /// Writes the audit line of every session start of `tab`: applied when the exec connection
    /// came up, failed with the error when it did not.
    fn watch_shell(&mut self, tab: &Entity<ShellTab>, cx: &mut Context<Self>) {
        cx.subscribe(tab, |shell, tab, event: &ShellEvent, cx| {
            shell.audit_shell_start(&tab, event, cx);
        })
        .detach();
    }

    fn audit_shell_start(
        &mut self,
        tab: &Entity<ShellTab>,
        event: &ShellEvent,
        cx: &mut Context<Self>,
    ) {
        let (target, command) = {
            let tab = tab.read(cx);
            (tab.target().clone(), tab.command())
        };
        let (outcome, error) = match event {
            ShellEvent::Opened => (AuditOutcome::Applied, None),
            ShellEvent::OpenFailed { error } => (AuditOutcome::Failed, Some(error.clone())),
        };
        let (object, fields) = shell_audit(&target, command);
        let Some(entry) = self
            .guard_for(&target.cluster, cx)
            .map(|guard| connect_entry(AUDIT_ACTION, object, fields, &guard, outcome, error))
        else {
            return;
        };
        let config_dir = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf);
        let shell = cx.weak_entity();
        cx.spawn(async move |_, cx| append_in_background(&shell, config_dir, entry, cx).await)
            .detach();
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen shell-confirm-fixture`: the Open shell dialog of a fixed pod in the primary
    /// cluster, over an unlocked session. It skips the gate, and its confirm button and Enter do
    /// nothing (`ConfirmDialog::show_fixture`), so it can never open a session.
    pub(super) fn open_shell_confirm_fixture(
        &mut self,
        cluster: &ClusterRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::AppContext as _;

        use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
        use crate::write_guard::{WriteLock, confirm_step};
        let Some(session) = self.slot_session(cluster).cloned() else {
            return;
        };
        session.update(cx, |session, cx| session.set_lock(WriteLock::Unlocked, cx));
        let Some(guard) = self.guard_for(cluster, cx) else {
            return;
        };
        let target = ShellTarget {
            cluster: cluster.clone(),
            namespace: "payments".to_owned(),
            pod: "api-7d9f8c-m8n2p".to_owned(),
            container: "api".to_owned(),
        };
        let intent = shell_intent(
            &target,
            ShellCommand::Auto,
            guard.display_name().to_owned(),
            "Open shell",
            "Open shell",
            Rc::new(|_, _, _, _, _| {}),
        );
        let confirm = confirm_step(guard.profile.confirm, intent.risk, intent.expected());
        let inputs = DialogInputs {
            shell: cx.weak_entity(),
            kind: DialogKind::Connect(Rc::new(intent)),
            confirm,
            environment: guard.profile.environment,
            generation: guard.generation,
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        dialog.update(cx, |dialog, _| dialog.show_fixture());
        ConfirmDialog::open(&dialog, window, cx);
    }
}

/// The intent of a shell start: Open shell, or Reconnect, of one container.
fn shell_intent(
    target: &ShellTarget,
    command: ShellCommand,
    cluster_name: String,
    label: &str,
    button: &str,
    open: Rc<crate::app_shell::write_flow::ConnectOpen>,
) -> ConnectIntent {
    let (object, fields) = shell_audit(target, command);
    ConnectIntent {
        cluster: target.cluster.clone(),
        cluster_name: cluster_name.into(),
        action: ResourceAction::OpenShell,
        label: label.to_owned().into(),
        button: button.to_owned().into(),
        risk: action_risk(ResourceAction::OpenShell),
        warnings: Vec::new(),
        object,
        fields,
        open,
    }
}

#[cfg(test)]
#[path = "shell_open_tests.rs"]
mod shell_open_tests;
