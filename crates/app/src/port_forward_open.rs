//! Starting, stopping, and auditing port forwards (spec 0035): the app-side glue between the entry
//! points (the Forward buttons, the menus, F, the palette, the page, the dialogs), the guarded flow
//! of `write_flow`, the `PortForwards` list, and the audit line.
//!
//! A child of `app_shell`, like `shell_open`: every step names the cluster of the target and takes
//! its guard, permit, and connection from that cluster's own slot, never from the primary. A
//! forward then keeps running on its own connection clone, so a switch or a released slot does not
//! end it (decision 20).

use std::collections::HashMap;
use std::rc::Rc;

use cluster::{ClusterConnection, PortForwardPermit};
use futures::channel::mpsc;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Context, SharedString, Subscription, Window};

use super::AppShell;
use super::write_flow::{ConnectIntent, ConnectOpen, PortForwardOpen};
use crate::audit_log::{
    AuditEntry, AuditField, AuditObject, AuditOutcome, append_audit, connect_entry,
};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::environment::Environment;
use crate::port_forward_menu::{ForwardSubject, MenuState, first_tcp_port, menu_state};
use crate::port_forwards::{
    ForwardId, ForwardOrigin, ForwardPreset, ForwardSpec, LocalPortSpec, MAX_RUNNING_FORWARDS,
    PortForwards, StartOutcome, StartReport, TargetSpec,
};
use crate::resource_actions::{
    ActionAvailability, ResourceAction, action_availability, action_label, action_risk,
    unavailable_text,
};
use crate::settings::AppSettings;
use crate::table_selection::{ClusterObject, ResourceKey};

/// The name an audit line gives every forward start, a restart and a retry included: each one is
/// a new forward.
const AUDIT_ACTION: &str = "Port-forward";
/// Why a start has no result: it was stopped, or a newer start replaced it, before it reported.
const ABANDONED_TEXT: &str = "the forward was stopped or replaced before it reported";

/// One confirmed start: the row to reuse, if any, and what to forward where.
struct ForwardStart {
    cluster: ClusterRef,
    spec: ForwardSpec,
    existing: Option<ForwardId>,
}

/// The starts that have not reported yet, each with the audit line to write if none ever does. A
/// request may already have reached the server when a forward is stopped, replaced by a restart,
/// or the app quits, so the start must not vanish from the log.
#[derive(Default)]
pub(super) struct ForwardStarts {
    pending: HashMap<ForwardId, AuditEntry>,
    /// Writes what is still pending when the app quits.
    quit: Option<Subscription>,
}

/// What the dialog names and the audit line records: the target and the two ports, never traffic.
fn forward_audit(spec: &ForwardSpec) -> (AuditObject, Vec<AuditField>) {
    (
        AuditObject {
            kind: spec.target.kind.object_kind().to_owned(),
            namespace: Some(spec.namespace.clone()),
            name: spec.target.name.clone(),
        },
        vec![
            AuditField {
                path: "remote_port".to_owned(),
                value: Some(spec.remote_port.to_string()),
            },
            AuditField {
                path: "local_port".to_owned(),
                value: Some(spec.requested_local_port().to_string()),
            },
        ],
    )
}

/// The intent of a forward start: the target and the two ports are what the dialog names and the
/// audit line records. A forward is a `Change` that asks the cluster's tier on every start.
fn forward_intent(
    cluster: &ClusterRef,
    cluster_name: String,
    spec: &ForwardSpec,
    open: Rc<PortForwardOpen>,
) -> ConnectIntent {
    let (object, fields) = forward_audit(spec);
    let label = format!(
        "{} {}:{}",
        action_label(ResourceAction::PortForward),
        spec.target.name,
        spec.remote_port
    );
    ConnectIntent {
        cluster: cluster.clone(),
        cluster_name: cluster_name.into(),
        action: ResourceAction::PortForward,
        label: label.into(),
        button: "Forward".into(),
        risk: action_risk(ResourceAction::PortForward),
        warnings: Vec::new(),
        object,
        fields,
        open: ConnectOpen::PortForward(open),
    }
}

fn warn(window: &mut Window, cx: &mut App, text: String) {
    window.push_notification(Notification::warning(text), cx);
}

/// The text of a start the list refuses before any dialog: a typed port another forward holds, or
/// the cap of running forwards.
fn refusal_text(
    forwards: &PortForwards,
    spec: &ForwardSpec,
    existing: Option<ForwardId>,
) -> Option<String> {
    if let LocalPortSpec::Exact(port) = spec.local_port
        && forwards.port_holder(port, existing).is_some()
    {
        return Some(format!("Port {port} is used by another forward"));
    }
    let is_running_row = existing
        .and_then(|id| forwards.get(id))
        .is_some_and(|forward| forward.state.is_running());
    (!is_running_row && forwards.running_count() >= MAX_RUNNING_FORWARDS)
        .then(|| format!("Stop a forward first ({MAX_RUNNING_FORWARDS} running)"))
}

impl AppShell {
    /// Forward `port` of the target with an automatic local port, in `cluster`: the Forward
    /// buttons, the menus, and F with one port.
    pub(crate) fn start_forward_port(
        &mut self,
        cluster: &ClusterRef,
        namespace: &str,
        target: &TargetSpec,
        port: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let spec = ForwardSpec {
            namespace: namespace.to_owned(),
            target: target.clone(),
            remote_port: port,
            local_port: LocalPortSpec::Auto,
        };
        self.start_forward(cluster, spec, None, window, cx);
    }

    /// The one entry of a forward start: the gate and the confirm tier of `cluster`'s own slot, then
    /// the stream. `existing` is the row a restart, a retry, or a preset start reuses. Nothing
    /// starts without the dialog, for every tier and trigger.
    pub(crate) fn start_forward(
        &mut self,
        cluster: &ClusterRef,
        spec: ForwardSpec,
        existing: Option<ForwardId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cluster_name) = self
            .guard_for(cluster, cx)
            .map(|guard| guard.display_name().to_owned())
        else {
            let name = self
                .cluster_label(cluster, cx)
                .unwrap_or_else(|| cluster.context.clone());
            warn(window, cx, format!("Open {name} to start this forward"));
            return;
        };
        if let Some(text) = refusal_text(self.port_forwards.read(cx), &spec, existing) {
            warn(window, cx, text);
            return;
        }
        let start = Rc::new(ForwardStart {
            cluster: cluster.clone(),
            spec,
            existing,
        });
        let open: Rc<PortForwardOpen> = {
            let start = Rc::clone(&start);
            Rc::new(move |shell, permit, connection, window, cx| {
                shell.open_forward(&start, permit, connection, window, cx);
            })
        };
        let intent = forward_intent(cluster, cluster_name, &start.spec, open);
        self.start_connect(intent, window, cx);
    }

    /// The confirmed start: adds or resets the row, opens the stream on the cluster's own
    /// connection, and remembers the audit line to write if it never reports.
    fn open_forward(
        &mut self,
        start: &ForwardStart,
        permit: PortForwardPermit,
        connection: ClusterConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ForwardStart {
            cluster,
            spec,
            existing,
        } = start;
        let existing = *existing;
        // The list may have changed while the dialog was open.
        if let Some(text) = refusal_text(self.port_forwards.read(cx), spec, existing) {
            warn(window, cx, text);
            return;
        }
        let Some((origin, abandoned)) = self.guard_for(cluster, cx).map(|guard| {
            let (object, fields) = forward_audit(spec);
            let abandoned = connect_entry(
                AUDIT_ACTION,
                object,
                fields,
                &guard,
                AuditOutcome::Abandoned,
                Some(ABANDONED_TEXT.to_owned()),
            );
            let origin = ForwardOrigin {
                cluster: cluster.clone(),
                cluster_label: self
                    .cluster_label(cluster, cx)
                    .unwrap_or_else(|| cluster.context.clone())
                    .into(),
                environment: guard.profile.environment,
            };
            (origin, abandoned)
        }) else {
            return;
        };
        if let Some(id) = existing {
            // A restart replaces a start that may not have reported.
            self.abandon_forward_start(id, cx);
        }
        let (control_sender, control_receiver) = mpsc::unbounded();
        let updates = connection.port_forward(permit, spec.request(), control_receiver);
        let runtime = cx.global::<ClusterRuntime>().clone();
        let id = self.port_forwards.update(cx, |forwards, cx| {
            let id = forwards.begin(origin, spec.clone(), existing, jiff::Timestamp::now());
            let subscription = runtime.subscribe(
                updates,
                cx,
                move |forwards, update, cx| forwards.on_update(id, update, cx),
                move |forwards, cx| forwards.on_closed(id, cx),
            );
            forwards.attach(id, subscription, control_sender);
            id
        });
        self.forward_starts.pending.insert(id, abandoned);
        self.watch_forward_quit(cx);
        cx.notify();
    }

    /// Stops the forward: the listener closes and every socket is aborted. Needs no gate and no
    /// confirm (decision 23). A start that never reported gets its `abandoned` audit line.
    pub(crate) fn stop_forward(&mut self, id: ForwardId, cx: &mut Context<Self>) {
        self.abandon_forward_start(id, cx);
        self.port_forwards.update(cx, |forwards, cx| {
            forwards.stop(id);
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn stop_all_forwards(&mut self, cx: &mut Context<Self>) {
        for id in self.port_forwards.read(cx).running_ids() {
            self.stop_forward(id, cx);
        }
    }

    /// Restart, Retry, and Start (a preset): the same guarded start on the row's own cluster.
    pub(crate) fn start_forward_again(
        &mut self,
        id: ForwardId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((cluster, spec)) = self
            .port_forwards
            .read(cx)
            .get(id)
            .map(|forward| (forward.cluster.clone(), forward.spec.clone()))
        else {
            return;
        };
        self.start_forward(&cluster, spec, Some(id), window, cx);
    }

    /// Change local port…: the row keeps the new typed port. A running forward restarts through
    /// the guarded start; any other row only stores it (and its preset).
    pub(crate) fn change_local_port(
        &mut self,
        id: ForwardId,
        port: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((cluster, mut spec, is_running, is_preset)) =
            self.port_forwards.read(cx).get(id).map(|forward| {
                (
                    forward.cluster.clone(),
                    forward.spec.clone(),
                    forward.state.is_running(),
                    forward.is_preset,
                )
            })
        else {
            return;
        };
        let old = spec.clone();
        spec.local_port = LocalPortSpec::Exact(port);
        if is_preset {
            AppSettings::update(cx, |settings| {
                for preset in &mut settings.port_forward.presets {
                    if preset.cluster == cluster && preset.spec.is_same_target(&old) {
                        preset.spec.local_port = LocalPortSpec::Exact(port);
                    }
                }
            });
        }
        if is_running {
            self.start_forward(&cluster, spec, Some(id), window, cx);
            return;
        }
        if let Some(text) = refusal_text(self.port_forwards.read(cx), &spec, Some(id)) {
            warn(window, cx, text);
            return;
        }
        self.port_forwards.update(cx, |forwards, cx| {
            forwards.set_spec(id, spec);
            cx.notify();
        });
    }

    /// Save as preset: the settings keep the forward under `port_forward.presets`.
    pub(crate) fn save_forward_preset(&mut self, id: ForwardId, cx: &mut Context<Self>) {
        let known: Vec<ForwardPreset> = AppSettings::get(cx).port_forward.presets.clone();
        let preset = self
            .port_forwards
            .update(cx, |forwards, _| forwards.save_preset(id, &known));
        if let Some(preset) = preset {
            AppSettings::update(cx, |settings| settings.port_forward.presets.push(preset));
        }
        cx.notify();
    }

    /// Remove preset…: the settings forget it; a running row keeps running as a plain forward.
    pub(crate) fn remove_forward_preset(&mut self, id: ForwardId, cx: &mut Context<Self>) {
        let Some((cluster, spec)) = self
            .port_forwards
            .read(cx)
            .get(id)
            .map(|forward| (forward.cluster.clone(), forward.spec.clone()))
        else {
            return;
        };
        AppSettings::update(cx, |settings| {
            settings
                .port_forward
                .presets
                .retain(|preset| !(preset.cluster == cluster && preset.spec.is_same_target(&spec)));
        });
        self.port_forwards
            .update(cx, |forwards, _| forwards.clear_preset(id));
        cx.notify();
    }

    /// Puts the presets of the settings in the list as `Stopped · preset` rows. Nothing starts.
    pub(super) fn sync_forward_presets(&mut self, cx: &mut Context<Self>) {
        // The fixture screens hold fixed rows that no preset may replace.
        #[cfg(feature = "screenshot")]
        if self.forward_fixture {
            return;
        }
        let presets: Vec<ForwardPreset> = AppSettings::get(cx).port_forward.presets.clone();
        let rows: Vec<(ClusterRef, SharedString, Environment)> = self
            .catalog
            .read(cx)
            .groups(cx)
            .into_iter()
            .flat_map(|group| group.rows)
            .map(|row| (row.cluster, row.label.into(), row.profile.environment))
            .collect();
        self.port_forwards.update(cx, |forwards, cx| {
            forwards.load_presets(&presets, |cluster| {
                rows.iter()
                    .find(|(known, ..)| known == cluster)
                    .map_or_else(
                        || (cluster.context.clone().into(), Environment::Staging),
                        |(_, label, environment)| (label.clone(), *environment),
                    )
            });
            cx.notify();
        });
    }

    /// The viewed cluster changed (its lock may have): its running forwards refuse new local
    /// connections while it is locked and accept them again when it is unlocked (decision 22).
    pub(super) fn sync_forward_lock(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) {
        let Some(lock) = self.guard_for(cluster, cx).map(|guard| guard.lock) else {
            return;
        };
        self.port_forwards
            .update(cx, |forwards, _| forwards.sync_lock(cluster, lock));
    }

    /// F (and the palette entry that dispatches it) on the cursor row, in the cursor's own
    /// cluster: one TCP port starts at once; several ports, or none, open New forward filled in.
    pub(crate) fn run_port_forward_key(
        &mut self,
        object: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(subject) = self.forward_subject_of(object, cx) else {
            return;
        };
        match menu_state(&subject, ActionAvailability::Enabled) {
            MenuState::Direct(choice) => {
                let (namespace, target) = (&subject.namespace, &subject.target);
                self.start_forward_port(
                    &object.cluster,
                    namespace,
                    target,
                    choice.remote_port,
                    window,
                    cx,
                );
            }
            MenuState::Pick(_) | MenuState::NewForward => {
                self.open_new_forward_for(&object.cluster, &subject, window, cx);
            }
            MenuState::Disabled(reason) => {
                let label = action_label(ResourceAction::PortForward);
                warn(window, cx, unavailable_text(label, &reason));
            }
        }
    }

    /// What `object` declares to forward, read from its own cluster's live data.
    fn forward_subject_of(&self, object: &ClusterObject, cx: &App) -> Option<ForwardSubject> {
        let live = self.slot_live(&object.cluster, cx)?;
        match &object.key {
            ResourceKey::Pod { .. } => live
                .pods
                .items()
                .iter()
                .find(|pod| object.key.is_pod(pod))
                .map(crate::port_forward_menu::pod_subject),
            ResourceKey::Kind { .. } => {
                crate::port_forward_menu::row_subject(live.row_of(&object.key)?)
            }
            ResourceKey::Node { .. } => None,
        }
    }

    /// The gate of the Start and Retry buttons of a row: the row's own cluster, never the primary.
    /// `None` when the cluster is not viewed.
    pub(crate) fn forward_start_gate(
        &self,
        cluster: &ClusterRef,
        cx: &App,
    ) -> Option<ActionAvailability> {
        let guard = self.guard_for(cluster, cx)?;
        Some(action_availability(ResourceAction::PortForward, &guard))
    }

    /// The port New forward starts from: the first TCP port of the subject.
    pub(crate) fn open_new_forward_for(
        &mut self,
        cluster: &ClusterRef,
        subject: &ForwardSubject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prefill = crate::app_shell::port_forward_dialogs::NewForwardPrefill {
            cluster: cluster.clone(),
            namespace: subject.namespace.clone(),
            target: Some(subject.target.clone()),
            remote_port: first_tcp_port(subject),
        };
        self.open_new_forward(prefill, window, cx);
    }

    // ---- audit ----

    /// The start of a forward reported: applied when the target resolved and the port is bound,
    /// failed with the error when it ended first. One line per start; never traffic.
    pub(super) fn audit_forward_start(&mut self, report: &StartReport, cx: &mut Context<Self>) {
        let Some(mut entry) = self.forward_starts.pending.remove(&report.id) else {
            return;
        };
        match &report.outcome {
            StartOutcome::Up => {
                entry.outcome = AuditOutcome::Applied;
                entry.error = None;
                // An automatic port may have moved: the line records the port that was bound.
                let bound = self
                    .port_forwards
                    .read(cx)
                    .get(report.id)
                    .and_then(|forward| forward.local);
                if let Some(local) = bound
                    && let Some(field) = entry
                        .fields
                        .iter_mut()
                        .find(|field| field.path == "local_port")
                {
                    field.value = Some(local.port().to_string());
                }
            }
            StartOutcome::Failed(error) => {
                entry.outcome = AuditOutcome::Failed;
                entry.error = Some(error.clone());
            }
        }
        self.write_audit_line(entry, cx);
    }

    /// The forward is gone or replaced with its start unreported.
    fn abandon_forward_start(&mut self, id: ForwardId, cx: &mut Context<Self>) {
        if let Some(entry) = self.forward_starts.pending.remove(&id) {
            self.write_audit_line(entry, cx);
        }
    }

    fn watch_forward_quit(&mut self, cx: &mut Context<Self>) {
        if self.forward_starts.quit.is_some() {
            return;
        }
        self.forward_starts.quit = Some(cx.on_app_quit(|shell, cx| {
            shell.abandon_all_forward_starts(cx);
            std::future::ready(())
        }));
    }

    /// The app is quitting: every unreported start is written before the process ends, so the
    /// append is synchronous (a few lines at most).
    fn abandon_all_forward_starts(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf) else {
            return;
        };
        for (_, entry) in self.forward_starts.pending.drain() {
            if let Err(error) = append_audit(&dir, &entry) {
                tracing::warn!(kind = ?error.kind(), "could not append to the audit log");
            }
        }
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen port-forward-confirm-fixture`: the forward dialog of a fixed pod of a fixed
    /// Production cluster. It needs no cluster at all, skips the gate, and its confirm button and
    /// Enter do nothing (`ConfirmDialog::show_fixture`), so it can never start a forward.
    pub(super) fn open_forward_confirm_fixture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::AppContext as _;

        use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
        use crate::screenshot::forward_fixture_clusters;
        use crate::write_guard::{ActionRisk, ConfirmMode, confirm_step};
        let ((cluster, label), _) = forward_fixture_clusters();
        let spec = ForwardSpec {
            namespace: "payments".to_owned(),
            target: TargetSpec::pod("postgres-0"),
            remote_port: 5432,
            local_port: LocalPortSpec::Auto,
        };
        let intent = forward_intent(
            &cluster,
            label.to_owned(),
            &spec,
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

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen port-forwards`: the five rows of the W7 page, with the drawer of the first open.
    /// Nothing starts and nothing is requested.
    pub(super) fn fill_forward_fixture(&mut self, has_drawer: bool, cx: &mut Context<Self>) {
        let rows = crate::screenshot::forward_fixtures(jiff::Timestamp::now());
        self.port_forwards.update(cx, |forwards, cx| {
            let ids: Vec<ForwardId> = rows
                .into_iter()
                .map(|row| forwards.insert_fixture(row))
                .collect();
            forwards.select(ids.first().copied().filter(|_| has_drawer));
            cx.notify();
        });
    }

    /// `--screen port-forward-remove-fixture`: the Remove preset dialog of the fixed preset row.
    pub(super) fn open_remove_preset_fixture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preset = self
            .port_forwards
            .read(cx)
            .forwards()
            .iter()
            .find(|forward| forward.is_preset)
            .map(|forward| forward.id);
        if let Some(id) = preset {
            self.open_remove_preset(id, window, cx);
        }
    }
}

/// The target a row's Go to target reveals: the object of the forward in its own cluster.
pub(crate) fn target_key(spec: &ForwardSpec) -> Option<ResourceKey> {
    ResourceKey::of_object(
        spec.target.kind.object_kind(),
        Some(&spec.namespace),
        &spec.target.name,
    )
}

#[cfg(test)]
#[path = "port_forward_open_tests.rs"]
mod port_forward_open_tests;
