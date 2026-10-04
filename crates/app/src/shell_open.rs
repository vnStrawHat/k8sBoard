//! Opening and reopening shell tabs (spec 0036): the app-side glue between the entry points (S, the
//! menus, the palette, the dock, Reconnect), the guarded flow of `write_flow`, the dock, and the
//! audit line.
//!
//! A child of `app_shell`, like `write_flow`: every step names the cluster of the pod and takes
//! its guard, permit, and connection from that cluster's own slot, never from the primary.

use std::collections::HashMap;
use std::rc::Rc;

use cluster::ShellCommand;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{Context, Entity, EntityId, Subscription, WeakEntity, Window};

use super::AppShell;
use super::write_flow::{ConnectIntent, ConnectOpen, ExecOpen, queue_audit, report_audit};
use crate::audit_log::{
    AuditEntry, AuditField, AuditObject, AuditOutcome, append_audit, connect_entry,
};
use crate::cluster_registry::ClusterRef;
use crate::dock::shell_cap_text;
use crate::resource_actions::{ResourceAction, action_label, action_risk, default_shell_container};
use crate::settings::AppSettings;
use crate::shell_tab::{ShellEvent, ShellGrant, ShellKind, ShellTab, ShellTarget, short_pod_name};
use crate::table_selection::{ClusterObject, ResourceKey};

/// The container to open a shell in, and the cluster of its pod.
#[derive(Clone)]
pub(crate) struct ShellOpen {
    pub(crate) cluster: ClusterRef,
    pub(crate) namespace: String,
    pub(crate) pod: String,
    pub(crate) short_pod: String,
    pub(crate) container: String,
}

/// The name an audit line gives every session start, a reconnect included: a new exec is a new
/// shell.
const AUDIT_ACTION: &str = "Open shell";
/// Why a start has no result: its tab was closed, or a newer start replaced it.
const ABANDONED_TEXT: &str = "the session was closed or replaced before it reported";

/// The starts that have not reported yet, each with the audit line to write if none ever does. A
/// request may already have reached the server when a tab is closed, the app quits, or a Reconnect
/// replaces the session, so the start must not vanish from the log.
#[derive(Default)]
pub(super) struct ShellStarts {
    pending: HashMap<EntityId, AuditEntry>,
    /// Writes what is still pending when the app quits.
    quit: Option<Subscription>,
}

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

/// What the audit line of a session start names, by what the tab runs: the action, the pod, and the
/// parameters. A debug start records the image and, for a node shell, the privilege; never bytes.
fn start_audit(
    tab: &ShellTab,
    command: ShellCommand,
) -> (&'static str, AuditObject, Vec<AuditField>) {
    let target = tab.target();
    let field = |path: &str, value: &str| AuditField {
        path: path.to_owned(),
        value: Some(value.to_owned()),
    };
    let pod = || AuditObject {
        kind: "Pod".to_owned(),
        namespace: Some(target.namespace.clone()),
        name: target.pod.clone(),
    };
    match tab.kind() {
        ShellKind::Exec => {
            let (object, fields) = shell_audit(target, command);
            (AUDIT_ACTION, object, fields)
        }
        ShellKind::Debug {
            target_container,
            image,
        } => (
            "Open debug shell",
            pod(),
            vec![
                field("container", &target.container),
                field("target_container", target_container),
                field("image", image),
            ],
        ),
        ShellKind::NodeShell { node, image } => (
            "Open node shell",
            pod(),
            vec![
                field("node", node),
                field("image", image),
                field("privileged", "true"),
            ],
        ),
    }
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
            self.slot_label(&open.cluster),
        ) else {
            let text = format!("{} is not open", open.cluster.context);
            window.push_notification(Notification::warning(text), cx);
            return;
        };
        let target = ShellTarget {
            cluster: open.cluster,
            namespace: open.namespace,
            pod: open.pod,
            short_pod: open.short_pod,
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
                    shell.begin_shell_start(&tab, ShellCommand::Auto, cx);
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
                short_pod_name(pod),
                container.name.clone(),
            ))
        });
        let Some((namespace, pod, short_pod, container)) = found else {
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
            short_pod,
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
            Rc::new(move |shell, permit, connection, _, cx| {
                let Some(entity) = tab.upgrade() else {
                    return;
                };
                let grant = ShellGrant {
                    connection,
                    permit,
                    command,
                };
                // The session it replaces may not have reported yet.
                shell.begin_shell_start(&entity, command, cx);
                entity.update(cx, |tab, cx| tab.connect(grant, cx));
            }),
        );
        self.start_connect(intent, window, cx);
    }

    /// Writes the audit line of every session start of `tab`: applied when the exec connection
    /// came up, failed with the error when it did not.
    pub(super) fn watch_shell(&mut self, tab: &Entity<ShellTab>, cx: &mut Context<Self>) {
        cx.subscribe(tab, |shell, tab, event: &ShellEvent, cx| {
            shell.audit_shell_start(&tab, event, cx);
            // A node shell pod is deleted with its session, whichever way it ended.
            if *event == ShellEvent::Ended {
                shell.cleanup_tab(tab.entity_id(), cx);
            }
        })
        .detach();
        let id = tab.entity_id();
        cx.observe_release(tab, move |shell, _, cx| {
            shell.abandon_shell_start(id, cx);
            // The tab was closed, or released with its cluster or the whole view.
            shell.cleanup_tab(id, cx);
        })
        .detach();
    }

    /// A session of `tab` is starting with `command`: remembers the line to write if it never
    /// reports. A start still pending in the same tab is replaced, and its line is written now.
    pub(super) fn begin_shell_start(
        &mut self,
        tab: &Entity<ShellTab>,
        command: ShellCommand,
        cx: &mut Context<Self>,
    ) {
        let target = tab.read(cx).target().clone();
        let (action, object, fields) = start_audit(tab.read(cx), command);
        let Some(entry) = self.guard_for(&target.cluster, cx).map(|guard| {
            let error = Some(ABANDONED_TEXT.to_owned());
            connect_entry(
                action,
                object,
                fields,
                &guard,
                AuditOutcome::Abandoned,
                error,
            )
        }) else {
            return;
        };
        if let Some(replaced) = self.shell_starts.pending.insert(tab.entity_id(), entry) {
            self.write_audit_line(replaced, cx);
        }
        if self.shell_starts.quit.is_none() {
            self.shell_starts.quit = Some(cx.on_app_quit(|shell, cx| {
                shell.abandon_all_shell_starts(cx);
                std::future::ready(())
            }));
        }
    }

    /// The tab is gone with its start unreported.
    fn abandon_shell_start(&mut self, id: EntityId, cx: &mut Context<Self>) {
        if let Some(entry) = self.shell_starts.pending.remove(&id) {
            self.write_audit_line(entry, cx);
        }
    }

    /// The app is quitting: every unreported start is written before the process ends, so the
    /// append is synchronous (a few lines at most).
    fn abandon_all_shell_starts(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf) else {
            return;
        };
        for (_, entry) in self.shell_starts.pending.drain() {
            if let Err(error) = append_audit(&dir, &entry) {
                tracing::warn!(kind = ?error.kind(), "could not append to the audit log");
            }
        }
    }

    /// Queues the line now, so file order is call order, and reports a failed write when it
    /// finishes.
    pub(super) fn write_audit_line(&mut self, entry: AuditEntry, cx: &mut Context<Self>) {
        let config_dir = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf);
        let queued = queue_audit(config_dir.as_deref(), &entry);
        let shell = cx.weak_entity();
        cx.spawn(async move |_, cx| report_audit(queued, &shell, cx).await)
            .detach();
    }

    fn audit_shell_start(
        &mut self,
        tab: &Entity<ShellTab>,
        event: &ShellEvent,
        cx: &mut Context<Self>,
    ) {
        let (outcome, error) = match event {
            ShellEvent::Opened => (AuditOutcome::Applied, None),
            ShellEvent::OpenFailed { error } => (AuditOutcome::Failed, Some(error.clone())),
            // The end of a session is not a start: nothing reports twice.
            ShellEvent::Ended => return,
        };
        // It reported, so it is no longer pending.
        self.shell_starts.pending.remove(&tab.entity_id());
        let (target, action, object, fields) = {
            let tab = tab.read(cx);
            let (action, object, fields) = start_audit(tab, tab.command());
            (tab.target().clone(), action, object, fields)
        };
        let Some(entry) = self
            .guard_for(&target.cluster, cx)
            .map(|guard| connect_entry(action, object, fields, &guard, outcome, error))
        else {
            return;
        };
        self.write_audit_line(entry, cx);
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen shell-confirm-fixture`: the Open shell dialog of a fixed pod of a fixed Production
    /// cluster. It needs no cluster at all, skips the gate, and its confirm button and Enter do
    /// nothing (`ConfirmDialog::show_fixture`), so it can never open a session.
    pub(super) fn open_shell_confirm_fixture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::AppContext as _;

        use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
        use crate::environment::Environment;
        use crate::screenshot::{SHELL_FIXTURE_CLUSTER, shell_fixture_target};
        use crate::write_guard::{ActionRisk, ConfirmMode, confirm_step};
        let target = shell_fixture_target();
        let intent = shell_intent(
            &target,
            ShellCommand::Auto,
            SHELL_FIXTURE_CLUSTER.to_owned(),
            "Open shell",
            "Open shell",
            Rc::new(|_, _, _, _, _| {}),
        );
        let confirm = confirm_step(ConfirmMode::TypeName, ActionRisk::Change, intent.expected());
        let inputs = DialogInputs {
            shell: cx.weak_entity(),
            kind: DialogKind::Connect(Rc::new(intent)),
            confirm,
            environment: Environment::Production,
            generation: 0,
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
    open: Rc<ExecOpen>,
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
        expected_name: None,
        open: ConnectOpen::Exec(open),
    }
}

#[cfg(test)]
#[path = "shell_open_tests.rs"]
mod shell_open_tests;
