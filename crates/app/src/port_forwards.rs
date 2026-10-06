//! The port forwards of the app (spec 0035): one list for every cluster, owned by `AppShell`. A
//! running forward holds its own stream, which owns a `ClusterConnection` clone, so a cluster
//! switch or a released slot does not stop it. The state transitions are pure (`apply`) and tested
//! without a window; the guarded start lives in `app_shell/port_forward_open.rs`.

use std::collections::VecDeque;
use std::net::SocketAddrV4;

use cluster::{
    ForwardControl, ForwardError, ForwardEvent, ForwardRequest, ForwardTarget, ForwardTraffic,
    ForwardUpdate, LocalPort, default_local_port,
};
use futures::channel::mpsc::UnboundedSender;
use gpui_kit::{Context, EventEmitter, SharedString};
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::WatchSubscription;
use crate::environment::Environment;
use crate::status_tone::{StatusLabel, StatusTone};
use crate::write_guard::WriteLock;

/// Each forward is a listener and up to 64 WebSockets to an API server, so the list is capped.
pub(crate) const MAX_RUNNING_FORWARDS: usize = 20;
/// The recent events a forward keeps, newest last.
const MAX_EVENTS: usize = 20;
/// The reconnect tries the transport makes (spec decision 13); the status reads `n/5`.
const RECONNECT_TRIES: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ForwardId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TargetKind {
    Pod,
    Service,
    Deployment,
    StatefulSet,
}

impl TargetKind {
    /// The short word of the Target column and of the New forward field: `svc/payments-api`.
    pub(crate) fn short(self) -> &'static str {
        match self {
            Self::Pod => "pod",
            Self::Service => "svc",
            Self::Deployment => "deploy",
            Self::StatefulSet => "sts",
        }
    }

    /// The Kubernetes kind: what the audit line names and Go to target reveals.
    pub(crate) fn object_kind(self) -> &'static str {
        match self {
            Self::Pod => "Pod",
            Self::Service => "Service",
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct TargetSpec {
    pub(crate) kind: TargetKind,
    pub(crate) name: String,
}

impl TargetSpec {
    pub(crate) fn pod(name: impl Into<String>) -> Self {
        Self {
            kind: TargetKind::Pod,
            name: name.into(),
        }
    }

    fn cluster_target(&self) -> ForwardTarget {
        let name = self.name.clone();
        match self.kind {
            TargetKind::Pod => ForwardTarget::Pod { name },
            TargetKind::Service => ForwardTarget::Service { name },
            TargetKind::Deployment => ForwardTarget::Deployment { name },
            TargetKind::StatefulSet => ForwardTarget::StatefulSet { name },
        }
    }
}

/// The local port the user asked for. A typed port (dialog, preset) is `Exact` and never moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LocalPortSpec {
    Auto,
    Exact(u16),
}

impl LocalPortSpec {
    fn typed(self) -> Option<u16> {
        match self {
            Self::Auto => None,
            Self::Exact(port) => Some(port),
        }
    }
}

/// What the transport listens on: a typed port is `Exact`, none starts from the default for the
/// remote port.
pub(crate) fn local_port_for(typed: Option<u16>, remote: u16) -> LocalPort {
    match typed {
        Some(port) => LocalPort::Exact(port),
        None => LocalPort::Auto(default_local_port(remote)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ForwardSpec {
    pub(crate) namespace: String,
    pub(crate) target: TargetSpec,
    pub(crate) remote_port: u16,
    pub(crate) local_port: LocalPortSpec,
}

impl ForwardSpec {
    pub(crate) fn request(&self) -> ForwardRequest {
        ForwardRequest {
            namespace: self.namespace.clone(),
            target: self.target.cluster_target(),
            remote_port: self.remote_port,
            local_port: local_port_for(self.local_port.typed(), self.remote_port),
        }
    }

    /// `payments/svc/payments-api`.
    pub(crate) fn target_text(&self) -> String {
        format!(
            "{}/{}/{}",
            self.namespace,
            self.target.kind.short(),
            self.target.name
        )
    }

    /// `svc/payments-api`, as the New forward field takes it.
    pub(crate) fn short_target_text(&self) -> String {
        format!("{}/{}", self.target.kind.short(), self.target.name)
    }

    /// The port the row shows before anything is bound.
    pub(crate) fn requested_local_port(&self) -> u16 {
        self.local_port
            .typed()
            .unwrap_or_else(|| default_local_port(self.remote_port))
    }

    /// The same forward, whatever its local port: how presets are told apart.
    pub(crate) fn is_same_target(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.target == other.target
            && self.remote_port == other.remote_port
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ForwardState {
    Starting,
    Active,
    /// The cluster is locked: new local connections are refused, open ones continue.
    Paused,
    Reconnecting {
        attempt: u8,
    },
    Failed(ForwardFailure),
    Stopped,
}

impl ForwardState {
    /// Whether a stream is (or is about to be) up.
    pub(crate) fn is_running(&self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Active | Self::Paused | Self::Reconnecting { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ForwardFailure {
    PortInUse(u16),
    PortReserved(u16),
    NotPermitted,
    TargetLost,
    Other(SharedString),
}

fn failure_of(error: &ForwardError) -> ForwardFailure {
    match error {
        ForwardError::PortInUse(port) => ForwardFailure::PortInUse(*port),
        ForwardError::PortReserved(port) => ForwardFailure::PortReserved(*port),
        ForwardError::NotPermitted => ForwardFailure::NotPermitted,
        ForwardError::TargetLost => ForwardFailure::TargetLost,
        other => ForwardFailure::Other(other.to_string().into()),
    }
}

impl ForwardFailure {
    fn text(&self) -> SharedString {
        match self {
            Self::PortInUse(port) => format!("Port {port} in use").into(),
            Self::PortReserved(port) => {
                format!("Port {port} is reserved or needs more rights").into()
            }
            Self::NotPermitted => "Not permitted".into(),
            Self::TargetLost => "Target lost".into(),
            Self::Other(text) => text.clone(),
        }
    }
}

/// One line of the drawer's Recent events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ForwardLogLine {
    pub(crate) at: jiff::Timestamp,
    pub(crate) text: SharedString,
    pub(crate) tone: Option<StatusTone>,
}

impl ForwardLogLine {
    /// `09:12`, in the local time zone.
    pub(crate) fn time_text(&self) -> String {
        self.at
            .to_zoned(TimeZone::system())
            .strftime("%H:%M")
            .to_string()
    }
}

/// The resources of a running forward. Dropping it drops the subscription, which closes the
/// listener and aborts every socket.
struct ForwardRun {
    _subscription: WatchSubscription,
    control: UnboundedSender<ForwardControl>,
    /// The last control sent, so a lock change is sent once.
    sent: ForwardControl,
}

pub(crate) struct Forward {
    pub(crate) id: ForwardId,
    pub(crate) cluster: ClusterRef,
    /// The switcher label at the time of the start, for rows of clusters no longer viewed.
    pub(crate) cluster_label: SharedString,
    pub(crate) environment: Environment,
    pub(crate) spec: ForwardSpec,
    pub(crate) state: ForwardState,
    pub(crate) local: Option<SocketAddrV4>,
    pub(crate) pod: Option<String>,
    pub(crate) traffic: ForwardTraffic,
    pub(crate) events: VecDeque<ForwardLogLine>,
    pub(crate) started_at: Option<jiff::Timestamp>,
    pub(crate) is_preset: bool,
    /// The cluster is locked: the transport refuses new connections, whatever else happens.
    is_refusing: bool,
    run: Option<ForwardRun>,
}

impl Forward {
    pub(crate) fn status(&self) -> StatusLabel {
        let (text, tone): (SharedString, StatusTone) = match &self.state {
            ForwardState::Starting => ("Starting…".into(), StatusTone::Info),
            ForwardState::Active => ("Active".into(), StatusTone::Ok),
            ForwardState::Paused => ("Paused · read-only".into(), StatusTone::Warn),
            ForwardState::Reconnecting { attempt } => (
                format!("Reconnecting {attempt}/{RECONNECT_TRIES}").into(),
                StatusTone::Warn,
            ),
            ForwardState::Failed(failure) => (failure.text(), StatusTone::Bad),
            ForwardState::Stopped => ("Stopped · preset".into(), StatusTone::Done),
        };
        StatusLabel { text, tone }
    }

    /// `5432 → localhost:15432`; `—` for the local port while the listener is not bound yet.
    pub(crate) fn ports_text(&self) -> String {
        let remote = self.spec.remote_port;
        match (&self.state, self.local) {
            (_, Some(local)) => format!("{remote} → localhost:{}", local.port()),
            (ForwardState::Starting, None) => format!("{remote} → localhost:—"),
            _ => format!("{remote} → localhost:{}", self.spec.requested_local_port()),
        }
    }

    /// `2h 14m`, and `—` unless the forward is carrying traffic or about to.
    pub(crate) fn uptime_text(&self, now: jiff::Timestamp) -> String {
        match (&self.state, self.started_at) {
            (ForwardState::Active | ForwardState::Paused, Some(started)) => {
                uptime_text(now.as_second() - started.as_second())
            }
            _ => "—".to_owned(),
        }
    }

    /// The address the forward listens on, once it does.
    pub(crate) fn local_address_text(&self) -> Option<String> {
        self.local
            .map(|local| format!("127.0.0.1:{}", local.port()))
    }
}

/// `2h 14m`, `38m`, `<1m`; days past a day (`3d 4h`).
pub(crate) fn uptime_text(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    let (days, hours, minutes) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (days, hours, minutes) {
        (0, 0, 0) => "<1m".to_owned(),
        (0, 0, minutes) => format!("{minutes}m"),
        (0, hours, minutes) => format!("{hours}h {minutes}m"),
        (days, hours, _) => format!("{days}d {hours}h"),
    }
}

/// Decimal units like the wireframe: `182 MB`, `9.4 MB`.
pub(crate) fn byte_count_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000. && unit + 1 < UNITS.len() {
        value /= 1000.;
        unit += 1;
    }
    if unit == 0 || value >= 10. {
        format!("{} {}", value.round() as u64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A forward kept in the settings (`port_forward.presets`): the cluster and the spec, names and
/// numbers only. Presets never start by themselves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ForwardPreset {
    pub(crate) cluster: ClusterRef,
    pub(crate) spec: ForwardSpec,
}

impl ForwardPreset {
    /// Names that can be object names and a port that can be bound. The settings file is edited by
    /// hand, and the transport puts the names into a request path unescaped.
    fn is_valid(&self) -> bool {
        let spec = &self.spec;
        is_dns_subdomain(&spec.namespace)
            && is_dns_subdomain(&spec.target.name)
            && spec.remote_port != 0
            && spec.local_port != LocalPortSpec::Exact(0)
    }
}

/// What the start of a forward reported first: the target resolved and the port is bound, or the
/// forward ended before that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartOutcome {
    Up,
    Failed(String),
}

/// Tells the shell that the start of `id` reported, for the audit line.
pub(crate) struct StartReport {
    pub(crate) id: ForwardId,
    pub(crate) outcome: StartOutcome,
}

/// The new row's identity: everything a row needs before the transport says anything.
pub(crate) struct ForwardOrigin {
    pub(crate) cluster: ClusterRef,
    pub(crate) cluster_label: SharedString,
    pub(crate) environment: Environment,
}

pub(crate) struct PortForwards {
    forwards: Vec<Forward>,
    next_id: u64,
    /// The row whose drawer is open.
    selected: Option<ForwardId>,
}

impl EventEmitter<StartReport> for PortForwards {}

impl PortForwards {
    pub(crate) fn new() -> Self {
        Self {
            forwards: Vec::new(),
            next_id: 1,
            selected: None,
        }
    }

    pub(crate) fn forwards(&self) -> &[Forward] {
        &self.forwards
    }

    pub(crate) fn get(&self, id: ForwardId) -> Option<&Forward> {
        self.forwards.iter().find(|forward| forward.id == id)
    }

    fn get_mut(&mut self, id: ForwardId) -> Option<&mut Forward> {
        self.forwards.iter_mut().find(|forward| forward.id == id)
    }

    pub(crate) fn selected(&self) -> Option<&Forward> {
        self.get(self.selected?)
    }

    pub(crate) fn select(&mut self, id: Option<ForwardId>) {
        self.selected = id;
    }

    /// The forwards with a stream up or starting: the status bar and the sidebar count them.
    pub(crate) fn running_count(&self) -> usize {
        self.forwards
            .iter()
            .filter(|forward| forward.state.is_running())
            .count()
    }

    /// The running forwards that are not saved presets: a preset starts again with the app, the
    /// others end for good when the app or the cluster is left.
    pub(crate) fn running_unsaved(&self) -> impl Iterator<Item = &Forward> {
        self.forwards
            .iter()
            .filter(|forward| forward.state.is_running() && !forward.is_preset)
    }

    /// A running forward of this exact target and remote port in `cluster`: what a Forward button
    /// shows as live.
    pub(crate) fn running_for(
        &self,
        cluster: &ClusterRef,
        namespace: &str,
        target: &TargetSpec,
        remote_port: u16,
    ) -> Option<&Forward> {
        self.forwards.iter().find(|forward| {
            forward.state.is_running()
                && forward.cluster == *cluster
                && forward.spec.namespace == namespace
                && forward.spec.target == *target
                && forward.spec.remote_port == remote_port
        })
    }

    /// Another running forward that holds (or is about to hold) `port`, for a port the user typed.
    pub(crate) fn port_holder(&self, port: u16, except: Option<ForwardId>) -> Option<&Forward> {
        self.forwards.iter().find(|forward| {
            forward.state.is_running()
                && Some(forward.id) != except
                && forward.local.map_or_else(
                    || forward.spec.local_port == LocalPortSpec::Exact(port),
                    |local| local.port() == port,
                )
        })
    }

    /// The rows in page order: running first, then by target.
    pub(crate) fn sorted(&self, filter: &str) -> Vec<&Forward> {
        let needle = filter.trim().to_lowercase();
        let mut rows: Vec<&Forward> = self
            .forwards
            .iter()
            .filter(|forward| {
                needle.is_empty()
                    || forward.spec.target_text().to_lowercase().contains(&needle)
                    || forward.cluster_label.to_lowercase().contains(&needle)
            })
            .collect();
        rows.sort_by_cached_key(|forward| {
            (
                !forward.state.is_running(),
                forward.spec.target_text(),
                forward.spec.remote_port,
            )
        });
        rows
    }

    /// Starts (or restarts) the row of `existing`, or adds a new one, in `Starting`.
    pub(crate) fn begin(
        &mut self,
        origin: ForwardOrigin,
        spec: ForwardSpec,
        existing: Option<ForwardId>,
        now: jiff::Timestamp,
    ) -> ForwardId {
        let id = match existing.filter(|id| self.get(*id).is_some()) {
            Some(id) => id,
            None => {
                let id = ForwardId(self.next_id);
                self.next_id += 1;
                self.forwards.push(Forward {
                    id,
                    cluster: origin.cluster.clone(),
                    cluster_label: origin.cluster_label.clone(),
                    environment: origin.environment.clone(),
                    spec: spec.clone(),
                    state: ForwardState::Starting,
                    local: None,
                    pod: None,
                    traffic: ForwardTraffic::default(),
                    events: VecDeque::new(),
                    started_at: None,
                    is_preset: false,
                    is_refusing: false,
                    run: None,
                });
                id
            }
        };
        if let Some(forward) = self.get_mut(id) {
            forward.cluster_label = origin.cluster_label;
            forward.environment = origin.environment;
            forward.spec = spec;
            forward.state = ForwardState::Starting;
            forward.local = None;
            forward.pod = None;
            forward.traffic = ForwardTraffic::default();
            forward.started_at = None;
            forward.is_refusing = false;
            forward.run = None;
            let text = format!("started for {}", forward.spec.short_target_text());
            forward.push_event(now, text, None);
        }
        id
    }

    /// Hands the running stream to the row. A stop before this drops it with the row.
    pub(crate) fn attach(
        &mut self,
        id: ForwardId,
        subscription: WatchSubscription,
        control: UnboundedSender<ForwardControl>,
    ) {
        if let Some(forward) = self.get_mut(id) {
            forward.run = Some(ForwardRun {
                _subscription: subscription,
                control,
                sent: ForwardControl::Accept,
            });
        }
    }

    /// The row for the stream's update. Returns the start report when this update is the first
    /// word on whether the start worked.
    pub(crate) fn apply(
        &mut self,
        id: ForwardId,
        update: ForwardUpdate,
        now: jiff::Timestamp,
    ) -> Option<StartOutcome> {
        let forward = self.get_mut(id)?;
        let was_starting = forward.state == ForwardState::Starting;
        match update {
            ForwardUpdate::Listening { local, .. } => forward.local = Some(local),
            ForwardUpdate::Resolved { pod, .. } => {
                forward.pod = Some(pod);
                forward.started_at.get_or_insert(now);
                forward.state = forward.steady_state();
            }
            ForwardUpdate::Traffic(traffic) => forward.traffic = traffic,
            ForwardUpdate::Event(event) => forward.apply_event(event, now),
            ForwardUpdate::Ended(error) => {
                let failure = failure_of(&error);
                let text = format!("ended: {}", failure.text());
                forward.push_event(now, text, Some(StatusTone::Bad));
                forward.state = ForwardState::Failed(failure);
                forward.local = None;
                forward.run = None;
                return was_starting.then(|| StartOutcome::Failed(error.to_string()));
            }
        }
        let is_up = forward.state != ForwardState::Starting;
        (was_starting && is_up).then_some(StartOutcome::Up)
    }

    /// `apply` for the subscription: reports the start to the shell and repaints. A traffic sample
    /// repaints only while its drawer is open, because it changes nothing else on screen.
    pub(crate) fn on_update(
        &mut self,
        id: ForwardId,
        update: ForwardUpdate,
        cx: &mut Context<Self>,
    ) {
        let is_traffic = matches!(update, ForwardUpdate::Traffic(_));
        if let Some(outcome) = self.apply(id, update, jiff::Timestamp::now()) {
            cx.emit(StartReport { id, outcome });
        }
        if !is_traffic || self.selected == Some(id) {
            cx.notify();
        }
    }

    /// The stream ended without saying why: its owner dropped it, or the runtime stopped.
    pub(crate) fn on_closed(&mut self, id: ForwardId, cx: &mut Context<Self>) {
        let Some(forward) = self.get_mut(id) else {
            return;
        };
        if !forward.state.is_running() {
            return;
        }
        let was_starting = forward.state == ForwardState::Starting;
        let failure = ForwardFailure::Other("The forward stopped".into());
        forward.push_event(
            jiff::Timestamp::now(),
            "ended: the forward stopped".to_owned(),
            Some(StatusTone::Bad),
        );
        forward.state = ForwardState::Failed(failure);
        forward.local = None;
        forward.run = None;
        if was_starting {
            cx.emit(StartReport {
                id,
                outcome: StartOutcome::Failed("the forward stopped".to_owned()),
            });
        }
        cx.notify();
    }

    /// Stops the forward: the listener closes and every socket is aborted. A preset row stays as
    /// `Stopped · preset`; any other row is removed. `false` for an unknown row.
    pub(crate) fn stop(&mut self, id: ForwardId) -> bool {
        let Some(index) = self.forwards.iter().position(|forward| forward.id == id) else {
            return false;
        };
        if self.forwards[index].is_preset {
            let forward = &mut self.forwards[index];
            forward.run = None;
            forward.local = None;
            forward.state = ForwardState::Stopped;
            forward.traffic = ForwardTraffic::default();
            forward.started_at = None;
        } else {
            self.forwards.remove(index);
            if self.selected == Some(id) {
                self.selected = None;
            }
        }
        true
    }

    /// The ids of every running forward, for Stop all.
    pub(crate) fn running_ids(&self) -> Vec<ForwardId> {
        self.forwards
            .iter()
            .filter(|forward| forward.state.is_running())
            .map(|forward| forward.id)
            .collect()
    }

    /// Sends the lock of `cluster` to its running forwards: a locked cluster refuses new local
    /// connections, an unlocked one accepts them again. A control is sent once per change.
    pub(crate) fn sync_lock(&mut self, cluster: &ClusterRef, lock: WriteLock) {
        let wanted = match lock {
            WriteLock::Locked => ForwardControl::Refuse,
            WriteLock::Unlocked => ForwardControl::Accept,
        };
        for forward in &mut self.forwards {
            if forward.cluster != *cluster {
                continue;
            }
            let Some(run) = forward.run.as_mut() else {
                continue;
            };
            if run.sent != wanted && run.control.unbounded_send(wanted).is_ok() {
                run.sent = wanted;
            }
        }
    }

    /// Marks the row as a preset; `Some` with what to store unless the settings hold it already.
    pub(crate) fn save_preset(
        &mut self,
        id: ForwardId,
        presets: &[ForwardPreset],
    ) -> Option<ForwardPreset> {
        let forward = self.get_mut(id)?;
        forward.is_preset = true;
        let preset = ForwardPreset {
            cluster: forward.cluster.clone(),
            spec: ForwardSpec {
                // A preset keeps the port the forward really listens on, as a typed one.
                local_port: LocalPortSpec::Exact(
                    forward
                        .local
                        .map_or_else(|| forward.spec.requested_local_port(), |a| a.port()),
                ),
                ..forward.spec.clone()
            },
        };
        let is_known = presets.iter().any(|known| {
            known.cluster == preset.cluster && known.spec.is_same_target(&preset.spec)
        });
        (!is_known).then_some(preset)
    }

    /// Stores a new spec on a row that is not running (Change local port… on a stopped row).
    pub(crate) fn set_spec(&mut self, id: ForwardId, spec: ForwardSpec) {
        if let Some(forward) = self.get_mut(id) {
            forward.spec = spec;
        }
    }

    /// The row stops being a preset. A stopped row goes away; a running one keeps running.
    pub(crate) fn clear_preset(&mut self, id: ForwardId) {
        let Some(forward) = self.get_mut(id) else {
            return;
        };
        forward.is_preset = false;
        if forward.state == ForwardState::Stopped {
            self.stop(id);
        }
    }

    /// Replaces the `Stopped` preset rows with `presets` (the settings changed elsewhere, or the
    /// app just started). Running rows are untouched, and a preset a running row already shows is
    /// not listed twice. `describe` names the cluster of a preset; a stopped row takes it again
    /// (the catalog may have loaded since) and the preset's spec (its port may have been edited).
    /// A preset that is not valid is dropped: the transport puts its names into a request path.
    pub(crate) fn load_presets(
        &mut self,
        presets: &[ForwardPreset],
        describe: impl Fn(&ClusterRef) -> (SharedString, Environment),
    ) {
        let presets: Vec<&ForwardPreset> =
            presets.iter().filter(|preset| preset.is_valid()).collect();
        let listed = |forward: &Forward| {
            presets.iter().copied().find(|preset| {
                preset.cluster == forward.cluster && preset.spec.is_same_target(&forward.spec)
            })
        };
        // A stopped row of a preset that is gone goes away; the others keep their place and
        // their open drawer.
        self.forwards.retain(|forward| {
            !(forward.is_preset && forward.state == ForwardState::Stopped)
                || listed(forward).is_some()
        });
        for forward in &mut self.forwards {
            let preset = listed(forward);
            forward.is_preset = preset.is_some();
            if let Some(preset) = preset.filter(|_| forward.state == ForwardState::Stopped) {
                (forward.cluster_label, forward.environment) = describe(&preset.cluster);
                forward.spec = preset.spec.clone();
            }
        }
        for preset in presets {
            let is_shown = self.forwards.iter().any(|forward| {
                forward.cluster == preset.cluster && forward.spec.is_same_target(&preset.spec)
            });
            if is_shown {
                continue;
            }
            let (cluster_label, environment) = describe(&preset.cluster);
            let id = ForwardId(self.next_id);
            self.next_id += 1;
            self.forwards.push(Forward {
                id,
                cluster: preset.cluster.clone(),
                cluster_label,
                environment,
                spec: preset.spec.clone(),
                state: ForwardState::Stopped,
                local: None,
                pod: None,
                traffic: ForwardTraffic::default(),
                events: VecDeque::new(),
                started_at: None,
                is_preset: true,
                is_refusing: false,
                run: None,
            });
        }
        if self.selected.is_some_and(|id| self.get(id).is_none()) {
            self.selected = None;
        }
    }

    /// A row built from fixed data, for the fixture screen and the tests.
    #[cfg(any(test, feature = "screenshot"))]
    pub(crate) fn insert_fixture(&mut self, fixture: ForwardFixture) -> ForwardId {
        let id = ForwardId(self.next_id);
        self.next_id += 1;
        self.forwards.push(Forward {
            id,
            cluster: fixture.cluster,
            cluster_label: fixture.cluster_label,
            environment: fixture.environment,
            spec: fixture.spec,
            state: fixture.state,
            local: fixture.local,
            pod: fixture.pod,
            traffic: fixture.traffic,
            events: fixture.events.into(),
            started_at: fixture.started_at,
            is_preset: fixture.is_preset,
            is_refusing: false,
            run: None,
        });
        id
    }
}

/// The parts of a fixture row; `insert_fixture` fills the rest.
#[cfg(any(test, feature = "screenshot"))]
pub(crate) struct ForwardFixture {
    pub(crate) cluster: ClusterRef,
    pub(crate) cluster_label: SharedString,
    pub(crate) environment: Environment,
    pub(crate) spec: ForwardSpec,
    pub(crate) state: ForwardState,
    pub(crate) local: Option<SocketAddrV4>,
    pub(crate) pod: Option<String>,
    pub(crate) traffic: ForwardTraffic,
    pub(crate) events: Vec<ForwardLogLine>,
    pub(crate) started_at: Option<jiff::Timestamp>,
    pub(crate) is_preset: bool,
}

impl Forward {
    /// The state of a forward that is up: `Paused` while the cluster is locked.
    fn steady_state(&self) -> ForwardState {
        if self.is_refusing {
            ForwardState::Paused
        } else {
            ForwardState::Active
        }
    }

    fn push_event(&mut self, at: jiff::Timestamp, text: String, tone: Option<StatusTone>) {
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(ForwardLogLine {
            at,
            text: text.into(),
            tone,
        });
    }

    fn apply_event(&mut self, event: ForwardEvent, now: jiff::Timestamp) {
        let warn = Some(StatusTone::Warn);
        match event {
            ForwardEvent::ConnectionLost { reason } => {
                self.push_event(now, format!("connection lost: {reason}"), warn);
            }
            ForwardEvent::Reconnecting { attempt } => {
                self.state = ForwardState::Reconnecting { attempt };
                self.push_event(
                    now,
                    format!("reconnecting {attempt}/{RECONNECT_TRIES}"),
                    warn,
                );
            }
            ForwardEvent::Reconnected { pod } => {
                self.state = self.steady_state();
                self.push_event(now, format!("reconnected to {pod}"), Some(StatusTone::Ok));
                self.pod = Some(pod);
            }
            ForwardEvent::ConnectionRefused { reason } => {
                self.push_event(now, format!("connection refused: {reason}"), warn);
            }
            ForwardEvent::ConnectionLimit => {
                self.push_event(now, "connection limit reached".to_owned(), warn);
            }
            ForwardEvent::Paused => {
                self.is_refusing = true;
                if self.state == ForwardState::Active {
                    self.state = ForwardState::Paused;
                }
                self.push_event(now, "paused: the cluster is read-only".to_owned(), warn);
            }
            ForwardEvent::Resumed => {
                self.is_refusing = false;
                if self.state == ForwardState::Paused {
                    self.state = ForwardState::Active;
                }
                self.push_event(now, "resumed".to_owned(), Some(StatusTone::Ok));
            }
        }
    }
}

/// Why `parse_target` refused a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetParseError {
    /// No `kind/name` shape, or a kind that is not forwardable.
    Shape,
    /// A name that cannot be a Kubernetes object name.
    Name,
}

impl TargetParseError {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Shape => "Use pod/NAME, svc/NAME, deploy/NAME, or sts/NAME",
            Self::Name => "Not a valid object name",
        }
    }
}

/// `pod/api-0`, `svc/payments-api`, `deploy/web`, `sts/db` (and the long kind names).
pub(crate) fn parse_target(text: &str) -> Result<TargetSpec, TargetParseError> {
    let (kind, name) = text.trim().split_once('/').ok_or(TargetParseError::Shape)?;
    let kind = match kind.to_ascii_lowercase().as_str() {
        "pod" | "po" | "pods" => TargetKind::Pod,
        "svc" | "service" | "services" => TargetKind::Service,
        "deploy" | "deployment" | "deployments" => TargetKind::Deployment,
        "sts" | "statefulset" | "statefulsets" => TargetKind::StatefulSet,
        _ => return Err(TargetParseError::Shape),
    };
    if !is_dns_subdomain(name) {
        return Err(TargetParseError::Name);
    }
    Ok(TargetSpec {
        kind,
        name: name.to_owned(),
    })
}

/// DNS-1123 subdomain, the shape of every name this page can forward to. The transport puts the
/// namespace and the name in a request path without escaping them, so nothing else gets through.
pub(crate) fn is_dns_subdomain(name: &str) -> bool {
    (1..=253).contains(&name.len())
        && name.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// A port number the user typed: 1 to 65535.
pub(crate) fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|port| *port != 0)
}

/// The error under a local port field that may be left empty.
pub(crate) const LOCAL_PORT_FIELD_ERROR: &str = "Leave empty for automatic, or enter 1 to 65535";

/// The local port a field holds: empty is automatic, a typed one is exact. `None` is not a port.
pub(crate) fn parse_local_port_field(text: &str) -> Option<LocalPortSpec> {
    if text.trim().is_empty() {
        return Some(LocalPortSpec::Auto);
    }
    parse_port(text).map(LocalPortSpec::Exact)
}

#[cfg(test)]
#[path = "port_forwards_tests.rs"]
mod port_forwards_tests;
