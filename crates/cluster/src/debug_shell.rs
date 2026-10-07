//! Attaching a shell to a container k8sBoard created (spec 0037): a debug container or a node shell
//! pod, or (0040) to a running container of a pod's own spec that has a terminal. `debug_shell` is
//! the only attach call site: the `debug_shell.rs` row of the 0030 allow-list. It needs an
//! `AttachPermit`, refuses to start while the connection's `WritePolicy` is `Blocked`, and waits for
//! the container to run with a 1 s GET poll (no watch) before it attaches. It calls no `spawn`: the
//! stream owns the `AttachedProcess`, so dropping the stream ends the attach, and `stdinOnce` ends
//! the shell with it.
//!
//! Nothing here logs, traces, or keeps session bytes (C1); the poll keeps fixed reason words and
//! drops the server's messages. The one exception is a failed image pull, which reads the newest
//! `Failed` event of the pod once, keeps one masked line, and sends it as `ImagePullFailed`.

use std::pin::Pin;
use std::time::Duration;

use futures::{Stream, StreamExt, stream};
use k8s_openapi::api::core::v1::{ContainerState, ContainerStatus, Event, Pod};
use kube::Api;
use kube::api::{AttachParams, AttachedProcess, ListParams};
use tokio::time::Instant;

use crate::connection::{ClusterConnection, ClusterError, run_raw};
use crate::object_write::{WriteError, WritePolicy};
use crate::object_yaml::mask_url_userinfo;
use crate::pod_shell::{
    GridSize, READ_BYTES, ShellInput, ShellUpdate, UpgradeVerb, connect_error, drive_process,
};

const ATTACH: UpgradeVerb = UpgradeVerb {
    action: "attaching a debug shell",
    verb: "attach",
};
const WAIT_ACTION: &str = "waiting for the debug container";
const CONTAINER_ATTACH: UpgradeVerb = UpgradeVerb {
    action: "attaching to a container",
    verb: "attach",
};
const CONTAINER_WAIT_ACTION: &str = "waiting for the container";
/// How often the pod is read while the container starts.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Image pulls are slow, but a container that has not run by now is not going to.
const WAIT_CAP: Duration = Duration::from_secs(120);
const NO_SHELL: &str = "the node has no shell; node shell needs sh on the host";
/// The exit codes of a shell that is missing (127) or cannot run (126).
const NO_SHELL_EXIT_CODES: [i32; 2] = [126, 127];
/// The waiting reasons of an image that cannot be pulled; the user is told which image and why.
const IMAGE_PULL_REASONS: [&str; 2] = ["ErrImagePull", "ImagePullBackOff"];
/// Waiting reasons that will not clear by themselves.
const FATAL_WAITING_REASONS: [&str; 4] = [
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
];
const MAX_REASON_CHARS: usize = 64;

/// Proof that the session SSAR allowed both attach verbs. Not `Clone`: one permit opens one attach.
#[derive(Debug)]
pub struct AttachPermit(());

impl AttachPermit {
    /// Only `AccessReport::attach_permit` calls this, after checking both verbs.
    pub(crate) fn granted() -> Self {
        Self(())
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self(())
    }
}

/// Which status list holds the container: a node shell pod's own container, an ephemeral one, or
/// (0040) a running container of the pod's spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachWait {
    NodeShellPod,
    EphemeralContainer,
    Container,
}

impl AttachWait {
    fn attach_verb(self) -> UpgradeVerb {
        match self {
            Self::Container => CONTAINER_ATTACH,
            Self::NodeShellPod | Self::EphemeralContainer => ATTACH,
        }
    }

    fn wait_action(self) -> &'static str {
        match self {
            Self::Container => CONTAINER_WAIT_ACTION,
            Self::NodeShellPod | Self::EphemeralContainer => WAIT_ACTION,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachRequest {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub wait: AttachWait,
    /// The terminal size at attach; later sizes arrive as `ShellInput::Resize`.
    pub size: GridSize,
}

/// Where a container stands, from one read of its pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    /// Not running yet; the fixed reason word when the pod reports one.
    Waiting(Option<String>),
    Running,
    Failed(String),
    /// The image could not be pulled (`ErrImagePull` or `ImagePullBackOff`).
    ImagePullFailed,
}

type Input = Box<dyn Stream<Item = ShellInput> + Send + Unpin>;
type Driven = Pin<Box<dyn Stream<Item = ShellUpdate> + Send>>;

impl ClusterConnection {
    /// action: "attaching a debug shell". Nothing happens until polled on the app's tokio runtime.
    ///
    /// Items arrive as any number of `Waiting`, then `Started`, any number of `Output`, then
    /// `Exited`; or one `Failed` instead. A stream that ends without `Exited` or `Failed` was
    /// dropped by its owner. Dropping the stream ends the attach: the WebSocket closes and the
    /// shell gets a hangup.
    pub fn attach_shell(
        &self,
        _permit: AttachPermit,
        request: AttachRequest,
        input: impl Stream<Item = ShellInput> + Send + Unpin + 'static,
    ) -> impl Stream<Item = ShellUpdate> + Send + 'static {
        let start = if self.write_policy() == WritePolicy::Blocked {
            // The kill switch comes first: nothing was created, so there is nothing to attach to.
            Phase::Finished(Some(ClusterError::Rendered {
                message: WriteError::WritesBlocked.to_string(),
            }))
        } else {
            Phase::Waiting(Wait::new(self.clone(), request, Box::new(input)))
        };
        stream::unfold(start, step)
    }
}

/// The state of one attach: polling the pod, attaching, driving the process, or done.
enum Phase {
    Waiting(Wait),
    Attaching(Wait),
    Driving(Driven),
    /// Done; holds the failure still to report, if any.
    Finished(Option<ClusterError>),
}

struct Wait {
    connection: ClusterConnection,
    request: AttachRequest,
    input: Input,
    started: Instant,
    is_first_poll: bool,
    /// The latest waiting reason, and whether the app has been told it.
    reason: Option<String>,
    is_reason_sent: bool,
}

impl Wait {
    fn new(connection: ClusterConnection, request: AttachRequest, input: Input) -> Self {
        Self {
            connection,
            request,
            input,
            started: Instant::now(),
            is_first_poll: true,
            reason: None,
            is_reason_sent: true,
        }
    }
}

/// One read of the pod; `None` when it is gone.
async fn read_pod(
    connection: &ClusterConnection,
    request: &AttachRequest,
) -> Result<Option<Pod>, ClusterError> {
    let api: Api<Pod> = Api::namespaced(connection.client().clone(), &request.namespace);
    connection
        .run(request.wait.wait_action(), api.get_opt(&request.pod))
        .await
}

async fn step(phase: Phase) -> Option<(ShellUpdate, Phase)> {
    let mut phase = phase;
    loop {
        phase = match phase {
            Phase::Waiting(mut wait) => {
                if !std::mem::take(&mut wait.is_first_poll) {
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
                let pod = match read_pod(&wait.connection, &wait.request).await {
                    Ok(pod) => pod,
                    Err(error) => return Some(failed(error)),
                };
                let readiness = readiness(pod.as_ref(), &wait.request.container, wait.request.wait);
                match readiness {
                    Readiness::Running => Phase::Attaching(wait),
                    Readiness::Failed(message) => {
                        return Some(failed(ClusterError::Rendered { message }));
                    }
                    Readiness::ImagePullFailed => {
                        let detail = pull_failure_detail(&wait.connection, &wait.request).await;
                        let update = ShellUpdate::ImagePullFailed { detail };
                        return Some((update, Phase::Finished(None)));
                    }
                    Readiness::Waiting(reason) => {
                        if wait.started.elapsed() >= WAIT_CAP {
                            return Some(failed(timeout_error(wait.reason.as_deref())));
                        }
                        if reason.is_some() && reason != wait.reason {
                            wait.reason.clone_from(&reason);
                            wait.is_reason_sent = false;
                        }
                        if wait.is_reason_sent {
                            Phase::Waiting(wait)
                        } else {
                            wait.is_reason_sent = true;
                            let update =
                                ShellUpdate::Waiting(wait.reason.clone().unwrap_or_default());
                            return Some((update, Phase::Waiting(wait)));
                        }
                    }
                }
            }
            Phase::Attaching(wait) => {
                let Wait {
                    connection,
                    request,
                    input,
                    ..
                } = wait;
                match attach(&connection, &request, input).await {
                    Ok(driven) => return Some((ShellUpdate::Started, Phase::Driving(driven))),
                    Err(error) => return Some(failed(error)),
                }
            }
            Phase::Driving(mut driven) => {
                let update = driven.next().await?;
                return Some((update, Phase::Driving(driven)));
            }
            Phase::Finished(Some(error)) => return Some(failed(error)),
            Phase::Finished(None) => return None,
        };
    }
}

/// The cause of a failed pull from the newest `Failed` event of the pod, or `None` when the events
/// cannot be read or say nothing: the user is still told the image could not be pulled. This read
/// is best effort, so its own failure is swallowed.
async fn pull_failure_detail(
    connection: &ClusterConnection,
    request: &AttachRequest,
) -> Option<String> {
    let api: Api<Event> = Api::namespaced(connection.client().clone(), &request.namespace);
    let params = ListParams::default().fields(&format!(
        "involvedObject.name={},reason=Failed",
        request.pod
    ));
    let events = connection
        .run("reading the image pull events", api.list(&params))
        .await
        .ok()?;
    let newest = events
        .items
        .iter()
        .filter(|event| {
            event
                .message
                .as_deref()
                .is_some_and(|message| message.starts_with(PULL_FAILURE_PREFIX))
        })
        .max_by_key(|event| event.last_timestamp.as_ref().map(|time| time.0))?;
    pull_cause(newest.message.as_deref()?)
}

/// What the kubelet starts every pull failure message with.
const PULL_FAILURE_PREFIX: &str = "Failed to pull image \"";
/// The longest cause kept: a registry lookup failure quotes the image, the URL and the resolver,
/// about 300 characters, and the tab wraps it.
const MAX_CAUSE_CHARS: usize = 600;

/// The cause of a kubelet pull failure message on one line: the head repeats the image name, which
/// the app already shows, so it is cut off. Credentials in a URL are masked.
fn pull_cause(message: &str) -> Option<String> {
    let tail = message
        .split_once("\": ")
        .map_or(message, |(_, tail)| tail)
        .lines()
        .next()?
        .trim();
    let masked = mask_url_userinfo(tail);
    let tail = masked.as_deref().unwrap_or(tail);
    let mut cause: String = tail
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_CAUSE_CHARS)
        .collect();
    if tail.chars().count() > MAX_CAUSE_CHARS {
        cause.push('…');
    }
    (!cause.is_empty()).then_some(cause)
}

fn failed(error: ClusterError) -> (ShellUpdate, Phase) {
    (ShellUpdate::Failed(error), Phase::Finished(None))
}

fn timeout_error(last_reason: Option<&str>) -> ClusterError {
    let reason = last_reason.unwrap_or("no status yet");
    ClusterError::Rendered {
        message: format!(
            "container did not start within {} s ({reason})",
            WAIT_CAP.as_secs()
        ),
    }
}

/// Opens the attach connection and hands the pipes to the shared driver.
async fn attach(
    connection: &ClusterConnection,
    request: &AttachRequest,
    input: Input,
) -> Result<Driven, ClusterError> {
    let api: Api<Pod> = Api::namespaced(connection.client().clone(), &request.namespace);
    // A 64 KiB pipe lets one read return as much as `READ_BYTES` instead of kube's 1 KiB default.
    let params = AttachParams::interactive_tty()
        .container(&request.container)
        .max_stdout_buf_size(READ_BYTES);
    let opened = run_raw(attach_process(&api, &request.pod, &params)).await;
    let process = match opened {
        Ok(Ok(process)) => process,
        Ok(Err(error)) => {
            return Err(connect_error(
                connection.context(),
                request.wait.attach_verb(),
                error,
            ));
        }
        Err(_elapsed) => {
            return Err(ClusterError::TimedOut {
                context: connection.context().to_owned(),
                action: request.wait.attach_verb().action,
            });
        }
    };
    tracing::debug!(
        context = %connection.context(),
        namespace = %request.namespace,
        pod = %request.pod,
        container = %request.container,
        "debug shell attached"
    );
    let driven = drive_process(
        process,
        connection.context().to_owned(),
        request.size,
        input,
    )?;
    Ok(Box::pin(driven))
}

/// The one attach call site, one row of the 0030 exception table (`debug_shell.rs`, `attach`).
#[allow(clippy::disallowed_methods)]
async fn attach_process(
    api: &Api<Pod>,
    pod: &str,
    params: &AttachParams,
) -> Result<AttachedProcess, kube::Error> {
    api.attach(pod, params).await
}

/// Where `container` stands in `pod`. Waiting and termination messages are dropped: only the fixed
/// reason words and exit codes are read.
pub(crate) fn readiness(pod: Option<&Pod>, container: &str, wait: AttachWait) -> Readiness {
    let Some(pod) = pod else {
        return Readiness::Failed("the pod no longer exists".to_owned());
    };
    let Some(status) = pod.status.as_ref() else {
        return Readiness::Waiting(None);
    };
    if matches!(status.phase.as_deref(), Some("Failed" | "Succeeded")) {
        return Readiness::Failed("the pod has ended".to_owned());
    }
    // Native sidecars report in the init status list.
    let found = match wait {
        AttachWait::NodeShellPod => find_status(status.container_statuses.as_deref(), container),
        AttachWait::EphemeralContainer => {
            find_status(status.ephemeral_container_statuses.as_deref(), container)
        }
        AttachWait::Container => find_status(status.container_statuses.as_deref(), container)
            .or_else(|| find_status(status.init_container_statuses.as_deref(), container)),
    };
    match found.and_then(|status| status.state.as_ref()) {
        Some(state) => state_readiness(state, wait),
        None => Readiness::Waiting(None),
    }
}

fn find_status<'a>(
    statuses: Option<&'a [ContainerStatus]>,
    container: &str,
) -> Option<&'a ContainerStatus> {
    statuses
        .unwrap_or_default()
        .iter()
        .find(|status| status.name == container)
}

fn state_readiness(state: &ContainerState, wait: AttachWait) -> Readiness {
    if state.running.is_some() {
        return Readiness::Running;
    }
    if let Some(terminated) = &state.terminated {
        if wait == AttachWait::NodeShellPod && NO_SHELL_EXIT_CODES.contains(&terminated.exit_code) {
            return Readiness::Failed(NO_SHELL.to_owned());
        }
        let reason = terminated.reason.as_deref().map(fixed_word);
        return Readiness::Failed(match reason {
            Some(reason) if !reason.is_empty() => format!("the container ended ({reason})"),
            _ => "the container ended".to_owned(),
        });
    }
    let reason = state
        .waiting
        .as_ref()
        .and_then(|waiting| waiting.reason.as_deref())
        .map(fixed_word)
        .filter(|reason| !reason.is_empty());
    if reason
        .as_deref()
        .is_some_and(|reason| IMAGE_PULL_REASONS.contains(&reason))
    {
        return Readiness::ImagePullFailed;
    }
    match reason {
        Some(reason) if FATAL_WAITING_REASONS.contains(&reason.as_str()) => {
            Readiness::Failed(format!("the container could not start: {reason}"))
        }
        reason => Readiness::Waiting(reason),
    }
}

/// A reason as the one CamelCase word it should be: letters and digits only, so server text cannot
/// smuggle anything else into a notice.
fn fixed_word(reason: &str) -> String {
    reason
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(MAX_REASON_CHARS)
        .collect()
}

#[cfg(test)]
#[path = "debug_shell_tests.rs"]
mod debug_shell_tests;
