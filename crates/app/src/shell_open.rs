//! Opening and reopening shell tabs (spec 0036): the app-side glue between the entry points (S, the
//! menus, the palette, the dock, Reconnect), the guarded flow of `write_flow`, the dock, and the
//! audit line.
//!
//! A child of `app_shell`, like `write_flow`: every step names the cluster of the pod and takes
//! its guard, permit, and connection from that cluster's own slot, never from the primary.

use std::collections::HashMap;
use std::rc::Rc;

use cluster::{ContainerTerminal, ShellCommand};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{Context, Entity, EntityId, Subscription, WeakEntity, Window};

use super::AppShell;
use super::write_flow::{
    ConnectIntent, ConnectOpen, ContainerAttachOpen, ExecOpen, queue_audit, report_audit,
};
use crate::audit_log::{
    AuditEntry, AuditField, AuditObject, AuditOutcome, append_audit, connect_entry,
};
use crate::cluster_registry::ClusterRef;
use crate::dock::shell_cap_text;
use crate::log_workload::pod_tab_name;
use crate::resource_actions::{
    DefaultShell, ResourceAction, action_label, action_risk, default_attach_container,
    default_shell, open_shell_picker,
};
use crate::settings::AppSettings;
use crate::shell_tab::{AttachGrant, ShellEvent, ShellGrant, ShellKind, ShellTab, ShellTarget};
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
/// The audit action of an attach start: each press is its own start and its own line.
const ATTACH_AUDIT_ACTION: &str = "Attach";
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
        ShellKind::Attach => (
            ATTACH_AUDIT_ACTION,
            pod(),
            vec![field("container", &target.container)],
        ),
    }
}

/// What the main process of the container receives from an attach, and what can stop it.
const ATTACH_WARNING: &str = "What you type goes to the main process of {container}; Ctrl C, Ctrl D or exit may stop it, and the container restarts.";
/// A container with `stdinOnce` closes its input after the first attach.
const ATTACH_ONCE_WARNING: &str = "This container closes its input after one attach (stdinOnce): closing the tab ends its process.";

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
            self.label_of(&open.cluster),
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
        // Read once: the dialog, the grant, and the start must all name the same shell.
        let command = AppSettings::get(cx).terminal.default_shell;
        let intent = shell_intent(
            &target,
            command,
            cluster_name,
            "Open shell",
            "Open shell",
            Rc::new(move |shell, permit, connection, window, cx| {
                let grant = ShellGrant {
                    connection,
                    permit,
                    command,
                };
                let tab = dock.update(cx, |dock, cx| {
                    dock.open_shell(opened.clone(), tab_label.clone(), grant, window, cx)
                });
                if let Some(tab) = tab {
                    shell.watch_shell(&tab, cx);
                    shell.begin_shell_start(&tab, command, cx);
                }
            }),
        );
        self.start_connect(intent, window, cx);
    }

    /// S on a pod: its default container (the first running main one, else the first running one), in
    /// the cluster of the cursor row. A pod with several containers shows the container picker of
    /// the Open shell submenu first.
    pub(crate) fn open_default_shell(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ResourceKey::Pod { .. } = &subject.key else {
            return;
        };
        let found = self.live_of(&subject.cluster, cx).and_then(|live| {
            let pod = live
                .pods
                .items()
                .iter()
                .find(|pod| subject.key.is_pod(pod))?;
            let open = ShellOpen {
                cluster: subject.cluster.clone(),
                namespace: pod.namespace.clone(),
                pod: pod.name.clone(),
                short_pod: pod_tab_name(pod),
                container: String::new(),
            };
            Some((open, default_shell(pod)))
        });
        match found {
            Some((open, DefaultShell::Open(container))) => {
                self.start_shell(ShellOpen { container, ..open }, window, cx);
            }
            Some((open, DefaultShell::Pick(choices))) => {
                let shell = cx.entity().downgrade();
                open_shell_picker(
                    choices,
                    move |container| {
                        let open = ShellOpen {
                            container,
                            ..open.clone()
                        };
                        let shell = shell.clone();
                        Box::new(move |window, cx| {
                            // The confirm opens its own dialog, so the picker goes first.
                            window.close_dialog(cx);
                            let open = open.clone();
                            let _ =
                                shell.update(cx, |shell, cx| shell.start_shell(open, window, cx));
                        })
                    },
                    window,
                    cx,
                );
            }
            Some((_, DefaultShell::Unavailable)) | None => {
                let text = format!(
                    "{} is unavailable: No running container",
                    action_label(ResourceAction::OpenShell)
                );
                window.push_notification(Notification::warning(text), cx);
            }
        }
    }

    /// Attaches to `open.container` through the guarded flow of the pod's own cluster (spec 0040):
    /// the gate, the tier dialog (the cluster's own), then a new Attach tab and its audit line. No
    /// dry-run: an attach changes nothing on the server.
    pub(crate) fn start_attach(
        &mut self,
        open: ShellOpen,
        terminal: ContainerTerminal,
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
            self.label_of(&open.cluster),
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
        let intent = attach_intent(
            &target,
            terminal,
            cluster_name,
            Rc::new(move |shell, permit, connection, window, cx| {
                let grant = AttachGrant { connection, permit };
                let tab = dock.update(cx, |dock, cx| {
                    dock.open_attach(
                        opened.clone(),
                        ShellKind::Attach,
                        tab_label.clone(),
                        grant,
                        window,
                        cx,
                    )
                });
                if let Some(tab) = tab {
                    shell.watch_shell(&tab, cx);
                    shell.begin_shell_start(&tab, ShellCommand::Auto, cx);
                }
            }),
        );
        self.start_connect(intent, window, cx);
    }

    /// A on a pod: its default container with a terminal, in the cluster of the cursor row.
    pub(crate) fn attach_default(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ResourceKey::Pod { .. } = &subject.key else {
            return;
        };
        let found = self.live_of(&subject.cluster, cx).and_then(|live| {
            let pod = live
                .pods
                .items()
                .iter()
                .find(|pod| subject.key.is_pod(pod))?;
            let container = default_attach_container(pod).ok()?;
            Some((
                ShellOpen {
                    cluster: subject.cluster.clone(),
                    namespace: pod.namespace.clone(),
                    pod: pod.name.clone(),
                    short_pod: pod_tab_name(pod),
                    container: container.name.clone(),
                },
                container.terminal,
            ))
        });
        let Some((open, terminal)) = found else {
            let text = format!(
                "{} is unavailable: No running container has a terminal (stdin and tty); use View logs",
                action_label(ResourceAction::Attach)
            );
            window.push_notification(Notification::warning(text), cx);
            return;
        };
        self.start_attach(open, terminal, window, cx);
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
    /// `--screen shell-confirm-fixture` and `--screen attach-confirm`: the Open shell or Attach
    /// dialog of a fixed pod of a fixed Production cluster. It needs no cluster at all, skips the
    /// gate, and its confirm button and Enter do nothing (`ConfirmDialog::show_fixture`), so it can
    /// never open a session.
    pub(super) fn open_shell_confirm_fixture(
        &mut self,
        launch: crate::launch_options::LaunchScreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::AppContext as _;

        use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
        use crate::environment::Environment;
        use crate::screenshot::{SHELL_FIXTURE_CLUSTER, shell_fixture_target};
        use crate::write_guard::{ActionRisk, ConfirmMode, confirm_step};
        let target = shell_fixture_target();
        let intent = if launch == crate::launch_options::LaunchScreen::AttachConfirm {
            attach_intent(
                &target,
                ContainerTerminal::InteractiveOnce,
                SHELL_FIXTURE_CLUSTER.to_owned(),
                Rc::new(|_, _, _, _, _| {}),
            )
        } else {
            shell_intent(
                &target,
                ShellCommand::Auto,
                SHELL_FIXTURE_CLUSTER.to_owned(),
                "Open shell",
                "Open shell",
                Rc::new(|_, _, _, _, _| {}),
            )
        };
        let confirm = confirm_step(ConfirmMode::TypeName, ActionRisk::Change, intent.expected());
        let inputs = DialogInputs {
            shell: cx.weak_entity(),
            kind: DialogKind::Connect(Rc::new(intent)),
            confirm,
            environment: Environment::PRODUCTION,
            generation: 0,
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        dialog.update(cx, |dialog, _| dialog.show_fixture());
        ConfirmDialog::open(&dialog, window, cx);
    }
}

/// The intent of an attach: a `Change` on the cluster's own tier, with the main-process warning.
fn attach_intent(
    target: &ShellTarget,
    terminal: ContainerTerminal,
    cluster_name: String,
    open: Rc<ContainerAttachOpen>,
) -> ConnectIntent {
    let mut warnings = vec![
        ATTACH_WARNING
            .replace("{container}", &target.container)
            .into(),
    ];
    if terminal == ContainerTerminal::InteractiveOnce {
        warnings.push(ATTACH_ONCE_WARNING.into());
    }
    ConnectIntent {
        cluster: target.cluster.clone(),
        cluster_name: cluster_name.into(),
        action: ResourceAction::Attach,
        label: format!("Attach to {}/{}", target.pod, target.container).into(),
        button: action_label(ResourceAction::Attach).into(),
        risk: action_risk(ResourceAction::Attach),
        warnings,
        object: AuditObject {
            kind: "Pod".to_owned(),
            namespace: Some(target.namespace.clone()),
            name: target.pod.clone(),
        },
        fields: vec![AuditField {
            path: "container".to_owned(),
            value: Some(target.container.clone()),
        }],
        open: ConnectOpen::Attach(open),
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
        open: ConnectOpen::Exec(open),
    }
}

#[cfg(test)]
#[path = "shell_open_tests.rs"]
mod shell_open_tests;
