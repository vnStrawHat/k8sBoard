//! Pod exec over kube's `ws` transport (spec 0036). `pod_shell` is the only exec call site: the
//! `pod_shell.rs` row of the 0030 allow-list. It needs an `ExecPermit`, refuses to start while
//! the connection's `WritePolicy` is `Blocked`, and calls no `spawn`: kube's message-loop task
//! belongs to the `AttachedProcess`, which the stream owns, so dropping the stream ends the
//! session.
//!
//! Nothing here logs, traces, or keeps session bytes (C1); traces carry names, durations, byte
//! counts, and the exit code only.

use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::time::Instant;

use futures::future::Either;
use futures::{Sink, SinkExt, Stream, StreamExt, stream};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::Api;
use kube::api::{AttachParams, AttachedProcess, TerminalSize};
use kube::client::UpgradeConnectionError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::connection::{
    ClusterConnection, ClusterError, REQUEST_TIMEOUT, classify_error, run_raw,
};
use crate::object_write::{WriteError, WritePolicy};
use crate::reason_text::reason_text;

const SHELL_ACTION: &str = "opening a shell";
/// The most one `Output` carries: one read of the process's stdout pipe.
const READ_BYTES: usize = 64 * 1024;
/// Input waits here while the pipe to the process is full; past it the input stream is not
/// polled, so a stream of keystrokes cannot grow the backlog without bound. The bound is soft: an
/// item is taken whole, so one large paste (a single `Bytes`) can take the backlog past it by its
/// own size, and no more than that. Capping one paste is the app's job (0036 step 3b).
const STDIN_BACKLOG_BYTES: usize = 256 * 1024;
/// Resolved at run time through the container's `PATH`. The private OSC 7770 names the pick for
/// the tab header.
const AUTO_SCRIPT: &str = r#"for s in bash ash sh; do if command -v "$s" >/dev/null 2>&1; then printf '\033]7770;%s\007' "$s"; exec "$s"; fi; done"#;

/// The size of the terminal in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

/// Which shell to run in the container.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellCommand {
    /// The first of bash, ash, sh the container has; the pick is reported with OSC 7770.
    #[default]
    Auto,
    Bash,
    Sh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellRequest {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub shell: ShellCommand,
    /// The terminal size at open; later sizes arrive as `ShellInput::Resize`.
    pub size: GridSize,
}

/// What the app sends to the shell.
// Debug is manual: byte counts only, never the bytes (C1).
pub enum ShellInput {
    Bytes(Vec<u8>),
    Resize(GridSize),
}

impl fmt::Debug for ShellInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(bytes) => formatter
                .debug_tuple("Bytes")
                .field(&format_args!("{} bytes", bytes.len()))
                .finish(),
            Self::Resize(size) => formatter.debug_tuple("Resize").field(size).finish(),
        }
    }
}

/// What the shell sends back.
// Debug is manual: byte counts only, never the bytes (C1).
pub enum ShellUpdate {
    /// The exec connection is up. Sent once, before any output.
    Started,
    /// One read of the terminal output, at most 64 KiB.
    Output(Vec<u8>),
    /// The process ended. Nothing follows.
    Exited(ShellExit),
    /// The session could not open or broke. Nothing follows.
    Failed(ClusterError),
}

impl fmt::Debug for ShellUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Started => formatter.write_str("Started"),
            Self::Output(bytes) => formatter
                .debug_tuple("Output")
                .field(&format_args!("{} bytes", bytes.len()))
                .finish(),
            Self::Exited(exit) => formatter.debug_tuple("Exited").field(exit).finish(),
            Self::Failed(error) => formatter.debug_tuple("Failed").field(error).finish(),
        }
    }
}

/// How the process ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellExit {
    /// The exit code, when the server reported one.
    pub code: Option<i32>,
    /// The server's own words, when it gave no code (for example "executable file not found").
    pub message: Option<String>,
}

/// Proof that the session SSAR allowed both exec verbs. Not `Clone`: one permit opens one shell.
#[derive(Debug)]
pub struct ExecPermit(());

impl ExecPermit {
    /// Only `AccessReport::exec_permit` calls this, after checking both verbs.
    pub(crate) fn granted() -> Self {
        Self(())
    }

    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self(())
    }
}

impl ClusterConnection {
    /// action: "opening a shell". Nothing happens until polled on the app's tokio runtime.
    ///
    /// Items arrive as `Started`, any number of `Output`, then `Exited`; or one `Failed` instead.
    /// A stream that ends without `Exited` or `Failed` was dropped by its owner. Dropping the
    /// stream ends the session: the WebSocket closes and the remote shell gets a hangup.
    pub fn pod_shell(
        &self,
        _permit: ExecPermit,
        request: ShellRequest,
        input: impl Stream<Item = ShellInput> + Send + Unpin + 'static,
    ) -> impl Stream<Item = ShellUpdate> + Send + 'static {
        let connection = self.clone();
        stream::once(async move {
            match open_session(connection, request, input).await {
                Ok(updates) => Either::Right(stream::iter([ShellUpdate::Started]).chain(updates)),
                Err(error) => Either::Left(stream::iter([ShellUpdate::Failed(error)])),
            }
        })
        .flatten()
    }
}

/// Opens the exec connection. The kill switch comes first: exec can change anything in a
/// container, so it is gated like a write (spec 0030).
async fn open_session(
    connection: ClusterConnection,
    request: ShellRequest,
    input: impl Stream<Item = ShellInput> + Send + Unpin + 'static,
) -> Result<impl Stream<Item = ShellUpdate> + Send + 'static, ClusterError> {
    if connection.write_policy() == WritePolicy::Blocked {
        return Err(ClusterError::Rendered {
            message: WriteError::WritesBlocked.to_string(),
        });
    }
    let api: Api<Pod> = Api::namespaced(connection.client().clone(), &request.namespace);
    // A 64 KiB pipe lets one read return as much as `READ_BYTES` instead of kube's 1 KiB default.
    let params = AttachParams::interactive_tty()
        .container(&request.container)
        .max_stdout_buf_size(READ_BYTES);
    let started = Instant::now();
    let opened = run_raw(exec_process(
        &api,
        &request.pod,
        argv(request.shell),
        &params,
    ))
    .await;
    let mut process = match opened {
        Ok(Ok(process)) => process,
        Ok(Err(error)) => return Err(exec_error(connection.context(), error)),
        Err(_elapsed) => {
            return Err(ClusterError::TimedOut {
                context: connection.context().to_owned(),
                action: SHELL_ACTION,
            });
        }
    };
    tracing::debug!(
        context = %connection.context(),
        namespace = %request.namespace,
        pod = %request.pod,
        container = %request.container,
        elapsed = ?started.elapsed(),
        "shell opened"
    );
    let (Some(stdout), Some(stdin), Some(resize), Some(status)) = (
        process.stdout(),
        process.stdin(),
        process.terminal_size(),
        process.take_status(),
    ) else {
        return Err(ClusterError::Rendered {
            message: "the exec connection has no terminal pipes".to_owned(),
        });
    };
    let io = ProcessIo {
        stdout,
        stdin,
        resize,
        status,
    };
    // The first size goes through the same path as every later one.
    let first_size = stream::iter([ShellInput::Resize(request.size)]);
    Ok(drive(
        process,
        io,
        first_size.chain(input),
        connection.context().to_owned(),
    ))
}

/// The one exec call site, one row of the 0030 exception table (`pod_shell.rs`, `exec`).
#[allow(clippy::disallowed_methods)]
async fn exec_process(
    api: &Api<Pod>,
    pod: &str,
    command: Vec<&'static str>,
    params: &AttachParams,
) -> Result<AttachedProcess, kube::Error> {
    api.exec(pod, command, params).await
}

/// The command to run: bare names, resolved through the container's `PATH`.
fn argv(shell: ShellCommand) -> Vec<&'static str> {
    match shell {
        ShellCommand::Auto => vec!["sh", "-c", AUTO_SCRIPT],
        ShellCommand::Bash => vec!["bash"],
        ShellCommand::Sh => vec!["sh"],
    }
}

/// A refused upgrade carries no `Status` body, so the text is fixed.
fn upgrade_error(code: u16, context: &str) -> ClusterError {
    let context = context.to_owned();
    match code {
        401 => ClusterError::Unauthorized {
            context,
            action: SHELL_ACTION,
            message: "the server refused the exec connection (HTTP 401)".to_owned(),
        },
        403 => ClusterError::Forbidden {
            context,
            action: SHELL_ACTION,
            message: "the server refused the exec connection (HTTP 403); a shell needs get and \
                      create on pods/exec"
                .to_owned(),
        },
        404 => ClusterError::Api {
            context,
            action: SHELL_ACTION,
            code,
            message: "pod or container not found".to_owned(),
        },
        code => ClusterError::Api {
            context,
            action: SHELL_ACTION,
            code,
            message: format!("the exec connection was refused (HTTP {code})"),
        },
    }
}

/// `classify_error` would turn an upgrade refusal into `UnexpectedResponse`, so it is read first.
fn exec_error(context: &str, error: kube::Error) -> ClusterError {
    match error {
        kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(code)) => {
            upgrade_error(code.as_u16(), context)
        }
        other => classify_error(context, SHELL_ACTION, other),
    }
}

/// Reads the final `Status` of the exec stream. A missing status means the connection ended
/// without one.
fn shell_exit(status: Option<Status>) -> ShellExit {
    let Some(status) = status else {
        return ShellExit {
            code: None,
            message: None,
        };
    };
    if status.status.as_deref() == Some("Success") {
        return ShellExit {
            code: Some(0),
            message: None,
        };
    }
    let code = status
        .details
        .iter()
        .flat_map(|details| details.causes.iter().flatten())
        .filter(|cause| cause.reason.as_deref() == Some("ExitCode"))
        .find_map(|cause| cause.message.as_deref()?.parse::<i32>().ok());
    ShellExit {
        code,
        // The server's words reach the tab as a note, so they take the shared sanitizer.
        message: if code.is_some() {
            None
        } else {
            status.message.as_deref().map(reason_text)
        },
    }
}

/// The process ends of one exec connection.
struct ProcessIo<R, W, Z, S> {
    stdout: R,
    stdin: W,
    /// kube's resize channel; a full channel means a resize is already queued.
    resize: Z,
    /// Resolves with the final `Status`, or `None` when the connection ended without one.
    status: S,
}

/// The testable core: any process ends work, including `tokio::io::duplex` halves. `owner` is
/// kept for the life of the stream, so dropping the stream drops it (for the real process that
/// aborts kube's message-loop task).
fn drive<O, R, W, Z, S, I>(
    owner: O,
    io: ProcessIo<R, W, Z, S>,
    input: I,
    context: String,
) -> impl Stream<Item = ShellUpdate> + Send + 'static
where
    O: Send + 'static,
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
    Z: Sink<TerminalSize> + Unpin + Send + 'static,
    S: Future<Output = Option<Status>> + Send + 'static,
    I: Stream<Item = ShellInput> + Unpin + Send + 'static,
{
    let session = Session {
        _owner: owner,
        stdout: io.stdout,
        stdin: io.stdin,
        resize: io.resize,
        status: Box::pin(io.status),
        input,
        context,
        read_buffer: vec![0; READ_BYTES],
        stdin_backlog: Vec::new(),
        pending_resize: None,
        is_input_open: true,
        is_finished: false,
    };
    stream::unfold(session, |mut session| async move {
        let update = session.next_update().await?;
        Some((update, session))
    })
}

struct Session<O, R, W, Z, S, I> {
    _owner: O,
    stdout: R,
    stdin: W,
    resize: Z,
    status: Pin<Box<S>>,
    input: I,
    context: String,
    read_buffer: Vec<u8>,
    /// Input the pipe to the process has not accepted yet.
    stdin_backlog: Vec<u8>,
    /// The latest resize not yet in the channel; a newer one replaces it.
    pending_resize: Option<GridSize>,
    is_input_open: bool,
    is_finished: bool,
}

impl<O, R, W, Z, S, I> Session<O, R, W, Z, S, I>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    Z: Sink<TerminalSize> + Unpin,
    S: Future<Output = Option<Status>>,
    I: Stream<Item = ShellInput> + Unpin,
{
    /// Runs until the next stdout read; input and resizes are served on the way. Every branch is
    /// cancel-safe: `read` and `write` complete or leave the buffers alone, and the pending
    /// bytes and resize live in `self`.
    async fn next_update(&mut self) -> Option<ShellUpdate> {
        if self.is_finished {
            return None;
        }
        loop {
            // Not biased: bulk output must not starve a keystroke such as Ctrl C.
            tokio::select! {
                read = self.stdout.read(&mut self.read_buffer) => {
                    return Some(self.on_read(read).await);
                }
                written = self.stdin.write(&self.stdin_backlog), if !self.stdin_backlog.is_empty() => {
                    self.on_written(written);
                }
                item = self.input.next(),
                    if self.is_input_open && self.stdin_backlog.len() < STDIN_BACKLOG_BYTES => {
                    self.on_input(item);
                }
                is_ready = poll_fn(|cx| self.resize.poll_ready_unpin(cx).map(|ready| ready.is_ok())),
                    if self.pending_resize.is_some() => {
                    self.on_resize_ready(is_ready);
                }
            }
        }
    }

    async fn on_read(&mut self, read: std::io::Result<usize>) -> ShellUpdate {
        match read {
            Ok(count) if count > 0 => ShellUpdate::Output(self.read_buffer[..count].to_vec()),
            // The end of stdout: the final status follows, then the stream ends. A status that
            // never comes must not hold the tab open, so the wait is bounded and reads as "no
            // status", like a connection that ended without one.
            Ok(_) => {
                self.is_finished = true;
                let status = tokio::time::timeout(REQUEST_TIMEOUT, self.status.as_mut())
                    .await
                    .unwrap_or(None);
                ShellUpdate::Exited(shell_exit(status))
            }
            Err(error) => {
                self.is_finished = true;
                ShellUpdate::Failed(ClusterError::Unreachable {
                    context: self.context.clone(),
                    action: SHELL_ACTION,
                    source: Box::new(error),
                })
            }
        }
    }

    fn on_written(&mut self, written: std::io::Result<usize>) {
        match written {
            Ok(count) if count > 0 => {
                self.stdin_backlog.drain(..count);
            }
            // The process side is gone; stdout reaches its end next.
            Ok(_) | Err(_) => {
                self.stdin_backlog.clear();
                self.is_input_open = false;
            }
        }
    }

    fn on_input(&mut self, item: Option<ShellInput>) {
        match item {
            Some(ShellInput::Bytes(bytes)) => self.stdin_backlog.extend_from_slice(&bytes),
            Some(ShellInput::Resize(size)) => self.pending_resize = Some(size),
            None => self.is_input_open = false,
        }
    }

    /// kube's resize sender is an `mpsc::Sender`, which needs no flush after `start_send`. A
    /// closed channel means the exec ended, so the resize is dropped.
    fn on_resize_ready(&mut self, is_ready: bool) {
        let Some(size) = self.pending_resize.take() else {
            return;
        };
        if is_ready {
            let _ = self.resize.start_send_unpin(TerminalSize {
                width: size.cols,
                height: size.rows,
            });
        }
    }
}

#[cfg(test)]
#[path = "pod_shell_tests.rs"]
mod pod_shell_tests;
