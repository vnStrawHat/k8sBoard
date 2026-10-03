use std::collections::VecDeque;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio::task::JoinHandle;

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest, reply};
use crate::object_write::WriteError;
use crate::reason_text::MAX_REASON_CHARS;

const NS: &str = "team-a";
const WAIT: Duration = Duration::from_secs(30);
const QUIET: Duration = Duration::from_millis(300);
const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"not found","reason":"NotFound","code":404}"#;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pod_target() -> ForwardTarget {
    ForwardTarget::Pod {
        name: "api-0".to_owned(),
    }
}

fn request(target: ForwardTarget, local_port: LocalPort) -> ForwardRequest {
    ForwardRequest {
        namespace: NS.to_owned(),
        target,
        remote_port: 8080,
        local_port,
    }
}

// --- fake API objects -------------------------------------------------------------------------

fn running_pod() -> String {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "api-0", "namespace": NS },
        "status": { "phase": "Running", "conditions": [{ "type": "Ready", "status": "True" }] },
    })
    .to_string()
}

/// Answers `GET .../pods/api-0` with a running pod.
fn pod_running(request: &RecordedRequest) -> (u16, String) {
    if request.path.ends_with("/pods/api-0") {
        (200, running_pod())
    } else {
        (404, NOT_FOUND.to_owned())
    }
}

fn pod_gone(_request: &RecordedRequest) -> (u16, String) {
    (404, NOT_FOUND.to_owned())
}

/// The pod exists for the first lookup (the start of the forward) and is gone for every later one.
fn running_then_gone() -> impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static {
    let gets = AtomicUsize::new(0);
    move |request| {
        if gets.fetch_add(1, Ordering::SeqCst) == 0 {
            pod_running(request)
        } else {
            pod_gone(request)
        }
    }
}

// --- the forward under test -------------------------------------------------------------------

/// What the fake `bind` seam does. Tests never listen on a fixed port.
#[derive(Clone, Copy)]
enum BindMode {
    /// Every request binds `127.0.0.1:0`; an IPv6 request is recorded and either joins the set
    /// as a second loopback listener or fails.
    Loopback { is_v6_up: bool },
    /// The real `TcpListener::bind`.
    Real,
}

/// The far end of one fake WebSocket: the pod side of the copy and its error channel.
struct FarEnd {
    stream: DuplexStream,
    error: oneshot::Sender<String>,
}

struct CountGuard(Arc<AtomicUsize>);

impl Drop for CountGuard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Fixture {
    updates: mpsc::UnboundedReceiver<ForwardUpdate>,
    control: mpsc::UnboundedSender<ForwardControl>,
    far_ends: mpsc::UnboundedReceiver<FarEnd>,
    /// Scripted open failures, taken one per socket before any socket opens.
    script: Arc<Mutex<VecDeque<SocketError>>>,
    binds: Arc<Mutex<Vec<SocketAddr>>>,
    second_listener: Arc<Mutex<Option<SocketAddr>>>,
    guards_dropped: Arc<AtomicUsize>,
    api: FakeApi,
    pump: JoinHandle<()>,
    /// How long one wait may take; a paused-time test that sits through a backoff raises it.
    wait: Duration,
}

impl Fixture {
    fn start(
        local_port: LocalPort,
        bind_mode: BindMode,
        respond: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static,
    ) -> Self {
        let (connection, api) = FakeApi::connection(WritePolicy::Allowed, respond);
        Self::start_on(connection, api, local_port, bind_mode)
    }

    /// A forward to the pod `api-0` over `connection`, which `api` records.
    fn start_on(
        connection: ClusterConnection,
        api: FakeApi,
        local_port: LocalPort,
        bind_mode: BindMode,
    ) -> Self {
        let binds = Arc::new(Mutex::new(Vec::new()));
        let second_listener = Arc::new(Mutex::new(None));
        let script = Arc::new(Mutex::new(VecDeque::new()));
        let guards_dropped = Arc::new(AtomicUsize::new(0));
        let (far_sender, far_ends) = mpsc::unbounded();
        let (control, control_receiver) = mpsc::unbounded();

        let bind = {
            let binds = Arc::clone(&binds);
            let second_listener = Arc::clone(&second_listener);
            move |address: SocketAddr| {
                locked(&binds).push(address);
                let second_listener = Arc::clone(&second_listener);
                async move {
                    match bind_mode {
                        BindMode::Real => TcpListener::bind(address).await,
                        BindMode::Loopback { is_v6_up } => {
                            if address.is_ipv6() && !is_v6_up {
                                return Err(io::ErrorKind::AddrNotAvailable.into());
                            }
                            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
                            if address.is_ipv6() {
                                *locked(&second_listener) = Some(listener.local_addr()?);
                            }
                            Ok(listener)
                        }
                    }
                }
            }
        };
        let open = {
            let script = Arc::clone(&script);
            let guards_dropped = Arc::clone(&guards_dropped);
            move |_pod: String, _port: u16| {
                let scripted = locked(&script).pop_front();
                let far_sender = far_sender.clone();
                let guards_dropped = Arc::clone(&guards_dropped);
                async move {
                    if let Some(error) = scripted {
                        return Err(error);
                    }
                    let (near, far) = duplex(1 << 20);
                    let (error_sender, error_receiver) = oneshot::channel();
                    let _ = far_sender.unbounded_send(FarEnd {
                        stream: far,
                        error: error_sender,
                    });
                    Ok(RemoteSocket {
                        stream: near,
                        error: Box::pin(error_receiver.map(|message| message.ok())),
                        _guard: Box::new(CountGuard(guards_dropped)),
                    })
                }
            }
        };
        let updates = forward_updates(
            connection,
            request(pod_target(), local_port),
            control_receiver,
            Seams { bind, open },
        );
        let (sender, receiver) = mpsc::unbounded();
        let pump = tokio::spawn(async move {
            let mut updates = Box::pin(updates);
            while let Some(update) = updates.next().await {
                if sender.unbounded_send(update).is_err() {
                    break;
                }
            }
        });
        Self {
            updates: receiver,
            control,
            far_ends,
            script,
            binds,
            second_listener,
            guards_dropped,
            api,
            pump,
            wait: WAIT,
        }
    }

    /// Allows waits as long as the whole reconnect backoff, in paused time.
    fn patient(mut self) -> Self {
        self.wait = Duration::from_secs(300);
        self
    }

    async fn next(&mut self) -> Option<ForwardUpdate> {
        tokio::time::timeout(self.wait, self.updates.next())
            .await
            .expect("an update arrives in time")
    }

    /// Skips updates until `pick` takes one.
    async fn until<T>(&mut self, mut pick: impl FnMut(ForwardUpdate) -> Option<T>) -> T {
        loop {
            let update = self.next().await.expect("the forward is still running");
            if let Some(found) = pick(update) {
                return found;
            }
        }
    }

    async fn next_event(&mut self) -> ForwardEvent {
        self.until(|update| match update {
            ForwardUpdate::Event(event) => Some(event),
            _ => None,
        })
        .await
    }

    async fn ended(&mut self) -> ForwardError {
        self.until(|update| match update {
            ForwardUpdate::Ended(error) => Some(error),
            _ => None,
        })
        .await
    }

    /// The `Listening` update, with its IPv6 flag.
    async fn listening(&mut self) -> (SocketAddrV4, bool) {
        self.until(|update| match update {
            ForwardUpdate::Listening { local, has_ipv6 } => Some((local, has_ipv6)),
            _ => None,
        })
        .await
    }

    async fn far_end(&mut self) -> FarEnd {
        tokio::time::timeout(WAIT, self.far_ends.next())
            .await
            .expect("a socket opens in time")
            .expect("the forward is still running")
    }

    /// Whether nothing arrives for a short while.
    async fn is_quiet(&mut self) -> bool {
        tokio::time::timeout(QUIET, self.updates.next())
            .await
            .is_err()
    }

    fn script(&self, error: SocketError) {
        locked(&self.script).push_back(error);
    }

    async fn stop(self) {
        self.pump.abort();
        let _ = self.pump.await;
    }
}

async fn connect(address: impl Into<SocketAddr>) -> TcpStream {
    TcpStream::connect(address.into())
        .await
        .expect("the listener accepts")
}

/// Whether the peer closes the connection without sending anything.
async fn sees_end_of_stream(client: &mut TcpStream) -> bool {
    let mut byte = [0; 1];
    match tokio::time::timeout(WAIT, client.read(&mut byte)).await {
        Ok(Ok(0) | Err(_)) => true,
        Ok(Ok(_)) | Err(_) => false,
    }
}

// --- start, binding, listeners ----------------------------------------------------------------

#[tokio::test]
async fn blocked_policy_sends_nothing() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, pod_running);
    let updates: Vec<_> = connection
        .port_forward(
            PortForwardPermit::for_tests(),
            request(pod_target(), LocalPort::Auto(18080)),
            stream::pending::<ForwardControl>(),
        )
        .collect()
        .await;
    assert!(
        matches!(
            updates.as_slice(),
            [ForwardUpdate::Ended(ForwardError::WritesBlocked)]
        ),
        "{updates:?}"
    );
    assert!(api.requests().is_empty(), "no request left the client");
}

#[test]
fn the_blocked_text_is_the_write_path_text() {
    assert_eq!(
        ForwardError::WritesBlocked.to_string(),
        WriteError::WritesBlocked.to_string()
    );
}

#[tokio::test]
async fn listener_binds_loopback_only() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: true },
        pod_running,
    );
    let (local, has_ipv6) = fixture.listening().await;
    assert_eq!(*local.ip(), Ipv4Addr::LOCALHOST);
    assert!(has_ipv6);
    let binds = locked(&fixture.binds).clone();
    assert_eq!(
        binds,
        [
            SocketAddr::from((Ipv4Addr::LOCALHOST, 18080)),
            SocketAddr::from((Ipv6Addr::LOCALHOST, local.port())),
        ],
        "only the two loopback addresses are ever requested, on the same port"
    );
    fixture.stop().await;
}

#[tokio::test]
async fn ipv6_listener_failure_is_ignored() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    let (local, has_ipv6) = fixture.listening().await;
    assert!(!has_ipv6);
    let _client = connect(local).await;
    let _far_end = fixture.far_end().await;
    fixture.stop().await;
}

#[tokio::test]
async fn connections_from_both_listeners_share_the_set() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: true },
        pod_running,
    );
    let (local, _has_ipv6) = fixture.listening().await;
    let second = locked(&fixture.second_listener).expect("the second listener is up");
    let _first_client = connect(local).await;
    let _second_client = connect(second).await;
    let _first_far_end = fixture.far_end().await;
    let _second_far_end = fixture.far_end().await;
    let traffic = fixture
        .until(|update| match update {
            ForwardUpdate::Traffic(traffic) if traffic.open_connections == 2 => Some(traffic),
            _ => None,
        })
        .await;
    assert_eq!(traffic.open_connections, 2);
    fixture.stop().await;
}

#[tokio::test]
async fn exact_port_in_use_ends_with_port_in_use() {
    let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a free port");
    let port = held.local_addr().expect("bound").port();
    let mut fixture = Fixture::start(LocalPort::Exact(port), BindMode::Real, pod_running);
    let error = fixture.ended().await;
    assert!(matches!(error, ForwardError::PortInUse(busy) if busy == port));
    assert_eq!(error.to_string(), format!("Port {port} in use"));
    fixture.stop().await;
}

/// A bind seam that fails on the ports `failing` names and otherwise binds any loopback port.
fn failing_bind(
    failing: &'static [u16],
    kind: io::ErrorKind,
    requested: Arc<Mutex<Vec<u16>>>,
) -> impl Fn(SocketAddr) -> BoxFuture<'static, io::Result<TcpListener>> {
    move |address| {
        locked(&requested).push(address.port());
        let is_failing = address.is_ipv4() && failing.contains(&address.port());
        async move {
            if is_failing {
                return Err(kind.into());
            }
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await
        }
        .boxed()
    }
}

#[tokio::test]
async fn bind_permission_denied_is_treated_as_in_use() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let bind = failing_bind(
        &[20_000],
        io::ErrorKind::PermissionDenied,
        Arc::clone(&requested),
    );
    let bound = bind_local(&bind, LocalPort::Auto(20_000))
        .await
        .expect("the next port is free");
    let ipv4_requests: Vec<u16> = locked(&requested).iter().copied().take(2).collect();
    assert_eq!(ipv4_requests, [20_000, 20_001]);
    assert!(bound.local.port() != 0);
}

#[tokio::test]
async fn exact_reserved_port_ends_port_reserved() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let bind = failing_bind(&[20_000], io::ErrorKind::PermissionDenied, requested);
    let Err(error) = bind_local(&bind, LocalPort::Exact(20_000)).await else {
        panic!("a reserved exact port cannot bind");
    };
    assert!(matches!(error, ForwardError::PortReserved(20_000)));
    assert_eq!(error.to_string(), "Port 20000 is reserved by the system");
}

#[tokio::test]
async fn auto_port_moves_to_the_next_free_port() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let bind = failing_bind(
        &[20_000, 20_001],
        io::ErrorKind::AddrInUse,
        Arc::clone(&requested),
    );
    bind_local(&bind, LocalPort::Auto(20_000))
        .await
        .expect("the third port is free");
    let ipv4_requests: Vec<u16> = locked(&requested).iter().copied().take(3).collect();
    assert_eq!(ipv4_requests, [20_000, 20_001, 20_002]);
}

#[tokio::test]
async fn auto_port_ends_on_other_bind_errors() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let bind = failing_bind(&[20_000], io::ErrorKind::AddrNotAvailable, requested);
    let Err(error) = bind_local(&bind, LocalPort::Auto(20_000)).await else {
        panic!("an unusable address is not a busy port");
    };
    assert!(matches!(error, ForwardError::Bind(_)), "{error}");
}

/// A bind seam whose first `busy` IPv6 requests fail with `kind`; every IPv4 request binds a
/// loopback port. Records each requested address.
fn ipv6_failing_bind(
    kind: io::ErrorKind,
    busy: usize,
    requested: Arc<Mutex<Vec<SocketAddr>>>,
) -> impl Fn(SocketAddr) -> BoxFuture<'static, io::Result<TcpListener>> {
    let failures = AtomicUsize::new(0);
    move |address| {
        locked(&requested).push(address);
        let fails = address.is_ipv6() && failures.fetch_add(1, Ordering::SeqCst) < busy;
        async move {
            if fails {
                return Err(kind.into());
            }
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await
        }
        .boxed()
    }
}

#[tokio::test]
async fn auto_port_skips_a_port_taken_on_ipv6() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let bind = ipv6_failing_bind(io::ErrorKind::AddrInUse, 1, Arc::clone(&requested));
    let bound = bind_local(&bind, LocalPort::Auto(20_000))
        .await
        .expect("the next port is free on both sides");
    assert!(bound.v6.is_some());
    let requests = locked(&requested).clone();
    let ipv4_ports: Vec<u16> = requests
        .iter()
        .filter(|address| address.is_ipv4())
        .map(SocketAddr::port)
        .collect();
    assert_eq!(ipv4_ports, [20_000, 20_001], "the first port was given up");
    assert_eq!(
        requests.len(),
        4,
        "each IPv4 bind was followed by an IPv6 one"
    );
}

#[tokio::test]
async fn auto_port_keeps_ipv4_when_ipv6_is_unavailable() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let bind = ipv6_failing_bind(io::ErrorKind::AddrNotAvailable, 1, Arc::clone(&requested));
    let bound = bind_local(&bind, LocalPort::Auto(20_000))
        .await
        .expect("no IPv6 is not a busy port");
    assert!(bound.v6.is_none());
    assert_eq!(locked(&requested).len(), 2, "no second IPv4 attempt");
}

#[tokio::test]
async fn exact_port_keeps_ipv4_when_ipv6_port_is_taken() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let bind = ipv6_failing_bind(io::ErrorKind::AddrInUse, 1, Arc::clone(&requested));
    let bound = bind_local(&bind, LocalPort::Exact(20_000))
        .await
        .expect("a typed port never moves");
    assert!(bound.v6.is_none());
    assert_eq!(locked(&requested).len(), 2);
}

#[test]
fn candidate_ports_then_os_assigned() {
    let ports: Vec<u16> = candidate_ports(8000).collect();
    let expected: Vec<u16> = (8000..=8020).chain([0]).collect();
    assert_eq!(ports, expected);
    let near_the_top: Vec<u16> = candidate_ports(65_530).collect();
    assert_eq!(
        near_the_top,
        [65_530, 65_531, 65_532, 65_533, 65_534, 65_535, 0]
    );
}

#[test]
fn default_local_port_table() {
    assert_eq!(default_local_port(5432), 15_432);
    assert_eq!(default_local_port(8080), 18_080);
    assert_eq!(default_local_port(9090), 19_090);
    assert_eq!(default_local_port(55_535), 65_535);
    assert_eq!(default_local_port(60_000), 60_000);
}

// --- upgrade refusals and socket ends ---------------------------------------------------------

#[test]
fn upgrade_codes_map_to_socket_errors() {
    assert_eq!(upgrade_error(401), SocketError::Unauthorized);
    assert_eq!(upgrade_error(403), SocketError::Forbidden);
    assert_eq!(upgrade_error(404), SocketError::NotFound);
    assert_eq!(
        upgrade_error(502),
        SocketError::Failed("the port-forward connection was refused (HTTP 502)".to_owned())
    );
    let refused = kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(
        http::StatusCode::FORBIDDEN,
    ));
    assert_eq!(socket_error("prod", refused), SocketError::Forbidden);
}

#[test]
fn socket_outcome_sorts_every_end() {
    assert!(matches!(
        socket_outcome(SocketEnd::Closed),
        SocketAction::Nothing
    ));
    assert!(matches!(
        socket_outcome(SocketEnd::Refused("no listener".to_owned())),
        SocketAction::Refused { reason } if reason == "no listener"
    ));
    assert!(matches!(
        socket_outcome(SocketEnd::Open(SocketError::Unauthorized)),
        SocketAction::End(ForwardError::Unauthorized)
    ));
    assert!(matches!(
        socket_outcome(SocketEnd::Open(SocketError::Forbidden)),
        SocketAction::End(ForwardError::NotPermitted)
    ));
    assert!(matches!(
        socket_outcome(SocketEnd::Open(SocketError::NotFound)),
        SocketAction::CheckPod { reason } if reason == "pod not found"
    ));
    assert!(matches!(
        socket_outcome(SocketEnd::Broken("reset".to_owned())),
        SocketAction::CheckPod { reason } if reason == "reset"
    ));
}

#[tokio::test]
async fn upgrade_403_ends_not_permitted() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    fixture.script(SocketError::Forbidden);
    let (local, _) = fixture.listening().await;
    let _client = connect(local).await;
    let error = fixture.ended().await;
    assert!(matches!(error, ForwardError::NotPermitted));
    assert_eq!(
        error.to_string(),
        "the server refused the port-forward connection (HTTP 403); a forward needs get and \
         create on pods/portforward"
    );
    fixture.stop().await;
}

#[tokio::test]
async fn upgrade_401_ends_unauthorized() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    fixture.script(SocketError::Unauthorized);
    let (local, _) = fixture.listening().await;
    let _client = connect(local).await;
    let error = fixture.ended().await;
    assert!(matches!(error, ForwardError::Unauthorized));
    assert_eq!(
        error.to_string(),
        "the server refused the port-forward connection (HTTP 401)"
    );
    fixture.stop().await;
}

#[tokio::test]
async fn upgrade_404_checks_the_pod_then_reconnects() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        running_then_gone(),
    );
    fixture.script(SocketError::NotFound);
    let (local, _) = fixture.listening().await;
    let _client = connect(local).await;
    assert_eq!(
        fixture.next_event().await,
        ForwardEvent::ConnectionLost {
            reason: "pod not found".to_owned()
        }
    );
    assert_eq!(
        fixture.next_event().await,
        ForwardEvent::Reconnecting { attempt: 1 }
    );
    fixture.stop().await;
}

#[tokio::test]
async fn live_pod_does_not_trigger_reconnect() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    fixture.script(SocketError::Failed("connection reset".to_owned()));
    let (local, _) = fixture.listening().await;
    let _client = connect(local).await;
    assert_eq!(
        fixture.next_event().await,
        ForwardEvent::ConnectionLost {
            reason: "connection reset".to_owned()
        }
    );
    assert!(fixture.is_quiet().await, "no reconnect follows a live pod");
    fixture.stop().await;
}

// --- reconnect --------------------------------------------------------------------------------

/// Runs `reconnect_target` on its own task and reports when each event arrived.
async fn timed_reconnect(
    respond: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static,
) -> (
    Vec<(ForwardEvent, tokio::time::Instant)>,
    Result<ResolvedPod, ForwardError>,
) {
    let (connection, _api) = FakeApi::connection(WritePolicy::Allowed, respond);
    let (updates, mut receiver) = mpsc::unbounded();
    let task = tokio::spawn(async move {
        reconnect_target(
            &connection,
            &request(pod_target(), LocalPort::Auto(18080)),
            &updates,
        )
        .await
    });
    let mut events = Vec::new();
    while let Some(update) = receiver.next().await {
        if let ForwardUpdate::Event(event) = update {
            events.push((event, tokio::time::Instant::now()));
        }
    }
    (events, task.await.expect("the task finishes"))
}

#[tokio::test(start_paused = true)]
async fn reconnect_backoff_is_1_5_15_30_60() {
    let started = tokio::time::Instant::now();
    let (events, result) = timed_reconnect(pod_gone).await;
    let attempts: Vec<(u8, Duration)> = events
        .iter()
        .filter_map(|(event, at)| match event {
            ForwardEvent::Reconnecting { attempt } => Some((*attempt, at.duration_since(started))),
            _ => None,
        })
        .collect();
    // An attempt is announced, waits its delay, then fails: the next one is announced after it.
    let seconds = |count| Duration::from_secs(count);
    assert_eq!(
        attempts,
        [
            (1, seconds(0)),
            (2, seconds(1)),
            (3, seconds(1 + 5)),
            (4, seconds(1 + 5 + 15)),
            (5, seconds(1 + 5 + 15 + 30)),
        ]
    );
    assert!(matches!(result, Err(ForwardError::TargetLost)));
    assert_eq!(
        tokio::time::Instant::now().duration_since(started),
        seconds(1 + 5 + 15 + 30 + 60)
    );
}

#[tokio::test(start_paused = true)]
async fn reconnect_stops_at_once_when_credentials_are_refused() {
    let (events, result) = timed_reconnect(|_| {
        (
            403,
            r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"forbidden","reason":"Forbidden","code":403}"#.to_owned(),
        )
    })
    .await;
    assert!(matches!(
        result,
        Err(ForwardError::Cluster(ClusterError::Forbidden { .. }))
    ));
    assert_eq!(events.len(), 1, "one announced attempt, no retry");
}

#[tokio::test(start_paused = true)]
async fn reconnect_success_emits_reconnected_and_resolved() {
    // GET 0 starts the forward, GET 1 is the "is the pod gone?" check, GET 2 is attempt 1, and
    // GET 3 (attempt 2) finds the pod again.
    let gets = AtomicUsize::new(0);
    let respond = move |request: &RecordedRequest| match gets.fetch_add(1, Ordering::SeqCst) {
        1 | 2 => pod_gone(request),
        _ => pod_running(request),
    };
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        respond,
    )
    .patient();
    fixture.script(SocketError::NotFound);
    let (local, _) = fixture.listening().await;
    let _client = connect(local).await;
    let mut events = Vec::new();
    let resolved = fixture
        .until(|update| match update {
            ForwardUpdate::Event(event) => {
                events.push(event);
                None
            }
            // The first `Resolved` came before the connection; the one after a reconnect is last.
            ForwardUpdate::Resolved { pod, pod_port } if !events.is_empty() => {
                Some((pod, pod_port))
            }
            _ => None,
        })
        .await;
    assert_eq!(
        events,
        [
            ForwardEvent::ConnectionLost {
                reason: "pod not found".to_owned()
            },
            ForwardEvent::Reconnecting { attempt: 1 },
            ForwardEvent::Reconnecting { attempt: 2 },
            ForwardEvent::Reconnected {
                pod: "api-0".to_owned()
            },
        ]
    );
    assert_eq!(resolved, ("api-0".to_owned(), 8080));
    fixture.stop().await;
}

#[tokio::test(start_paused = true)]
async fn five_failures_end_with_target_lost() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        running_then_gone(),
    )
    .patient();
    fixture.script(SocketError::NotFound);
    let (local, _) = fixture.listening().await;
    let _client = connect(local).await;
    let mut attempts = 0;
    let error = fixture
        .until(|update| match update {
            ForwardUpdate::Event(ForwardEvent::Reconnecting { .. }) => {
                attempts += 1;
                None
            }
            ForwardUpdate::Ended(error) => Some(error),
            _ => None,
        })
        .await;
    assert!(matches!(error, ForwardError::TargetLost));
    assert_eq!(attempts, 5);
    assert!(
        TcpStream::connect(SocketAddr::from(local)).await.is_err(),
        "the listener is closed once the forward ended"
    );
    fixture.stop().await;
}

#[tokio::test]
async fn connections_during_reconnect_are_closed() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        running_then_gone(),
    );
    fixture.script(SocketError::NotFound);
    let (local, _) = fixture.listening().await;
    let _first = connect(local).await;
    assert_eq!(
        fixture.next_event().await,
        ForwardEvent::ConnectionLost {
            reason: "pod not found".to_owned()
        }
    );
    assert_eq!(
        fixture.next_event().await,
        ForwardEvent::Reconnecting { attempt: 1 }
    );
    let mut second = connect(local).await;
    assert!(sees_end_of_stream(&mut second).await);
    fixture.stop().await;
}

// --- running forward --------------------------------------------------------------------------

#[tokio::test]
async fn connection_limit_closes_the_65th() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    let (local, _) = fixture.listening().await;
    let mut clients = Vec::new();
    let mut far_ends = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        clients.push(connect(local).await);
        far_ends.push(fixture.far_end().await);
    }
    let mut over_limit = connect(local).await;
    assert!(sees_end_of_stream(&mut over_limit).await);
    assert_eq!(fixture.next_event().await, ForwardEvent::ConnectionLimit);
    let mut another = connect(local).await;
    assert!(sees_end_of_stream(&mut another).await);
    let repeated = fixture.until(|update| match update {
        ForwardUpdate::Event(ForwardEvent::ConnectionLimit) => Some(()),
        _ => None,
    });
    assert!(
        tokio::time::timeout(QUIET, repeated).await.is_err(),
        "the limit is reported once"
    );
    fixture.stop().await;
}

#[tokio::test]
async fn traffic_counts_both_directions_once_per_second() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    let (local, _) = fixture.listening().await;
    let mut client = connect(local).await;
    let mut far = fixture.far_end().await;
    client.write_all(&[1; 10]).await.expect("the client writes");
    let mut received = [0; 10];
    far.stream
        .read_exact(&mut received)
        .await
        .expect("the pod side reads");
    far.stream
        .write_all(&[2; 20])
        .await
        .expect("the pod side writes");
    let mut echoed = [0; 20];
    client
        .read_exact(&mut echoed)
        .await
        .expect("the client reads");

    let traffic = fixture
        .until(|update| match update {
            ForwardUpdate::Traffic(traffic) if traffic.sent == 10 && traffic.received == 20 => {
                Some(traffic)
            }
            _ => None,
        })
        .await;
    assert_eq!(
        traffic,
        ForwardTraffic {
            open_connections: 1,
            received: 20,
            sent: 10
        }
    );
    // Nothing changed since: the next ticks send no sample.
    let unchanged = tokio::time::timeout(
        TRAFFIC_INTERVAL * 2 + QUIET,
        fixture.until(|update| matches!(update, ForwardUpdate::Traffic(_)).then_some(())),
    )
    .await;
    assert!(unchanged.is_err(), "a sample is sent only when it changed");
    fixture.stop().await;
}

#[tokio::test]
async fn error_channel_ends_the_socket_during_the_copy() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    let (local, _) = fixture.listening().await;
    let mut client = connect(local).await;
    let far = fixture.far_end().await;
    // The copy is idle; only the error channel speaks.
    far.error
        .send("connection refused\x07 on port 8080".to_owned())
        .expect("the socket listens");
    assert_eq!(
        fixture.next_event().await,
        ForwardEvent::ConnectionRefused {
            reason: "connection refused on port 8080".to_owned()
        }
    );
    assert!(sees_end_of_stream(&mut client).await);
    fixture.stop().await;
}

#[test]
fn reason_text_strips_and_caps_every_reason() {
    assert_eq!(reason_text("a\x1b[31mb\r\nc\x07"), "a[31mbc");
    let long = "x".repeat(500);
    assert_eq!(reason_text(&long).chars().count(), MAX_REASON_CHARS);
    // The resolve-error path reaches the UI through `ForwardError`'s text.
    let rendered = ClusterError::Rendered {
        message: format!("bad\x1b{}", "y".repeat(500)),
    };
    let text = ForwardError::Cluster(rendered).to_string();
    assert!(!text.contains('\x1b'));
    assert_eq!(text.chars().count(), MAX_REASON_CHARS);
}

#[tokio::test]
async fn a_slow_pod_check_does_not_stall_copies_or_the_lock() {
    // GET 0 starts the forward; every later GET, the pod check, never answers.
    let gets = AtomicUsize::new(0);
    let (connection, api) = FakeApi::with_answer(WritePolicy::Allowed, move |_| {
        let is_start = gets.fetch_add(1, Ordering::SeqCst) == 0;
        async move {
            if is_start {
                reply(200, running_pod().into_bytes())
            } else {
                std::future::pending().await
            }
        }
    });
    let mut fixture = Fixture::start_on(
        connection,
        api,
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
    );
    let (local, _) = fixture.listening().await;
    let mut healthy = connect(local).await;
    let mut healthy_far = fixture.far_end().await;
    fixture.script(SocketError::Failed("connection reset".to_owned()));
    let _failing = connect(local).await;
    assert_eq!(
        fixture.next_event().await,
        ForwardEvent::ConnectionLost {
            reason: "connection reset".to_owned()
        }
    );

    // The check is pending, and the open connection still copies.
    healthy.write_all(b"x").await.expect("the client writes");
    let mut byte = [0; 1];
    healthy_far
        .stream
        .read_exact(&mut byte)
        .await
        .expect("the pod side reads");
    assert_eq!(&byte, b"x");
    // The lock still answers.
    fixture
        .control
        .unbounded_send(ForwardControl::Refuse)
        .expect("the forward listens");
    assert_eq!(fixture.next_event().await, ForwardEvent::Paused);
    fixture
        .control
        .unbounded_send(ForwardControl::Accept)
        .expect("the forward listens");
    assert_eq!(fixture.next_event().await, ForwardEvent::Resumed);
    // New connections are closed while the check runs, as during a reconnect.
    let mut closed = connect(local).await;
    assert!(sees_end_of_stream(&mut closed).await);
    fixture.stop().await;
}

#[tokio::test]
async fn refuse_control_closes_new_connections_only() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    let (local, _) = fixture.listening().await;
    let mut open = connect(local).await;
    let mut open_far = fixture.far_end().await;

    fixture
        .control
        .unbounded_send(ForwardControl::Refuse)
        .expect("the forward listens");
    assert_eq!(fixture.next_event().await, ForwardEvent::Paused);
    let mut refused = connect(local).await;
    assert!(sees_end_of_stream(&mut refused).await);
    // The connection that was open keeps copying.
    open.write_all(b"x").await.expect("the client writes");
    let mut byte = [0; 1];
    open_far
        .stream
        .read_exact(&mut byte)
        .await
        .expect("the pod side reads");
    assert_eq!(&byte, b"x");

    fixture
        .control
        .unbounded_send(ForwardControl::Accept)
        .expect("the forward listens");
    assert_eq!(fixture.next_event().await, ForwardEvent::Resumed);
    let _accepted = connect(local).await;
    let _accepted_far = fixture.far_end().await;
    fixture.stop().await;
}

#[tokio::test]
async fn dropping_the_stream_aborts_sockets() {
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    let (local, _) = fixture.listening().await;
    let _client = connect(local).await;
    let _far = fixture.far_end().await;
    let guards_dropped = Arc::clone(&fixture.guards_dropped);
    assert_eq!(guards_dropped.load(Ordering::SeqCst), 0);
    fixture.stop().await;
    assert_eq!(guards_dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn debug_shows_no_payload() {
    let traffic = ForwardUpdate::Traffic(ForwardTraffic {
        open_connections: 1,
        received: 20,
        sent: 10,
    });
    assert_eq!(
        format!("{traffic:?}"),
        "Traffic(ForwardTraffic { open_connections: 1, received: 20, sent: 10 })"
    );
    let ended = ForwardUpdate::Ended(ForwardError::PortInUse(8080));
    assert_eq!(format!("{ended:?}"), "Ended(PortInUse(8080))");
    let refused = ForwardEvent::ConnectionRefused {
        reason: "no listener".to_owned(),
    };
    assert_eq!(
        format!("{refused:?}"),
        "ConnectionRefused { reason: \"no listener\" }"
    );
}

#[tokio::test]
async fn a_fixture_resolves_through_the_fake_api_only() {
    // The transport tests never reach a cluster: every GET goes to the fake.
    let mut fixture = Fixture::start(
        LocalPort::Auto(18080),
        BindMode::Loopback { is_v6_up: false },
        pod_running,
    );
    fixture.listening().await;
    let requests = fixture.api.requests();
    assert!(requests.iter().all(|request| request.method == "GET"));
    assert!(
        requests
            .iter()
            .all(|request| !request.path.contains("portforward"))
    );
    fixture.stop().await;
}

// --- the production socket opener, over the fake API -------------------------------------------

async fn open_with_status(code: u16) -> (Result<(), SocketError>, Vec<RecordedRequest>) {
    let (connection, api) =
        FakeApi::connection(WritePolicy::Allowed, move |_| (code, "{}".to_owned()));
    let opened = open_remote(connection, NS.to_owned(), "api-0".to_owned(), 8080).await;
    (opened.map(|_socket| ()), api.requests())
}

#[tokio::test]
async fn opening_a_socket_sends_one_get_upgrade_for_one_port() {
    let (opened, requests) = open_with_status(403).await;
    assert_eq!(opened, Err(SocketError::Forbidden));
    let [request] = requests.as_slice() else {
        panic!("expected one request, got {requests:?}");
    };
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.path,
        "/api/v1/namespaces/team-a/pods/api-0/portforward"
    );
    assert!(request.has_query("ports", "8080"), "{}", request.query);
}

#[tokio::test]
async fn opening_a_socket_maps_every_upgrade_refusal() {
    assert_eq!(
        open_with_status(401).await.0,
        Err(SocketError::Unauthorized)
    );
    assert_eq!(open_with_status(404).await.0, Err(SocketError::NotFound));
    assert_eq!(
        open_with_status(500).await.0,
        Err(SocketError::Failed(
            "the port-forward connection was refused (HTTP 500)".to_owned()
        ))
    );
}
