//! Port-forward over kube's `ws` transport (spec 0035). `port_forward` is the only `portforward`
//! call site: the `port_forward.rs` row of the 0030 allow-list. It needs a `PortForwardPermit`,
//! refuses to start while the connection's `WritePolicy` is `Blocked`, and calls no `spawn`: the
//! forward is one future inside the returned stream, so dropping the stream closes the listeners
//! and every socket. Kube's `Portforwarder` spawns its own loop, which `AbortOnDrop` stops.
//!
//! Listeners bind loopback only. Nothing here logs or keeps payload bytes; traces carry names,
//! ports, counts, and durations.

use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::channel::mpsc;
use futures::future::{BoxFuture, FusedFuture, FutureExt};
use futures::stream::{FusedStream, FuturesUnordered, Stream, StreamExt};
use futures::{future, stream};
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::Portforwarder;
use kube::client::UpgradeConnectionError;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, MissedTickBehavior};

use crate::connection::{ClusterConnection, ClusterError, classify_error, run_raw};
use crate::object_write::WritePolicy;
use crate::port_forward_target::{RESOLVE_ACTION, ResolvedPod, resolve_target};
use crate::reason_text::reason_text;

const FORWARD_ACTION: &str = "forwarding a port";
/// Each local connection is a WebSocket to the API server.
const MAX_CONNECTIONS: usize = 64;
/// How many ports after the preferred one an `Auto` port tries before asking the OS.
const CANDIDATE_SPAN: u16 = 20;
const TRAFFIC_INTERVAL: Duration = Duration::from_secs(1);
/// A failed `accept` (for example out of file handles) waits this long, so it cannot spin.
const ACCEPT_RETRY: Duration = Duration::from_millis(50);
/// The waits before each of the five reconnect attempts.
const RECONNECT_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

/// Proof that the session SSAR allowed both port-forward verbs. Not `Clone`: one permit starts
/// one forward.
#[derive(Debug)]
pub struct PortForwardPermit(());

impl PortForwardPermit {
    /// Only `AccessReport::port_forward_permit` calls this, after checking both verbs.
    pub(crate) fn granted() -> Self {
        Self(())
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self(())
    }
}

/// What to forward to. A Service, Deployment, or StatefulSet resolves to a ready pod.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ForwardTarget {
    Pod { name: String },
    Service { name: String },
    Deployment { name: String },
    StatefulSet { name: String },
}

/// The local port to listen on. A port the user typed is `Exact` and never moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalPort {
    /// Preferred: the next free port is taken when it is busy or reserved.
    Auto(u16),
    Exact(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardRequest {
    pub namespace: String,
    pub target: ForwardTarget,
    /// The pod port, or for a Service the Service port.
    pub remote_port: u16,
    pub local_port: LocalPort,
}

#[derive(Debug)]
pub enum ForwardUpdate {
    /// The local listener is up. Sent once, after the bind. `has_ipv6`: the `[::1]` listener is
    /// up too.
    Listening {
        local: SocketAddrV4,
        has_ipv6: bool,
    },
    /// The pod behind the target; sent at the start and after each reconnect.
    Resolved {
        pod: String,
        pod_port: u16,
    },
    /// At most once a second, and only when a number changed.
    Traffic(ForwardTraffic),
    Event(ForwardEvent),
    /// The forward ended. Nothing follows.
    Ended(ForwardError),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForwardTraffic {
    pub open_connections: u32,
    /// Bytes read from the pod, over every connection.
    pub received: u64,
    /// Bytes read from local clients, over every connection.
    pub sent: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwardEvent {
    ConnectionLost {
        reason: String,
    },
    Reconnecting {
        attempt: u8,
    },
    Reconnected {
        pod: String,
    },
    ConnectionRefused {
        reason: String,
    },
    ConnectionLimit,
    /// New local connections are refused (the cluster is locked).
    Paused,
    Resumed,
}

/// Whether new local connections are served. Open ones are never touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardControl {
    Accept,
    Refuse,
}

/// Why a forward ended. The texts are fixed: they name objects, never bytes.
#[derive(Debug, thiserror::Error)]
pub enum ForwardError {
    #[error("writes are blocked in this debug build (set K8SBOARD_ALLOW_WRITES=1)")]
    WritesBlocked,
    #[error("Port {0} in use")]
    PortInUse(u16),
    #[error("Port {0} is reserved or needs more rights")]
    PortReserved(u16),
    #[error("cannot listen on the local port: {0}")]
    Bind(String),
    #[error(
        "the server refused the port-forward connection (HTTP 403); a forward needs get and \
         create on pods/portforward"
    )]
    NotPermitted,
    #[error("the server refused the port-forward connection (HTTP 401)")]
    Unauthorized,
    #[error("the forward target was not found")]
    TargetNotFound,
    #[error("no ready pod backs the forward target")]
    NoReadyPod,
    #[error("the target port '{0}' is not declared by the pod")]
    PortNotDeclared(String),
    #[error("a Service without a selector, or an ExternalName Service, cannot be forwarded to")]
    UnsupportedService,
    #[error("the forward target was lost and did not come back")]
    TargetLost,
    #[error("{}", reason_text(&.0.to_string()))]
    Cluster(#[from] ClusterError),
}

impl ClusterConnection {
    /// action: "forwarding a port". Nothing happens until polled on the app's tokio runtime.
    ///
    /// Items arrive as `Listening` and `Resolved`, then `Traffic` and `Event` while it runs, and
    /// `Ended` last. A stream that ends without `Ended` was dropped by its owner: dropping the
    /// stream closes the listeners and aborts every socket.
    pub fn port_forward(
        &self,
        _permit: PortForwardPermit,
        request: ForwardRequest,
        control: impl Stream<Item = ForwardControl> + Send + Unpin + 'static,
    ) -> impl Stream<Item = ForwardUpdate> + Send + 'static {
        let connection = self.clone();
        let open = {
            let connection = self.clone();
            let namespace = request.namespace.clone();
            move |pod: String, port: u16| {
                open_remote(connection.clone(), namespace.clone(), pod, port)
            }
        };
        let seams = Seams {
            bind: |address: SocketAddr| TcpListener::bind(address),
            open,
        };
        forward_updates(connection, request, control, seams)
    }
}

/// The local port a forward starts from: 5432 becomes 15432, which is clear of privileged ports.
pub fn default_local_port(remote: u16) -> u16 {
    remote.checked_add(10_000).unwrap_or(remote)
}

/// The two edges of a forward that tests replace: binding a listener and opening a socket to
/// the pod. Production passes `TcpListener::bind` and the kube `portforward` call.
struct Seams<B, O> {
    bind: B,
    open: O,
}

/// One WebSocket to the pod, one port, and what keeps it alive.
struct RemoteSocket<S> {
    stream: S,
    /// Resolves with the error-channel message, or `None` when the sender is dropped.
    error: BoxFuture<'static, Option<String>>,
    /// Dropped with the socket: for the real one it aborts kube's loop task.
    _guard: Box<dyn Send>,
}

/// Why a socket could not open.
#[derive(Debug, PartialEq, Eq)]
enum SocketError {
    Unauthorized,
    Forbidden,
    NotFound,
    Failed(String),
}

/// How one local connection ended.
#[derive(Debug, PartialEq, Eq)]
enum SocketEnd {
    /// Both sides finished.
    Closed,
    /// The pod's error channel reported a failure (for example nothing listens on the port).
    Refused(String),
    Open(SocketError),
    /// The WebSocket or the local socket broke during the copy.
    Broken(String),
}

/// What a socket end means for the forward.
#[derive(Debug)]
enum SocketAction {
    Nothing,
    Refused {
        reason: String,
    },
    End(ForwardError),
    /// The pod may be gone: look before reconnecting.
    CheckPod {
        reason: String,
    },
}

/// The stream the app polls: the driver future runs inside it, so there is no task to leak.
fn forward_updates<B, BF, O, OF, S, C>(
    connection: ClusterConnection,
    request: ForwardRequest,
    control: C,
    seams: Seams<B, O>,
) -> impl Stream<Item = ForwardUpdate> + Send + 'static
where
    B: Fn(SocketAddr) -> BF + Send + Sync + 'static,
    BF: Future<Output = io::Result<TcpListener>> + Send,
    O: Fn(String, u16) -> OF + Send + Sync + 'static,
    OF: Future<Output = Result<RemoteSocket<S>, SocketError>> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C: Stream<Item = ForwardControl> + Send + Unpin + 'static,
{
    let (updates, receiver) = mpsc::unbounded();
    let driver = async move {
        let driver = Driver {
            connection: &connection,
            request: &request,
            seams: &seams,
            updates: &updates,
            counters: Counters::default(),
        };
        let error = driver.run(control).await;
        driver.emit(ForwardUpdate::Ended(error));
    };
    // The driver sends everything through `updates`; it yields no item of its own.
    stream::select(
        receiver,
        driver
            .into_stream()
            .filter_map(|()| future::ready(None::<ForwardUpdate>)),
    )
}

/// Shared with the `Counting` wrappers of every socket.
#[derive(Default)]
struct Counters {
    sent: Arc<AtomicU64>,
    received: Arc<AtomicU64>,
}

struct Driver<'a, B, O> {
    connection: &'a ClusterConnection,
    request: &'a ForwardRequest,
    seams: &'a Seams<B, O>,
    updates: &'a mpsc::UnboundedSender<ForwardUpdate>,
    counters: Counters,
}

type Reconnect<'a> = Pin<Box<dyn Future<Output = Result<ResolvedPod, ForwardError>> + Send + 'a>>;

/// What the forward is doing about a lost socket. Both steps wait on the network, so they are
/// futures polled by the main loop, never awaited inside it: copies, ticks, and the lock keep
/// running meanwhile.
#[derive(Default)]
struct Recovery<'a> {
    /// The lookup that tells whether the pod is gone.
    checking: Option<BoxFuture<'a, bool>>,
    /// The attempts to find the target again.
    reconnect: Option<Reconnect<'a>>,
}

impl Recovery<'_> {
    /// While either step runs, new connections are closed and further failures are ignored: every
    /// socket of a lost pod fails, and the first one is enough.
    fn is_active(&self) -> bool {
        self.checking.is_some() || self.reconnect.is_some()
    }
}

impl<'a, B, BF, O, OF, S> Driver<'a, B, O>
where
    B: Fn(SocketAddr) -> BF,
    BF: Future<Output = io::Result<TcpListener>> + Send,
    O: Fn(String, u16) -> OF,
    OF: Future<Output = Result<RemoteSocket<S>, SocketError>> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    fn emit(&self, update: ForwardUpdate) {
        // A closed receiver means the stream was dropped; there is nobody to tell.
        let _ = self.updates.unbounded_send(update);
    }

    fn emit_event(&self, event: ForwardEvent) {
        self.emit(ForwardUpdate::Event(event));
    }

    /// Runs until the forward ends, and returns why. The kill switch comes first: a forward
    /// opens connections into the cluster, so it is gated like a write (spec 0030).
    async fn run<C>(&'a self, control: C) -> ForwardError
    where
        C: Stream<Item = ForwardControl> + Unpin,
    {
        if self.connection.write_policy() == WritePolicy::Blocked {
            return ForwardError::WritesBlocked;
        }
        let mut target = match resolve_target(self.connection, self.request).await {
            Ok(target) => target,
            Err(error) => return error,
        };
        let bound = match bind_local(&self.seams.bind, self.request.local_port).await {
            Ok(bound) => bound,
            Err(error) => return error,
        };
        self.emit(ForwardUpdate::Listening {
            local: bound.local,
            has_ipv6: bound.v6.is_some(),
        });
        self.emit_resolved(&target);
        tracing::debug!(
            context = %self.connection.context(),
            namespace = %self.request.namespace,
            pod = %target.pod,
            local_port = bound.local.port(),
            "port-forward listening"
        );

        let mut control = control.fuse();
        let mut sockets = FuturesUnordered::new();
        let mut tick =
            tokio::time::interval_at(Instant::now() + TRAFFIC_INTERVAL, TRAFFIC_INTERVAL);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut recovery = Recovery::default();
        let mut gate = Gate::default();
        let mut last_traffic = ForwardTraffic::default();
        loop {
            tokio::select! {
                accepted = accept_either(&bound.v4, bound.v6.as_ref()) => match accepted {
                    Ok(local) => match gate.admit(sockets.len(), recovery.is_active()) {
                        Admission::Open => {
                            sockets.push(self.forward_socket(local, target.pod.clone(), target.pod_port));
                        }
                        Admission::Close => {}
                        Admission::CloseAndReportLimit => self.emit_event(ForwardEvent::ConnectionLimit),
                    },
                    Err(_) => tokio::time::sleep(ACCEPT_RETRY).await,
                },
                Some(end) = sockets.next() => {
                    if let Some(error) = self.on_socket_end(end, &target.pod, &mut recovery) {
                        return error;
                    }
                }
                _ = tick.tick() => {
                    let traffic = ForwardTraffic {
                        open_connections: u32::try_from(sockets.len()).unwrap_or(u32::MAX),
                        received: self.counters.received.load(Ordering::Relaxed),
                        sent: self.counters.sent.load(Ordering::Relaxed),
                    };
                    if traffic != last_traffic {
                        last_traffic = traffic;
                        self.emit(ForwardUpdate::Traffic(traffic));
                    }
                }
                item = control.next(), if !control.is_terminated() => {
                    if let Some(control) = item {
                        self.on_control(control, &mut gate);
                    }
                }
                is_gone = async {
                    match recovery.checking.as_mut() {
                        Some(pending) => pending.await,
                        None => future::pending().await,
                    }
                } => {
                    recovery.checking = None;
                    if is_gone {
                        recovery.reconnect = Some(Box::pin(reconnect_target(
                            self.connection,
                            self.request,
                            self.updates,
                        )));
                    }
                }
                resolved = async {
                    match recovery.reconnect.as_mut() {
                        Some(pending) => pending.await,
                        None => future::pending().await,
                    }
                } => {
                    recovery.reconnect = None;
                    match resolved {
                        Ok(resolved) => {
                            self.emit_resolved(&resolved);
                            target = resolved;
                        }
                        Err(error) => return error,
                    }
                }
            }
        }
    }

    fn emit_resolved(&self, target: &ResolvedPod) {
        self.emit(ForwardUpdate::Resolved {
            pod: target.pod.clone(),
            pod_port: target.pod_port,
        });
    }

    fn on_control(&self, control: ForwardControl, gate: &mut Gate) {
        let is_refusing = control == ForwardControl::Refuse;
        if gate.is_refusing == is_refusing {
            return;
        }
        gate.is_refusing = is_refusing;
        self.emit_event(if is_refusing {
            ForwardEvent::Paused
        } else {
            ForwardEvent::Resumed
        });
    }

    /// Returns the error that ends the forward, if the socket's end is one.
    fn on_socket_end(
        &'a self,
        end: SocketEnd,
        pod: &str,
        recovery: &mut Recovery<'a>,
    ) -> Option<ForwardError> {
        match socket_outcome(end) {
            SocketAction::Nothing => None,
            SocketAction::Refused { reason } => {
                self.emit_event(ForwardEvent::ConnectionRefused { reason });
                None
            }
            SocketAction::End(error) => Some(error),
            SocketAction::CheckPod { reason } => {
                if recovery.is_active() {
                    return None;
                }
                self.emit_event(ForwardEvent::ConnectionLost { reason });
                recovery.checking = Some(Box::pin(pod_is_gone(
                    self.connection,
                    &self.request.namespace,
                    pod.to_owned(),
                )));
                None
            }
        }
    }

    /// One local connection: open a WebSocket to the pod, then copy both ways. An error-channel
    /// message ends it at once, not after the copy (decision 30).
    async fn forward_socket(&self, local: TcpStream, pod: String, port: u16) -> SocketEnd {
        let remote = match (self.seams.open)(pod, port).await {
            Ok(remote) => remote,
            Err(error) => return SocketEnd::Open(error),
        };
        // The guard lives until this function returns, whatever the way out.
        let RemoteSocket {
            stream,
            error,
            _guard,
        } = remote;
        let mut local = Counting::new(local, Arc::clone(&self.counters.sent));
        let mut stream = Counting::new(stream, Arc::clone(&self.counters.received));
        let mut error = error.fuse();
        let copy = copy_bidirectional(&mut local, &mut stream);
        tokio::pin!(copy);
        loop {
            tokio::select! {
                copied = &mut copy => {
                    return match copied {
                        Ok(_) => SocketEnd::Closed,
                        Err(error) => SocketEnd::Broken(reason_text(&error.to_string())),
                    };
                }
                message = &mut error, if !error.is_terminated() => {
                    // A dropped sender (`None`) only means the loop ended; the copy decides.
                    if let Some(message) = message {
                        return SocketEnd::Refused(reason_text(&message));
                    }
                }
            }
        }
    }
}

/// Whether new local connections are served right now.
#[derive(Default)]
struct Gate {
    is_refusing: bool,
    is_limit_reported: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Admission {
    Open,
    /// Closed at once: reconnecting, or refused by the lock.
    Close,
    /// Over the limit: closed, and the caller reports it (once per episode).
    CloseAndReportLimit,
}

impl Gate {
    fn admit(&mut self, open: usize, is_reconnecting: bool) -> Admission {
        if is_reconnecting || self.is_refusing {
            return Admission::Close;
        }
        if open < MAX_CONNECTIONS {
            self.is_limit_reported = false;
            return Admission::Open;
        }
        if self.is_limit_reported {
            return Admission::Close;
        }
        self.is_limit_reported = true;
        Admission::CloseAndReportLimit
    }
}

/// Accepts on the IPv4 listener or, when there is one, the IPv6 one.
async fn accept_either(v4: &TcpListener, v6: Option<&TcpListener>) -> io::Result<TcpStream> {
    let accepted = match v6 {
        Some(v6) => tokio::select! {
            accepted = v4.accept() => accepted,
            accepted = v6.accept() => accepted,
        },
        None => v4.accept().await,
    };
    accepted.map(|(stream, _address)| stream)
}

/// The loopback listeners of one forward.
struct Bound {
    v4: TcpListener,
    /// Best effort: absent when `[::1]` is unavailable or the port is taken there.
    v6: Option<TcpListener>,
    local: SocketAddrV4,
}

#[derive(Debug)]
enum BindStep {
    NextPort,
    Fail(ForwardError),
}

/// What a failed bind means. A typed port never moves; an `Auto` port skips a busy or reserved
/// one (Windows answers `PermissionDenied` for an excluded range).
fn bind_outcome(kind: io::ErrorKind, local_port: LocalPort) -> BindStep {
    let is_unavailable = matches!(
        kind,
        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
    );
    match local_port {
        LocalPort::Auto(_) if is_unavailable => BindStep::NextPort,
        LocalPort::Exact(port) if kind == io::ErrorKind::AddrInUse => {
            BindStep::Fail(ForwardError::PortInUse(port))
        }
        LocalPort::Exact(port) if kind == io::ErrorKind::PermissionDenied => {
            BindStep::Fail(ForwardError::PortReserved(port))
        }
        _ => BindStep::Fail(ForwardError::Bind(kind.to_string())),
    }
}

/// The preferred port, the next 20, then `0`: any port the OS picks.
fn candidate_ports(preferred: u16) -> impl Iterator<Item = u16> {
    (preferred..=preferred.saturating_add(CANDIDATE_SPAN)).chain(std::iter::once(0))
}

/// Binds `127.0.0.1` first, then `[::1]` on the same port. These are the only two addresses a
/// forward ever binds.
///
/// `[::1]` is best effort: when it is unavailable (no IPv6) the forward runs without it. But a
/// port another process holds on `[::1]` would shadow the forward for `localhost` clients that
/// resolve to `::1` first, so an `Auto` port treats that as a busy port and moves on. A typed
/// (`Exact`) port keeps the IPv4 listener whatever happens on `[::1]`.
async fn bind_local<B, BF>(bind: &B, local_port: LocalPort) -> Result<Bound, ForwardError>
where
    B: Fn(SocketAddr) -> BF,
    BF: Future<Output = io::Result<TcpListener>>,
{
    let ports: Vec<u16> = match local_port {
        LocalPort::Exact(port) => vec![port],
        LocalPort::Auto(port) => candidate_ports(port).collect(),
    };
    for port in ports {
        let v4 = match bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await {
            Ok(listener) => listener,
            Err(error) => match bind_outcome(error.kind(), local_port) {
                BindStep::NextPort => continue,
                BindStep::Fail(error) => return Err(error),
            },
        };
        let local = match v4.local_addr() {
            Ok(SocketAddr::V4(local)) => local,
            Ok(SocketAddr::V6(_)) => {
                return Err(ForwardError::Bind("not an IPv4 address".to_owned()));
            }
            Err(error) => return Err(ForwardError::Bind(error.kind().to_string())),
        };
        let v6 = match bind(SocketAddr::from((Ipv6Addr::LOCALHOST, local.port()))).await {
            Ok(listener) => Some(listener),
            Err(error) if is_taken_on_ipv6(&error, local_port) => continue,
            Err(_) => None,
        };
        return Ok(Bound { v4, v6, local });
    }
    Err(ForwardError::Bind("no free local port".to_owned()))
}

/// Whether a failed `[::1]` bind means the port is held there, for a port that may move.
fn is_taken_on_ipv6(error: &io::Error, local_port: LocalPort) -> bool {
    matches!(local_port, LocalPort::Auto(_)) && error.kind() == io::ErrorKind::AddrInUse
}

/// Whether the pod is gone, not Running, or being deleted. A failed lookup reads as "still
/// there": a reconnect would fail the same way.
async fn pod_is_gone(connection: &ClusterConnection, namespace: &str, pod: String) -> bool {
    let api: Api<Pod> = Api::namespaced(connection.client().clone(), namespace);
    match connection.run(RESOLVE_ACTION, api.get(&pod)).await {
        Ok(pod) => {
            pod.metadata.deletion_timestamp.is_some()
                || pod.status.and_then(|status| status.phase).as_deref() != Some("Running")
        }
        Err(ClusterError::Api { code: 404, .. }) => true,
        Err(_) => false,
    }
}

/// Up to five attempts to find the target again, after the waits of `RECONNECT_DELAYS`. A
/// refusal of credentials ends it at once: retrying cannot fix RBAC.
async fn reconnect_target(
    connection: &ClusterConnection,
    request: &ForwardRequest,
    updates: &mpsc::UnboundedSender<ForwardUpdate>,
) -> Result<ResolvedPod, ForwardError> {
    let emit = |event| {
        let _ = updates.unbounded_send(ForwardUpdate::Event(event));
    };
    for (attempt, delay) in (1u8..).zip(RECONNECT_DELAYS) {
        emit(ForwardEvent::Reconnecting { attempt });
        tokio::time::sleep(delay).await;
        match resolve_target(connection, request).await {
            Ok(resolved) => {
                emit(ForwardEvent::Reconnected {
                    pod: resolved.pod.clone(),
                });
                return Ok(resolved);
            }
            Err(
                error @ ForwardError::Cluster(
                    ClusterError::Unauthorized { .. } | ClusterError::Forbidden { .. },
                ),
            ) => return Err(error),
            Err(_) => {}
        }
    }
    Err(ForwardError::TargetLost)
}

/// A refused upgrade carries no `Status` body, so the meaning comes from the code alone.
fn upgrade_error(code: u16) -> SocketError {
    match code {
        401 => SocketError::Unauthorized,
        403 => SocketError::Forbidden,
        404 => SocketError::NotFound,
        code => SocketError::Failed(format!(
            "the port-forward connection was refused (HTTP {code})"
        )),
    }
}

fn socket_error(context: &str, error: kube::Error) -> SocketError {
    match error {
        kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(code)) => {
            upgrade_error(code.as_u16())
        }
        // `classify_error` words credential failures without their detail.
        other => SocketError::Failed(reason_text(
            &classify_error(context, FORWARD_ACTION, other).to_string(),
        )),
    }
}

/// Sorts how a socket ended into what the forward does next.
fn socket_outcome(end: SocketEnd) -> SocketAction {
    match end {
        SocketEnd::Closed => SocketAction::Nothing,
        SocketEnd::Refused(reason) => SocketAction::Refused { reason },
        SocketEnd::Open(SocketError::Unauthorized) => SocketAction::End(ForwardError::Unauthorized),
        SocketEnd::Open(SocketError::Forbidden) => SocketAction::End(ForwardError::NotPermitted),
        SocketEnd::Open(SocketError::NotFound) => SocketAction::CheckPod {
            reason: "pod not found".to_owned(),
        },
        SocketEnd::Open(SocketError::Failed(reason)) | SocketEnd::Broken(reason) => {
            SocketAction::CheckPod { reason }
        }
    }
}

/// Opens one WebSocket to the pod for one local connection (30 s open timeout).
async fn open_remote(
    connection: ClusterConnection,
    namespace: String,
    pod: String,
    port: u16,
) -> Result<RemoteSocket<impl AsyncRead + AsyncWrite + Unpin + Send + 'static>, SocketError> {
    let api: Api<Pod> = Api::namespaced(connection.client().clone(), &namespace);
    let forwarder = match run_raw(connect_portforward(&api, &pod, port)).await {
        Ok(Ok(forwarder)) => forwarder,
        Ok(Err(error)) => return Err(socket_error(connection.context(), error)),
        Err(_elapsed) => {
            return Err(SocketError::Failed(
                "the port-forward connection timed out".to_owned(),
            ));
        }
    };
    // Kube's loop outlives a dropped `Portforwarder`, so the guard owns it from here on.
    let mut guard = AbortOnDrop(forwarder);
    let (Some(stream), Some(error)) = (guard.0.take_stream(port), guard.0.take_error(port)) else {
        return Err(SocketError::Failed(
            "the port-forward connection has no stream".to_owned(),
        ));
    };
    Ok(RemoteSocket {
        stream,
        error: Box::pin(error),
        _guard: Box::new(guard),
    })
}

/// The one portforward call site, one row of the 0030 exception table (`port_forward.rs`,
/// `portforward`).
#[allow(clippy::disallowed_methods)]
async fn connect_portforward(
    api: &Api<Pod>,
    pod: &str,
    port: u16,
) -> Result<Portforwarder, kube::Error> {
    api.portforward(pod, &[port]).await
}

struct AbortOnDrop(Portforwarder);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Adds the bytes read through it to a shared counter.
struct Counting<T> {
    inner: T,
    counter: Arc<AtomicU64>,
}

impl<T> Counting<T> {
    fn new(inner: T, counter: Arc<AtomicU64>) -> Self {
        Self { inner, counter }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Counting<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            let read = buf.filled().len() - before;
            self.counter.fetch_add(read as u64, Ordering::Relaxed);
        }
        poll
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Counting<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
#[path = "port_forward_tests.rs"]
mod port_forward_tests;
