use std::future::ready;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::mpsc::{self, TryRecvError};
use futures::{StreamExt, stream};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{StatusCause, StatusDetails};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};

use super::*;
use crate::fake_api::FakeApi;

const WAIT: Duration = Duration::from_secs(5);
const PIPE_BYTES: usize = 1 << 20;

fn size(cols: u16, rows: u16) -> GridSize {
    GridSize { cols, rows }
}

fn request(shell: ShellCommand) -> ShellRequest {
    ShellRequest {
        namespace: "team-a".to_owned(),
        pod: "api-0".to_owned(),
        container: "api".to_owned(),
        shell,
        size: size(100, 30),
    }
}

fn exit_status(code: &str) -> Status {
    Status {
        status: Some("Failure".to_owned()),
        reason: Some("NonZeroExitCode".to_owned()),
        message: Some("command terminated with non-zero exit code".to_owned()),
        details: Some(StatusDetails {
            causes: Some(vec![StatusCause {
                reason: Some("ExitCode".to_owned()),
                message: Some(code.to_owned()),
                ..StatusCause::default()
            }]),
            ..StatusDetails::default()
        }),
        ..Status::default()
    }
}

/// A fake process: the test writes stdout and reads stdin, and `drive` runs on its own task.
struct Harness {
    stdout: DuplexStream,
    stdin: DuplexStream,
    resize: mpsc::Receiver<TerminalSize>,
    input: mpsc::UnboundedSender<ShellInput>,
    updates: mpsc::UnboundedReceiver<ShellUpdate>,
}

impl Harness {
    fn start(status: Option<Status>) -> Self {
        let (resize_sender, resize) = mpsc::channel(8);
        Self::start_with(resize_sender, resize, ready(status))
    }

    fn start_with(
        resize_sender: mpsc::Sender<TerminalSize>,
        resize: mpsc::Receiver<TerminalSize>,
        status: impl Future<Output = Option<Status>> + Send + 'static,
    ) -> Self {
        let (stdout, stdout_reader) = duplex(PIPE_BYTES);
        let (stdin_writer, stdin) = duplex(PIPE_BYTES);
        let (input, input_receiver) = mpsc::unbounded();
        let io = ProcessIo {
            stdout: stdout_reader,
            stdin: stdin_writer,
            resize: resize_sender,
            status,
        };
        let updates = drive((), io, input_receiver, "fake".to_owned());
        let (sender, receiver) = mpsc::unbounded();
        tokio::spawn(async move {
            let mut updates = Box::pin(updates);
            while let Some(update) = updates.next().await {
                if sender.unbounded_send(update).is_err() {
                    break;
                }
            }
        });
        Self {
            stdout,
            stdin,
            resize,
            input,
            updates: receiver,
        }
    }

    async fn next_update(&mut self) -> Option<ShellUpdate> {
        tokio::time::timeout(WAIT, self.updates.next())
            .await
            .expect("an update arrives in time")
    }

    async fn next_output(&mut self) -> Vec<u8> {
        match self.next_update().await {
            Some(ShellUpdate::Output(bytes)) => bytes,
            other => panic!("expected output, got {other:?}"),
        }
    }
}

#[test]
fn argv_auto_loops_bash_ash_sh_and_reports_the_pick() {
    assert_eq!(
        argv(ShellCommand::Auto),
        [
            "sh",
            "-c",
            r#"for s in bash ash sh; do if command -v "$s" >/dev/null 2>&1; then printf '\033]7770;%s\007' "$s"; exec "$s"; fi; done"#
        ]
    );
    assert_eq!(argv(ShellCommand::Bash), ["bash"]);
    assert_eq!(argv(ShellCommand::Sh), ["sh"]);
}

#[tokio::test(start_paused = true)]
async fn drive_forwards_stdout_as_output() {
    let mut harness = Harness::start(None);
    harness.stdout.write_all(b"hello").await.expect("pipe open");
    assert_eq!(harness.next_output().await, b"hello");

    let bulk = vec![b'x'; 150_000];
    harness.stdout.write_all(&bulk).await.expect("pipe open");
    let mut received = 0;
    let mut chunks = 0;
    while received < bulk.len() {
        let chunk = harness.next_output().await;
        assert!(chunk.len() <= READ_BYTES, "a read is at most 64 KiB");
        received += chunk.len();
        chunks += 1;
    }
    assert_eq!(received, bulk.len());
    assert!(
        chunks >= 3,
        "150 000 bytes need at least three 64 KiB reads"
    );
}

#[tokio::test(start_paused = true)]
async fn drive_writes_input_bytes_to_stdin() {
    let mut harness = Harness::start(None);
    harness
        .input
        .unbounded_send(ShellInput::Bytes(b"ls\r".to_vec()))
        .expect("the session listens");
    let mut written = [0; 3];
    tokio::time::timeout(WAIT, harness.stdin.read_exact(&mut written))
        .await
        .expect("the bytes arrive in time")
        .expect("pipe open");
    assert_eq!(&written, b"ls\r");
}

#[tokio::test(start_paused = true)]
async fn drive_forwards_resize_latest_wins() {
    // A channel with no buffer holds one message: filling it makes the sink "full".
    let (mut sender, resize) = mpsc::channel(0);
    sender
        .try_send(TerminalSize {
            width: 1,
            height: 1,
        })
        .expect("the first message fits");
    let mut harness = Harness::start_with(sender, resize, ready(None));
    for (cols, rows) in [(10, 5), (20, 10), (30, 15)] {
        harness
            .input
            .unbounded_send(ShellInput::Resize(size(cols, rows)))
            .expect("the session listens");
    }
    // Let the session take all three while the sink is full.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let filler = harness.resize.next().await.expect("the filler is queued");
    assert_eq!((filler.width, filler.height), (1, 1));
    let delivered = tokio::time::timeout(WAIT, harness.resize.next())
        .await
        .expect("the latest resize arrives once the sink has room")
        .expect("the channel is open");
    assert_eq!((delivered.width, delivered.height), (30, 15));

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        matches!(harness.resize.try_recv(), Err(TryRecvError::Empty)),
        "the superseded resizes were dropped"
    );
}

#[tokio::test(start_paused = true)]
async fn drive_ends_with_exited_after_stdout_eof() {
    let mut harness = Harness::start(Some(exit_status("2")));
    harness.stdout.write_all(b"bye").await.expect("pipe open");
    harness.stdout.shutdown().await.expect("pipe closes");
    assert_eq!(harness.next_output().await, b"bye");
    match harness.next_update().await {
        Some(ShellUpdate::Exited(exit)) => assert_eq!(
            exit,
            ShellExit {
                code: Some(2),
                message: None
            }
        ),
        other => panic!("expected exit, got {other:?}"),
    }
    assert!(harness.next_update().await.is_none(), "nothing follows");
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn dropping_the_stream_drops_the_owner() {
    let is_dropped = Arc::new(AtomicBool::new(false));
    let (stdout, _stdout_writer) = duplex(8);
    let (stdin, _stdin_reader) = duplex(8);
    let (resize, _receiver) = mpsc::channel(1);
    let io = ProcessIo {
        stdout,
        stdin,
        resize,
        status: ready(None),
    };
    let updates = drive(
        DropFlag(Arc::clone(&is_dropped)),
        io,
        stream::pending::<ShellInput>(),
        "fake".to_owned(),
    );
    assert!(!is_dropped.load(Ordering::SeqCst));
    drop(updates);
    assert!(is_dropped.load(Ordering::SeqCst));
}

#[test]
fn shell_exit_reads_success_failure_and_unknown() {
    let success = Status {
        status: Some("Success".to_owned()),
        ..Status::default()
    };
    assert_eq!(
        shell_exit(Some(success)),
        ShellExit {
            code: Some(0),
            message: None
        }
    );
    assert_eq!(
        shell_exit(Some(exit_status("137"))),
        ShellExit {
            code: Some(137),
            message: None
        }
    );
    assert_eq!(
        shell_exit(None),
        ShellExit {
            code: None,
            message: None
        }
    );
    let not_found = Status {
        status: Some("Failure".to_owned()),
        message: Some("executable file not found in $PATH".to_owned()),
        ..Status::default()
    };
    assert_eq!(
        shell_exit(Some(not_found)),
        ShellExit {
            code: None,
            message: Some("executable file not found in $PATH".to_owned())
        }
    );
}

fn refused(code: http::StatusCode) -> ClusterError {
    exec_error(
        "prod",
        kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(code)),
    )
}

#[test]
fn upgrade_forbidden_maps_to_fixed_text() {
    match refused(http::StatusCode::FORBIDDEN) {
        ClusterError::Forbidden {
            context,
            action,
            message,
        } => {
            assert_eq!(context, "prod");
            assert_eq!(action, "opening a shell");
            assert_eq!(
                message,
                "the server refused the exec connection (HTTP 403); a shell needs get and \
                 create on pods/exec"
            );
        }
        other => panic!("expected Forbidden, got {other:?}"),
    }
}

#[test]
fn upgrade_unauthorized_and_not_found_map_to_typed_errors() {
    match refused(http::StatusCode::UNAUTHORIZED) {
        ClusterError::Unauthorized { message, .. } => {
            assert_eq!(message, "the server refused the exec connection (HTTP 401)");
        }
        other => panic!("expected Unauthorized, got {other:?}"),
    }
    match refused(http::StatusCode::NOT_FOUND) {
        ClusterError::Api { code, message, .. } => {
            assert_eq!(code, 404);
            assert_eq!(message, "pod or container not found");
        }
        other => panic!("expected Api 404, got {other:?}"),
    }
    match refused(http::StatusCode::BAD_GATEWAY) {
        ClusterError::Api { code, message, .. } => {
            assert_eq!(code, 502);
            assert_eq!(message, "the exec connection was refused (HTTP 502)");
        }
        other => panic!("expected Api 502, got {other:?}"),
    }
}

#[test]
fn shell_debug_prints_byte_counts_only() {
    assert_eq!(
        format!("{:?}", ShellInput::Bytes(b"secret".to_vec())),
        "Bytes(6 bytes)"
    );
    assert_eq!(
        format!("{:?}", ShellUpdate::Output(b"password: hunter2".to_vec())),
        "Output(17 bytes)"
    );
    assert_eq!(format!("{:?}", ShellUpdate::Started), "Started");
    assert_eq!(
        format!("{:?}", ShellInput::Resize(size(80, 24))),
        "Resize(GridSize { cols: 80, rows: 24 })"
    );
}

#[tokio::test]
async fn blocked_policy_sends_no_exec() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, "{}".to_owned()));
    let updates: Vec<_> = connection
        .pod_shell(
            ExecPermit::for_tests(),
            request(ShellCommand::Auto),
            stream::pending::<ShellInput>(),
        )
        .collect()
        .await;
    match updates.as_slice() {
        [ShellUpdate::Failed(ClusterError::Rendered { message })] => {
            assert_eq!(message, &WriteError::WritesBlocked.to_string());
        }
        other => panic!("expected one Failed, got {other:?}"),
    }
    assert!(api.requests().is_empty(), "no request left the client");
}

#[tokio::test]
async fn exec_opens_with_one_get_upgrade_and_maps_a_refusal() {
    let (connection, api) = FakeApi::connection(WritePolicy::Allowed, |_| (403, "{}".to_owned()));
    let updates: Vec<_> = connection
        .pod_shell(
            ExecPermit::for_tests(),
            request(ShellCommand::Bash),
            stream::pending::<ShellInput>(),
        )
        .collect()
        .await;
    assert!(
        matches!(
            updates.as_slice(),
            [ShellUpdate::Failed(ClusterError::Forbidden { .. })]
        ),
        "{updates:?}"
    );
    let requests = api.requests();
    let [exec] = requests.as_slice() else {
        panic!("expected one request, got {requests:?}");
    };
    assert_eq!(exec.method, "GET");
    assert_eq!(exec.path, "/api/v1/namespaces/team-a/pods/api-0/exec");
    for (key, value) in [
        ("container", "api"),
        ("tty", "true"),
        ("stdin", "true"),
        ("stdout", "true"),
        ("command", "bash"),
    ] {
        assert!(
            exec.has_query(key, value),
            "{key}={value} in {}",
            exec.query
        );
    }
    assert!(!exec.has_query_key("stderr"), "a TTY merges stderr");
}

#[test]
fn the_exit_message_is_sanitized() {
    let hostile = Status {
        status: Some("Failure".to_owned()),
        message: Some(format!("bad\x1b[2J\x07{}", "z".repeat(500))),
        ..Status::default()
    };
    let message = shell_exit(Some(hostile))
        .message
        .expect("a status without a code keeps its message");
    assert!(!message.contains('\x1b') && !message.contains('\x07'));
    assert_eq!(
        message.chars().count(),
        crate::reason_text::MAX_REASON_CHARS
    );
    assert!(message.starts_with("bad[2Jzz"));
}

#[tokio::test(start_paused = true)]
async fn a_status_that_never_comes_ends_the_session_after_the_request_timeout() {
    let (resize_sender, resize) = mpsc::channel(8);
    let mut harness = Harness::start_with(
        resize_sender,
        resize,
        std::future::pending::<Option<Status>>(),
    );
    harness
        .stdout
        .write_all(b"last words")
        .await
        .expect("pipe open");
    harness.stdout.shutdown().await.expect("pipe closes");
    assert_eq!(harness.next_output().await, b"last words");
    let started = tokio::time::Instant::now();
    // Longer than `WAIT`: the exit comes from the request timeout, not from the test's patience.
    let update = tokio::time::timeout(REQUEST_TIMEOUT * 2, harness.updates.next())
        .await
        .expect("the session ends")
        .expect("an exit is reported");
    assert!(matches!(
        update,
        ShellUpdate::Exited(ShellExit {
            code: None,
            message: None
        })
    ));
    assert_eq!(started.elapsed(), REQUEST_TIMEOUT);
    assert!(harness.updates.next().await.is_none(), "nothing follows");
}
