# 0035 · Forward transport (cluster crate)

[Back to index](README.md) · Step 1 · Modules: `port_forward.rs` (new) + `port_forward_tests.rs`, `access_review.rs`, `lib.rs`, `Cargo.toml`. Decisions 1–19.

## Verified APIs (kube 4.2, `.cargo-home/registry`)

| API | Where |
|---|---|
| `Api::<Pod>::portforward(name, &[u16]) -> Result<Portforwarder>` (`ws`); `Client::connect` → `Error::UpgradeConnection(ProtocolSwitch(StatusCode))` on a non-101 | `kube-client-4.2.0/src/api/subresource.rs:777`, `client/mod.rs:261`, `client/upgrade.rs:94` |
| `Portforwarder::{take_stream(port), take_error(port), abort(), join()}`; `new` calls `tokio::spawn` and **dropping it does not abort** the loop | `kube-client-4.2.0/src/api/portforward.rs:88–160` |
| tokio `TcpListener::bind`, `copy_bidirectional` | tokio `net`, `io-util` (already enabled in the lock; declared explicitly in `crates/cluster/Cargo.toml`) |

## API (`port_forward.rs`, `pub use` in `lib.rs`; no kube type in a signature)

```rust
/// Proof that the session SSAR allowed both port-forward verbs. Not `Clone`.
pub struct PortForwardPermit(());
impl AccessReport { pub fn port_forward_permit(&self) -> Option<PortForwardPermit>; } // access_review.rs
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ForwardTarget { Pod { name: String }, Service { name: String }, Deployment { name: String }, StatefulSet { name: String } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalPort { Auto(u16) /* preferred */, Exact(u16) }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardRequest { pub namespace: String, pub target: ForwardTarget, pub remote_port: u16, pub local_port: LocalPort }
pub enum ForwardUpdate {
    Listening { local: SocketAddrV4, has_ipv6: bool }, // once, after bind; `has_ipv6`: the `[::1]` listener is up
    Resolved { pod: String, pod_port: u16 },     // after start and after each reconnect
    Traffic(ForwardTraffic),                     // ≤ 1/s, only when changed
    Event(ForwardEvent),
    Ended(ForwardError),                         // last item
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForwardTraffic { pub open_connections: u32, pub received: u64, pub sent: u64 }
pub enum ForwardEvent { ConnectionLost { reason: String }, Reconnecting { attempt: u8 }, Reconnected { pod: String },
    ConnectionRefused { reason: String }, ConnectionLimit, Paused, Resumed }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ForwardControl { Accept, Refuse } // lock state (decision 22)
impl ClusterConnection {
    /// action: "forwarding a port". Nothing happens until polled on the app's tokio runtime.
    pub fn port_forward(&self, permit: PortForwardPermit, request: ForwardRequest,
        control: impl Stream<Item = ForwardControl> + Send + Unpin + 'static) -> impl Stream<Item = ForwardUpdate> + Send + 'static;
}
pub fn default_local_port(remote: u16) -> u16; // decision 17
```

`ForwardError` (`thiserror`): `WritesBlocked`, `PortInUse(u16)`, `PortReserved(u16)` (`Port {n} is reserved or needs more rights`), `Bind(String)`, `NotPermitted`, `Unauthorized`, `TargetNotFound`, `NoReadyPod`, `PortNotDeclared(String)` (named `targetPort` missing in the pod), `UnsupportedService` (no selector, ExternalName), `TargetLost`, `Cluster(ClusterError)`. `Display` texts are fixed (they name objects, never bytes).

## RBAC (decision 7)

- New `AccessCheck::GetPodPortForward` = `("get", "", "pods", Some("portforward"), true)`; `CreatePodPortForward` exists. `ALL` grows by one.
- `port_forward_permit` is `Some` only when both are allowed. The app gate reads the same two checks; reason `Not permitted: get and create pods/portforward`.

## Flow inside the stream

The stream is `stream::select(updates_rx, driver.into_stream().filter_map(|_| ready(None)))`: one `driver` future, no spawn. The driver is private and generic over its socket opener (`open: impl Fn(String, u16) -> F`, `F: Future<Output = Result<impl AsyncRead + AsyncWrite, SocketError>>`): production passes the kube `portforward` call; tests pass `tokio::io::duplex` halves or scripted errors (a 101 upgrade cannot be faked through `service_fn`).

1. Policy `Blocked` → `Ended(WritesBlocked)`, zero requests.
2. **Resolve** (`resolve_target`, GETs/lists through `connection.run`): Pod → GET; Service → GET service, list pods in the namespace, filter with `Selector::of_labels` + `ready_pod`, map the port with `service_target_port`; Deployment/StatefulSet → GET, `Selector::of(spec.selector)`, list, `ready_pod`; `remote_port` is the pod port. Errors: 404 → `TargetNotFound`; none ready → `NoReadyPod`.
3. **Bind** (`bind_local`): `127.0.0.1:p` first. `Exact(p)`: `AddrInUse` → `Ended(PortInUse(p))`, `PermissionDenied` (Windows WSAEACCES, excluded range) → `Ended(PortReserved(p))`. `Auto(p)`: both errors → next of `candidate_ports(p)` = `p..=p+20`, then `0`. Then best effort `[::1]:{bound port}`: any error is ignored (no IPv6, `AddrNotAvailable`), **except** that for `Auto` an `AddrInUse` on `[::1]` counts as a busy port, drops the IPv4 listener, and moves to the next candidate (changed 2026-10-03 by the orchestrator after the W2 security review: a port another process holds on `[::1]` would shadow the forward for `localhost` clients that resolve to `::1` first; `Exact` keeps the IPv4 listener, as before). Emit `Listening`, `Resolved`.
4. **Loop** (`select!`): accept on either listener → a `ForwardSocket` future in a `FuturesUnordered` (closed at once while reconnecting, while `Refuse` is the last control (`Paused` / `Resumed` events on change), or at 64 open, with `ConnectionLimit` once); socket end → classify; 1 s tick → `Traffic` if changed; reconnect timer.
5. `ForwardSocket`: `connection.run_raw(ACTION, api.portforward(pod, &[port]))` (0030 `run_raw`, 30 s open timeout, so `UpgradeConnection` reaches `upgrade_error` before `classify_error` turns it into `UnexpectedResponse`), `take_stream` + `take_error`, then `select!` over `copy_bidirectional(Counting(local), Counting(remote))` and the `take_error` future: an error-channel message ends the socket at once (decision 30). A guard calls `Portforwarder::abort()` on drop (kube detaches its task otherwise). Counting wraps `AsyncRead` and adds to `Arc<AtomicU64>`s (`sent` = local reads, `received` = remote reads); `open_connections` is the set length.
6. **Socket end classification** (pure `socket_outcome`): clean EOF → nothing; error channel text → `ConnectionRefused { reason }` (through `reason_text`); upgrade 403 → `Ended(NotPermitted)`, 401 → `Ended(Unauthorized)`; 404, WebSocket error, or open failure → GET the pod (`pod_is_gone`): gone, not Running, or deleting → reconnect; alive → `ConnectionLost { reason }` only.
7. **Reconnect**: `Event(ConnectionLost)`, then for attempt 1..=5: `Reconnecting { attempt }`, wait `RECONNECT_DELAYS[attempt-1]` (1, 5, 15, 30, 60 s), re-run step 2 → `Reconnected { pod }` + `Resolved`; after 5 failures → `Ended(TargetLost)`. The listener stays bound throughout.

## Upgrade errors (mandatory mapping, `upgrade_error`)

| Code | Result | Text |
|---|---|---|
| 401 | `ForwardError::Unauthorized` | `the server refused the port-forward connection (HTTP 401)` |
| 403 | `ForwardError::NotPermitted` | `the server refused the port-forward connection (HTTP 403); a forward needs get and create on pods/portforward` |
| 404 | reconnect check (step 6) | `pod not found` |
| other | `ClusterError::Api { code }` | `the port-forward connection was refused (HTTP {code})` |

`Error::Api` 401/403/404 on the resolve GETs use the existing `classify_error`.

## Pure helpers (unit-tested)

`ready_pod(&[Pod]) -> Option<&Pod>`, `service_target_port(&ServicePort, &Pod) -> Result<u16, ForwardError>`, `candidate_ports(u16) -> impl Iterator<Item = u16>`, `bind_outcome(io::ErrorKind, LocalPort) -> BindStep` (next port / `PortInUse` / `PortReserved`), `reason_text(&str) -> String` (control characters stripped, ≤ 200 chars; used for every reason string), `default_local_port(u16)`, `socket_outcome(..)`, `upgrade_error(StatusCode)`, `RECONNECT_DELAYS`.

## Safety

- Nothing traces payload bytes; traces carry action, target names, ports, byte counts, durations, codes.
- The permit, `WritePolicy`, and the allow-list row (0030 write-path.md): `pods/portforward`, `GET` + WebSocket upgrade, `/api/v1/namespaces/{ns}/pods/{pod}/portforward?ports={p}`, dry-run no, 0035. The documentation grep expects `access_review.rs`, `object_write.rs`, `pod_shell.rs`, `port_forward.rs`.
