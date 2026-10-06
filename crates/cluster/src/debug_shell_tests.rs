use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::{StreamExt, stream};
use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};

fn size() -> GridSize {
    GridSize {
        cols: 100,
        rows: 30,
    }
}

fn request(wait: AttachWait) -> AttachRequest {
    AttachRequest {
        namespace: "kube-system".to_owned(),
        pod: "k8sboard-node-shell-wk-03-x7k2q".to_owned(),
        container: "shell".to_owned(),
        wait,
        size: size(),
    }
}

/// A pod whose container `shell` is in `state`, listed under `list` (`containerStatuses` or
/// `ephemeralContainerStatuses`).
fn pod_with(list: &str, state: Value) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "k8sboard-node-shell-wk-03-x7k2q", "namespace": "kube-system"},
        "status": {"phase": "Pending", list: [{
            "name": "shell", "image": "busybox", "imageID": "", "ready": false,
            "restartCount": 0, "state": state,
        }]},
    })
}

fn pod(state: Value) -> Pod {
    serde_json::from_value(pod_with("containerStatuses", state)).expect("a pod")
}

fn running() -> Value {
    json!({"running": {"startedAt": "2026-10-03T10:00:00Z"}})
}

fn waiting(reason: &str) -> Value {
    json!({"waiting": {"reason": reason, "message": "token=hunter2"}})
}

fn terminated(code: i32) -> Value {
    json!({"terminated": {"exitCode": code, "reason": "Error", "message": "token=hunter2"}})
}

#[test]
fn readiness_table() {
    let node = AttachWait::NodeShellPod;
    assert_eq!(
        readiness(Some(&pod(running())), "shell", node),
        Readiness::Running
    );
    assert_eq!(
        readiness(Some(&pod(waiting("ContainerCreating"))), "shell", node),
        Readiness::Waiting(Some("ContainerCreating".to_owned()))
    );
    for reason in FATAL_WAITING_REASONS {
        assert_eq!(
            readiness(Some(&pod(waiting(reason))), "shell", node),
            Readiness::Failed(format!("the container could not start: {reason}"))
        );
    }
    for reason in IMAGE_PULL_REASONS {
        assert_eq!(
            readiness(Some(&pod(waiting(reason))), "shell", node),
            Readiness::ImagePullFailed
        );
    }
    for code in NO_SHELL_EXIT_CODES {
        assert_eq!(
            readiness(Some(&pod(terminated(code))), "shell", node),
            Readiness::Failed("the node has no shell; node shell needs sh on the host".to_owned()),
            "exit {code}"
        );
    }
    assert_eq!(
        readiness(Some(&pod(terminated(1))), "shell", node),
        Readiness::Failed("the container ended (Error)".to_owned())
    );
    assert_eq!(
        readiness(None, "shell", node),
        Readiness::Failed("the pod no longer exists".to_owned())
    );
}

#[test]
fn a_finished_pod_fails_whatever_its_containers_say() {
    for phase in ["Failed", "Succeeded"] {
        let mut value = pod_with("containerStatuses", running());
        value["status"]["phase"] = json!(phase);
        let pod: Pod = serde_json::from_value(value).expect("a pod");
        assert_eq!(
            readiness(Some(&pod), "shell", AttachWait::NodeShellPod),
            Readiness::Failed("the pod has ended".to_owned())
        );
    }
}

#[test]
fn an_unreported_container_is_still_waiting() {
    let no_status: Pod = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"},
    }))
    .expect("a pod");
    assert_eq!(
        readiness(Some(&no_status), "shell", AttachWait::NodeShellPod),
        Readiness::Waiting(None)
    );
    // Another container's state is not ours.
    assert_eq!(
        readiness(Some(&pod(running())), "other", AttachWait::NodeShellPod),
        Readiness::Waiting(None)
    );
}

#[test]
fn ephemeral_containers_read_their_own_status_list() {
    let value = pod_with("ephemeralContainerStatuses", running());
    let pod: Pod = serde_json::from_value(value).expect("a pod");
    assert_eq!(
        readiness(Some(&pod), "shell", AttachWait::EphemeralContainer),
        Readiness::Running
    );
    // The same pod read as a node shell pod has no `containerStatuses` entry for it.
    assert_eq!(
        readiness(Some(&pod), "shell", AttachWait::NodeShellPod),
        Readiness::Waiting(None)
    );
    // Exit 127 is a node shell finding only; a debug container just ended.
    let mut ended = pod_with("ephemeralContainerStatuses", terminated(127));
    ended["status"]["phase"] = json!("Running");
    let ended: Pod = serde_json::from_value(ended).expect("a pod");
    assert_eq!(
        readiness(Some(&ended), "shell", AttachWait::EphemeralContainer),
        Readiness::Failed("the container ended (Error)".to_owned())
    );
}

#[test]
fn waiting_messages_are_dropped() {
    for state in [
        waiting("ImagePullBackOff"),
        waiting("ContainerCreating"),
        terminated(1),
    ] {
        let text = format!(
            "{:?}",
            readiness(Some(&pod(state)), "shell", AttachWait::NodeShellPod)
        );
        assert!(!text.contains("hunter2"), "{text}");
    }
}

#[test]
fn reasons_keep_letters_and_digits_only() {
    let hostile = "Pull\u{1b}[31m Back\noff\r<script>";
    assert_eq!(fixed_word(hostile), "Pull31mBackoffscript");
    assert_eq!(fixed_word(&"x".repeat(200)).len(), MAX_REASON_CHARS);
}

fn blocked_input() -> impl Stream<Item = ShellInput> + Send + Unpin + 'static {
    stream::pending::<ShellInput>()
}

#[tokio::test]
async fn blocked_policy_fails_attach_before_any_request() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, "{}".to_owned()));
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::NodeShellPod),
            blocked_input(),
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

/// A fake cluster whose pod GET answers `states[n]` on the nth read (the last one repeats) and
/// whose attach is refused with `refusal`.
fn scripted(states: Vec<Value>, refusal: u16) -> (ClusterConnection, FakeApi, Arc<AtomicUsize>) {
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&reads);
    let (connection, api) = FakeApi::connection(WritePolicy::Allowed, move |request| {
        if request.path.ends_with("/attach") {
            return (refusal, "{}".to_owned());
        }
        let index = counter.fetch_add(1, Ordering::SeqCst).min(states.len() - 1);
        (
            200,
            pod_with("containerStatuses", states[index].clone()).to_string(),
        )
    });
    (connection, api, reads)
}

fn gets(requests: &[RecordedRequest]) -> Vec<&RecordedRequest> {
    requests
        .iter()
        .filter(|request| !request.path.ends_with("/attach"))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn wait_polls_get_every_second() {
    let (connection, api, _) = scripted(
        vec![
            waiting("ContainerCreating"),
            waiting("ContainerCreating"),
            waiting("Pulling"),
            running(),
        ],
        403,
    );
    let started = tokio::time::Instant::now();
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::NodeShellPod),
            blocked_input(),
        )
        .collect()
        .await;
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(3),
        "one read a second"
    );
    // The reason is reported when it changes, not on every read.
    match updates.as_slice() {
        [
            ShellUpdate::Waiting(first),
            ShellUpdate::Waiting(second),
            ShellUpdate::Failed(ClusterError::Forbidden { message, .. }),
        ] => {
            assert_eq!(first, "ContainerCreating");
            assert_eq!(second, "Pulling");
            assert!(message.contains("pods/attach"), "{message}");
        }
        other => panic!("unexpected updates: {other:?}"),
    }
    let requests = api.requests();
    let reads = gets(&requests);
    assert_eq!(reads.len(), 4);
    for read in reads {
        assert_eq!(read.method, "GET");
        assert_eq!(
            read.path,
            "/api/v1/namespaces/kube-system/pods/k8sboard-node-shell-wk-03-x7k2q"
        );
        assert!(!read.has_query_key("watch"), "no watcher: {}", read.query);
    }
}

#[tokio::test(start_paused = true)]
async fn attach_upgrade_403_names_pods_attach() {
    let (connection, api, _) = scripted(vec![running()], 403);
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::NodeShellPod),
            blocked_input(),
        )
        .collect()
        .await;
    match updates.as_slice() {
        [
            ShellUpdate::Failed(ClusterError::Forbidden {
                action, message, ..
            }),
        ] => {
            assert_eq!(*action, "attaching a debug shell");
            assert_eq!(
                message,
                "the server refused the attach connection (HTTP 403); a shell needs get and \
                 create on pods/attach"
            );
        }
        other => panic!("unexpected updates: {other:?}"),
    }
    let requests = api.requests();
    let attach = requests
        .iter()
        .find(|request| request.path.ends_with("/attach"))
        .expect("the attach request");
    assert_eq!(attach.method, "GET");
    for (key, value) in [
        ("container", "shell"),
        ("tty", "true"),
        ("stdin", "true"),
        ("stdout", "true"),
    ] {
        assert!(
            attach.has_query(key, value),
            "{key}={value} in {}",
            attach.query
        );
    }
    assert!(!attach.has_query_key("stderr"), "a TTY merges stderr");
}

#[tokio::test(start_paused = true)]
async fn wait_times_out_after_120_s_with_the_last_reason() {
    let (connection, api, reads) = scripted(vec![waiting("ContainerCreating")], 403);
    let started = tokio::time::Instant::now();
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::NodeShellPod),
            blocked_input(),
        )
        .collect()
        .await;
    assert_eq!(started.elapsed(), Duration::from_secs(120));
    match updates.as_slice() {
        [
            ShellUpdate::Waiting(_),
            ShellUpdate::Failed(ClusterError::Rendered { message }),
        ] => assert_eq!(
            message,
            "container did not start within 120 s (ContainerCreating)"
        ),
        other => panic!("unexpected updates: {other:?}"),
    }
    assert_eq!(reads.load(Ordering::SeqCst), 121);
    assert!(
        api.requests()
            .iter()
            .all(|request| !request.path.ends_with("/attach")),
        "a container that never ran is never attached"
    );
}

#[tokio::test(start_paused = true)]
async fn a_fatal_waiting_reason_fails_at_once() {
    let (connection, api, _) = scripted(vec![waiting("CreateContainerConfigError")], 403);
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::NodeShellPod),
            blocked_input(),
        )
        .collect()
        .await;
    match updates.as_slice() {
        [ShellUpdate::Failed(ClusterError::Rendered { message })] => {
            assert_eq!(
                message,
                "the container could not start: CreateContainerConfigError"
            );
        }
        other => panic!("unexpected updates: {other:?}"),
    }
    assert_eq!(api.requests().len(), 1);
}

/// A fake cluster whose pod waits with `ErrImagePull` and whose events list answers `events`.
fn pull_failing(events: Value) -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, move |request| {
        if request.path.ends_with("/events") {
            return (200, events.to_string());
        }
        (
            200,
            pod_with("containerStatuses", waiting("ErrImagePull")).to_string(),
        )
    })
}

fn pull_event(message: &str, last: &str) -> Value {
    json!({
        "metadata": {"name": format!("e-{last}"), "namespace": "kube-system"},
        "reason": "Failed", "type": "Warning", "message": message, "lastTimestamp": last,
    })
}

#[tokio::test(start_paused = true)]
async fn a_pull_failure_reports_the_newest_kubelet_cause_on_one_masked_line() {
    let old = pull_event(
        "Failed to pull image \"x\": old cause",
        "2026-10-06T07:00:00Z",
    );
    let new = pull_event(
        "Failed to pull image \"x\": rpc error: pull access denied at https://user:pw@reg.io/v2\nsecond line",
        "2026-10-06T07:01:00Z",
    );
    let (connection, api) = pull_failing(json!({"apiVersion": "v1", "kind": "EventList",
        "metadata": {}, "items": [new, old]}));
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::NodeShellPod),
            blocked_input(),
        )
        .collect()
        .await;
    match updates.as_slice() {
        [ShellUpdate::ImagePullFailed { detail }] => {
            let detail = detail.as_deref().expect("a cause");
            assert!(
                detail.starts_with("rpc error: pull access denied"),
                "{detail}"
            );
            assert!(!detail.contains("pw"), "{detail}");
            assert!(!detail.contains("second line"), "{detail}");
        }
        other => panic!("unexpected updates: {other:?}"),
    }
    let events = api
        .requests()
        .into_iter()
        .find(|request| request.path.ends_with("/events"))
        .expect("the events were read");
    assert_eq!(events.method, "GET");
    assert!(events.has_query_key("fieldSelector"), "{}", events.query);
}

#[tokio::test(start_paused = true)]
async fn a_pull_failure_without_readable_events_still_reports_the_failure() {
    let (connection, _api) = pull_failing(json!({"kind": "Status", "code": 403}));
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::NodeShellPod),
            blocked_input(),
        )
        .collect()
        .await;
    assert!(
        matches!(
            updates.as_slice(),
            [ShellUpdate::ImagePullFailed { detail: None }]
        ),
        "{updates:?}"
    );
}

#[test]
fn a_long_pull_cause_is_cut() {
    let message = format!("Failed to pull image \"x\": {}", "e".repeat(500));
    let cause = pull_cause(&message).expect("a cause");
    assert_eq!(cause.chars().count(), MAX_CAUSE_CHARS + 1);
    assert!(cause.ends_with('…'));
}

#[tokio::test(start_paused = true)]
async fn a_gone_pod_fails_the_wait() {
    let (connection, _api) = FakeApi::connection(WritePolicy::Allowed, |_| {
        (
            404,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
                "reason": "NotFound", "code": 404})
            .to_string(),
        )
    });
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::NodeShellPod),
            blocked_input(),
        )
        .collect()
        .await;
    match updates.as_slice() {
        [ShellUpdate::Failed(ClusterError::Rendered { message })] => {
            assert_eq!(message, "the pod no longer exists");
        }
        other => panic!("unexpected updates: {other:?}"),
    }
}

#[test]
fn waiting_update_prints_its_reason_only() {
    assert_eq!(
        format!("{:?}", ShellUpdate::Waiting("Pulling".to_owned())),
        "Waiting(\"Pulling\")"
    );
}

#[tokio::test]
async fn blocked_policy_sends_no_attach_or_get() {
    // Both kinds of wait: the kill switch is checked before the first read of the pod.
    for wait in [AttachWait::NodeShellPod, AttachWait::EphemeralContainer] {
        let (connection, api) =
            FakeApi::connection(WritePolicy::Blocked, |_| (200, "{}".to_owned()));
        let updates: Vec<_> = connection
            .attach_shell(AttachPermit::for_tests(), request(wait), blocked_input())
            .collect()
            .await;
        assert!(matches!(updates.as_slice(), [ShellUpdate::Failed(_)]));
        assert!(api.requests().is_empty(), "no GET and no attach");
    }
}

#[test]
fn attach_refusals_map_to_typed_errors_that_name_the_verb() {
    let refused = |code: u16| {
        connect_error(
            "prod",
            ATTACH,
            kube::Error::UpgradeConnection(kube::client::UpgradeConnectionError::ProtocolSwitch(
                http::StatusCode::from_u16(code).expect("a status"),
            )),
        )
    };
    match refused(401) {
        ClusterError::Unauthorized {
            action, message, ..
        } => {
            assert_eq!(action, "attaching a debug shell");
            assert_eq!(
                message,
                "the server refused the attach connection (HTTP 401)"
            );
        }
        other => panic!("expected Unauthorized, got {other:?}"),
    }
    match refused(404) {
        ClusterError::Api { code, message, .. } => {
            assert_eq!(code, 404);
            assert_eq!(message, "pod or container not found");
        }
        other => panic!("expected Api 404, got {other:?}"),
    }
    match refused(502) {
        ClusterError::Api { message, .. } => {
            assert_eq!(message, "the attach connection was refused (HTTP 502)");
        }
        other => panic!("expected Api 502, got {other:?}"),
    }
}

/// A pod whose container `name` runs, listed under `list`.
fn running_in(list: &str, name: &str) -> Pod {
    let value = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "api-0", "namespace": "kube-system"},
        "status": {"phase": "Running", list: [{
            "name": name, "image": "busybox", "imageID": "", "ready": true,
            "restartCount": 0, "state": running(),
        }]},
    });
    serde_json::from_value(value).expect("a pod")
}

#[test]
fn container_wait_reads_main_and_sidecar_statuses() {
    let main = running_in("containerStatuses", "app");
    assert_eq!(
        readiness(Some(&main), "app", AttachWait::Container),
        Readiness::Running
    );
    let sidecar = running_in("initContainerStatuses", "proxy");
    assert_eq!(
        readiness(Some(&sidecar), "proxy", AttachWait::Container),
        Readiness::Running
    );
    // A node shell wait does not look in the init list.
    assert_eq!(
        readiness(Some(&sidecar), "proxy", AttachWait::NodeShellPod),
        Readiness::Waiting(None)
    );
}

#[test]
fn container_wait_keeps_exit_127_as_an_ended_container() {
    let mut ended = pod_with("containerStatuses", terminated(127));
    ended["status"]["phase"] = json!("Running");
    let ended: Pod = serde_json::from_value(ended).expect("a pod");
    assert_eq!(
        readiness(Some(&ended), "shell", AttachWait::Container),
        Readiness::Failed("the container ended (Error)".to_owned())
    );
}

#[tokio::test(start_paused = true)]
async fn container_attach_requests_the_attach_path_after_one_read() {
    let (connection, api, reads) = scripted(vec![running()], 403);
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::Container),
            blocked_input(),
        )
        .collect()
        .await;
    match updates.as_slice() {
        [
            ShellUpdate::Failed(ClusterError::Forbidden {
                action, message, ..
            }),
        ] => {
            assert_eq!(*action, "attaching to a container");
            assert!(message.contains("pods/attach"), "{message}");
        }
        other => panic!("unexpected updates: {other:?}"),
    }
    assert_eq!(reads.load(Ordering::SeqCst), 1, "one read, then the attach");
    let requests = api.requests();
    let attach = requests
        .iter()
        .find(|request| request.path.ends_with("/attach"))
        .expect("the attach request");
    assert_eq!(attach.method, "GET");
    assert_eq!(
        attach.path,
        "/api/v1/namespaces/kube-system/pods/k8sboard-node-shell-wk-03-x7k2q/attach"
    );
    for (key, value) in [
        ("container", "shell"),
        ("tty", "true"),
        ("stdin", "true"),
        ("stdout", "true"),
    ] {
        assert!(
            attach.has_query(key, value),
            "{key}={value} in {}",
            attach.query
        );
    }
}

#[tokio::test]
async fn the_debug_policy_blocks_an_attach() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, "{}".to_owned()));
    let updates: Vec<_> = connection
        .attach_shell(
            AttachPermit::for_tests(),
            request(AttachWait::Container),
            blocked_input(),
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
