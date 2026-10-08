//! Tests of the 0032 workload operations: wire format, reads before the send, effects, errors.

use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};
use crate::workload_write_bodies::JOB_CONTROLLER_LABELS;

const NS: &str = "payments";
const DEPLOYMENT_PATH: &str = "/apis/apps/v1/namespaces/payments/deployments/api";
const REPLICA_SET_PATH: &str = "/apis/apps/v1/namespaces/payments/replicasets/api-6c1e2a";
const CRON_JOB_PATH: &str = "/apis/batch/v1/namespaces/payments/cronjobs/nightly";
const JOB_PATH: &str = "/apis/batch/v1/namespaces/payments/jobs/etl-1";
const JOBS_PATH: &str = "/apis/batch/v1/namespaces/payments/jobs";
const MERGE_PATCH: &str = "application/merge-patch+json";
/// A value that must never reach an error, a trace, or an audit field.
const ENV_LITERAL: &str = "s3cr3t-env";

fn object(kind: ObjectKind, name: &str) -> ObjectRef {
    ObjectRef::new(kind, Some(NS.to_owned()), name.to_owned()).expect("a namespaced kind")
}

fn request(kind: ObjectKind, name: &str, operation: WriteOperation) -> WriteRequest {
    WriteRequest::new(object(kind, name), operation).expect("the kind fits the operation")
}

fn roll_back() -> WriteRequest {
    request(
        ObjectKind::Deployment,
        "api",
        WriteOperation::RollBackDeployment {
            replica_set: "api-6c1e2a".to_owned(),
            revision: 37,
        },
    )
}

fn trigger() -> WriteRequest {
    request(
        ObjectKind::CronJob,
        "nightly",
        WriteOperation::TriggerCronJob,
    )
}

fn rerun() -> WriteRequest {
    request(ObjectKind::Job, "etl-1", WriteOperation::RerunJob)
}

fn status_json(code: u16, message: &str, fields: &[&str]) -> String {
    status_with_reason(code, message, fields, "Invalid")
}

fn status_with_reason(code: u16, message: &str, fields: &[&str], reason: &str) -> String {
    let causes: Vec<Value> = fields
        .iter()
        .map(|field| json!({ "reason": "FieldValueInvalid", "message": message, "field": field }))
        .collect();
    json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure", "message": message,
        "reason": reason, "details": { "causes": causes }, "code": code,
    })
    .to_string()
}

/// The answer to a write: a `Scale`, a created Job, or a bare object.
fn ok_reply(request: &RecordedRequest) -> (u16, String) {
    if request.path.ends_with("/scale") {
        let scale = json!({
            "apiVersion": "autoscaling/v1", "kind": "Scale",
            "metadata": {"name": "api"}, "spec": {"replicas": 5},
        });
        return (200, scale.to_string());
    }
    let name = if request.method == "POST" {
        "nightly-manual-x7k2p"
    } else {
        "api"
    };
    let object = json!({"apiVersion": "v1", "kind": "Thing", "metadata": {"name": name}});
    (
        if request.method == "POST" { 201 } else { 200 },
        object.to_string(),
    )
}

/// A fake cluster: GET serves `objects` (404 otherwise), every write is answered by `reply`.
fn cluster(
    objects: Vec<(&'static str, Value)>,
    reply: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static,
) -> (ClusterConnection, FakeApi) {
    let objects: Vec<(&str, String)> = objects
        .into_iter()
        .map(|(path, value)| (path, value.to_string()))
        .collect();
    FakeApi::connection(WritePolicy::Allowed, move |request| {
        if request.method != "GET" {
            return reply(request);
        }
        match objects.iter().find(|(path, _)| *path == request.path) {
            Some((_, body)) => (200, body.clone()),
            None => (404, status_json(404, "not found", &[])),
        }
    })
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

fn deployment_json(image: &str) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "api", "namespace": NS, "uid": "dep-uid", "resourceVersion": "100"},
        "spec": {"replicas": 3, "template": {
            "metadata": {"labels": {"app": "api", "pod-template-hash": "current-hash"}},
            "spec": {"containers": [{"name": "api", "image": image,
                "env": [{"name": "TOKEN", "value": ENV_LITERAL}]}]},
        }},
    })
}

fn replica_set_json(owner_uid: &str, revision: &str, image: &str) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {
            "name": "api-6c1e2a", "namespace": NS,
            "annotations": {"deployment.kubernetes.io/revision": revision},
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment", "name": "api",
                "uid": owner_uid, "controller": true}],
        },
        "spec": {"template": {
            "metadata": {"labels": {"app": "api", "pod-template-hash": "6c1e2a"}},
            "spec": {"containers": [{"name": "api", "image": image,
                "env": [{"name": "TOKEN", "value": ENV_LITERAL}]}]},
        }},
    })
}

fn rollback_cluster(owner_uid: &str, revision: &str, image: &str) -> (ClusterConnection, FakeApi) {
    cluster(
        vec![
            (DEPLOYMENT_PATH, deployment_json("api:2")),
            (
                REPLICA_SET_PATH,
                replica_set_json(owner_uid, revision, image),
            ),
        ],
        ok_reply,
    )
}

fn cron_job_json() -> Value {
    json!({
        "apiVersion": "batch/v1", "kind": "CronJob",
        "metadata": {"name": "nightly", "namespace": NS, "uid": "cron-uid"},
        "spec": {"schedule": "0 3 * * *", "jobTemplate": {
            "metadata": {"labels": {"app": "nightly"}, "annotations": {"note": "keep"}},
            "spec": {"template": {"spec": {"containers": [{"name": "job", "image": "job:1",
                "env": [{"name": "TOKEN", "value": ENV_LITERAL}]}], "restartPolicy": "Never"}}},
        }},
    })
}

fn job_json() -> Value {
    let controller_labels = json!({
        "controller-uid": "c", "job-name": "etl-1",
        "batch.kubernetes.io/controller-uid": "c", "batch.kubernetes.io/job-name": "etl-1",
    });
    let mut labels = controller_labels.clone();
    labels["app"] = json!("etl");
    json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {
            "name": "etl-1", "namespace": NS, "uid": "job-uid", "labels": labels,
            "annotations": {"team": "data"},
            "ownerReferences": [{"apiVersion": "batch/v1", "kind": "CronJob", "name": "etl", "uid": "x"}],
        },
        "spec": {
            "selector": {"matchLabels": {"controller-uid": "c"}},
            "manualSelector": false,
            "suspend": true,
            "completions": 4,
            "template": {
                "metadata": {"labels": labels},
                "spec": {"containers": [{"name": "etl", "image": "etl:1",
                    "env": [{"name": "TOKEN", "value": ENV_LITERAL}]}], "restartPolicy": "Never"},
            },
        },
    })
}

fn restarted_at() -> jiff::Timestamp {
    "2026-10-02T09:12:03.456789Z".parse().expect("a timestamp")
}

async fn written(request: &WriteRequest, mode: WriteMode) -> Vec<RecordedRequest> {
    let (connection, api) = cluster(Vec::new(), ok_reply);
    connection
        .write(request, mode)
        .await
        .expect("the write is accepted");
    api.requests()
}

#[tokio::test]
async fn allow_list_matches_the_operations() {
    let restart = |kind, name| {
        request(
            kind,
            name,
            WriteOperation::RestartRollout {
                restarted_at: restarted_at(),
            },
        )
    };
    let stamp = json!({"spec": {"template": {"metadata": {"annotations": {
        "kubectl.kubernetes.io/restartedAt": "2026-10-02T09:12:03Z"}}}}});
    let cases = [
        (
            request(
                ObjectKind::Deployment,
                "api",
                WriteOperation::ScaleWorkload {
                    replicas: 5,
                    previous: 2,
                },
            ),
            "/apis/apps/v1/namespaces/payments/deployments/api/scale",
            json!({"spec": {"replicas": 5}}),
        ),
        (
            request(
                ObjectKind::StatefulSet,
                "kafka",
                WriteOperation::ScaleWorkload {
                    replicas: 0,
                    previous: 2,
                },
            ),
            "/apis/apps/v1/namespaces/payments/statefulsets/kafka/scale",
            json!({"spec": {"replicas": 0}}),
        ),
        (
            restart(ObjectKind::Deployment, "api"),
            "/apis/apps/v1/namespaces/payments/deployments/api",
            stamp.clone(),
        ),
        (
            restart(ObjectKind::StatefulSet, "kafka"),
            "/apis/apps/v1/namespaces/payments/statefulsets/kafka",
            stamp.clone(),
        ),
        (
            restart(ObjectKind::DaemonSet, "agent"),
            "/apis/apps/v1/namespaces/payments/daemonsets/agent",
            stamp,
        ),
        (
            request(
                ObjectKind::Deployment,
                "api",
                WriteOperation::SetRolloutPaused { paused: true },
            ),
            DEPLOYMENT_PATH,
            json!({"spec": {"paused": true}}),
        ),
        (
            request(
                ObjectKind::CronJob,
                "nightly",
                WriteOperation::SetCronJobSuspended { suspended: false },
            ),
            CRON_JOB_PATH,
            json!({"spec": {"suspend": false}}),
        ),
    ];
    for (request, path, body) in cases {
        for (mode, query) in [
            (WriteMode::Commit, vec!["fieldManager=k8sboard"]),
            (
                WriteMode::DryRun,
                vec!["dryRun=All", "fieldManager=k8sboard"],
            ),
        ] {
            let requests = written(&request, mode).await;
            assert_eq!(requests.len(), 1, "{request:?}");
            assert_eq!(requests[0].method, "PATCH", "{request:?}");
            assert_eq!(requests[0].path, path, "{request:?}");
            assert_eq!(query_pairs(&requests[0]), query, "{request:?}");
            assert_eq!(
                requests[0].content_type.as_deref(),
                Some(MERGE_PATCH),
                "{request:?}"
            );
            assert_eq!(body_of(&requests[0]), body, "{request:?}");
        }
    }
}

#[tokio::test]
async fn scale_patches_the_scale_subresource() {
    let requests = written(
        &request(
            ObjectKind::StatefulSet,
            "kafka",
            WriteOperation::ScaleWorkload {
                replicas: 5,
                previous: 2,
            },
        ),
        WriteMode::Commit,
    )
    .await;
    assert_eq!(
        requests[0].path,
        "/apis/apps/v1/namespaces/payments/statefulsets/kafka/scale"
    );
    assert_eq!(body_of(&requests[0]), json!({"spec": {"replicas": 5}}));
}

#[test]
fn scale_rejects_other_kinds() {
    for kind in [
        ObjectKind::DaemonSet,
        ObjectKind::ReplicaSet,
        ObjectKind::Job,
    ] {
        let operation = WriteOperation::ScaleWorkload {
            replicas: 2,
            previous: 2,
        };
        assert!(
            WriteRequest::new(object(kind, "x"), operation).is_none(),
            "{kind:?}"
        );
    }
}

#[test]
fn operations_reject_kinds_they_do_not_fit() {
    let fits =
        |kind, operation: WriteOperation| WriteRequest::new(object(kind, "x"), operation).is_some();
    let restart = || WriteOperation::RestartRollout {
        restarted_at: restarted_at(),
    };
    assert!(fits(ObjectKind::DaemonSet, restart()));
    assert!(!fits(ObjectKind::CronJob, restart()));
    assert!(!fits(
        ObjectKind::StatefulSet,
        WriteOperation::SetRolloutPaused { paused: true }
    ));
    let roll_back = |kind| {
        fits(
            kind,
            WriteOperation::RollBackDeployment {
                replica_set: "rs".to_owned(),
                revision: 1,
            },
        )
    };
    assert!(roll_back(ObjectKind::Deployment));
    assert!(!roll_back(ObjectKind::StatefulSet));
    assert!(!fits(
        ObjectKind::Deployment,
        WriteOperation::SetCronJobSuspended { suspended: true }
    ));
    assert!(!fits(ObjectKind::Job, WriteOperation::TriggerCronJob));
    assert!(!fits(ObjectKind::CronJob, WriteOperation::RerunJob));
    assert!(fits(ObjectKind::CronJob, WriteOperation::TriggerCronJob));
    assert!(fits(ObjectKind::Job, WriteOperation::RerunJob));
}

#[test]
fn a_replica_set_name_that_changes_the_path_is_refused() {
    for name in ["a/b", "..", "A", "a?b", ""] {
        let operation = WriteOperation::RollBackDeployment {
            replica_set: name.to_owned(),
            revision: 1,
        };
        assert!(
            WriteRequest::new(object(ObjectKind::Deployment, "api"), operation).is_none(),
            "{name:?}"
        );
    }
}

#[tokio::test]
async fn restart_uses_the_kubectl_annotation() {
    let requests = written(
        &request(
            ObjectKind::Deployment,
            "api",
            WriteOperation::RestartRollout {
                restarted_at: restarted_at(),
            },
        ),
        WriteMode::Commit,
    )
    .await;
    let body = body_of(&requests[0]);
    let annotations = &body["spec"]["template"]["metadata"]["annotations"];
    assert_eq!(
        annotations["kubectl.kubernetes.io/restartedAt"],
        "2026-10-02T09:12:03Z"
    );
    assert!(!requests[0].body.contains("kube.kubernetes.io"));
    assert!(!requests[0].body.contains(".456"));
}

#[tokio::test]
async fn restart_dry_run_and_commit_send_the_same_body() {
    let restart = request(
        ObjectKind::Deployment,
        "api",
        WriteOperation::RestartRollout {
            restarted_at: restarted_at(),
        },
    );
    let dry_run = written(&restart, WriteMode::DryRun).await;
    let commit = written(&restart, WriteMode::Commit).await;
    assert_eq!(dry_run[0].body, commit[0].body);
}

#[tokio::test]
async fn pause_and_resume_send_booleans() {
    for paused in [true, false] {
        let requests = written(
            &request(
                ObjectKind::Deployment,
                "api",
                WriteOperation::SetRolloutPaused { paused },
            ),
            WriteMode::Commit,
        )
        .await;
        assert_eq!(body_of(&requests[0]), json!({"spec": {"paused": paused}}));
        assert!(!requests[0].body.contains("null"));
    }
}

#[tokio::test]
async fn suspend_and_resume_send_booleans() {
    for suspended in [true, false] {
        let requests = written(
            &request(
                ObjectKind::CronJob,
                "nightly",
                WriteOperation::SetCronJobSuspended { suspended },
            ),
            WriteMode::Commit,
        )
        .await;
        assert_eq!(
            body_of(&requests[0]),
            json!({"spec": {"suspend": suspended}})
        );
        assert!(!requests[0].body.contains("null"));
    }
}

#[tokio::test]
async fn patches_report_the_patched_effect() {
    let (connection, _api) = cluster(Vec::new(), ok_reply);
    let outcome = connection
        .write(
            &request(
                ObjectKind::Deployment,
                "api",
                WriteOperation::ScaleWorkload {
                    replicas: 5,
                    previous: 2,
                },
            ),
            WriteMode::Commit,
        )
        .await
        .expect("the patch is accepted");
    assert_eq!(outcome.effect, WriteEffect::Patched);
    assert_eq!(outcome.created_name, None);
}

#[tokio::test]
async fn rollback_reads_then_sends_a_json_patch() {
    let (connection, api) = rollback_cluster("dep-uid", "37", "api:1");
    let outcome = connection
        .write(&roll_back(), WriteMode::Commit)
        .await
        .expect("the roll back is accepted");
    assert_eq!(outcome.effect, WriteEffect::Patched);
    let requests = api.requests();
    let methods: Vec<&str> = requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    assert_eq!(methods, ["GET", "GET", "PATCH"]);
    assert_eq!(requests[0].path, DEPLOYMENT_PATH);
    assert_eq!(requests[1].path, REPLICA_SET_PATH);
    let patch = &requests[2];
    assert_eq!(patch.path, DEPLOYMENT_PATH);
    assert_eq!(query_pairs(patch), ["fieldManager=k8sboard"]);
    assert_eq!(
        patch.content_type.as_deref(),
        Some("application/json-patch+json")
    );
    let template = json!({
        "metadata": {"labels": {"app": "api"}},
        "spec": {"containers": [{"name": "api", "image": "api:1",
            "env": [{"name": "TOKEN", "value": ENV_LITERAL}]}]},
    });
    assert_eq!(
        body_of(patch),
        json!([
            {"op": "test", "path": "/metadata/uid", "value": "dep-uid"},
            {"op": "replace", "path": "/spec/template", "value": template},
        ])
    );
    assert!(!patch.body.contains("pod-template-hash"));
}

async fn refused_roll_back(
    owner_uid: &str,
    revision: &str,
    image: &str,
) -> (WriteError, Vec<RecordedRequest>) {
    let (connection, api) = rollback_cluster(owner_uid, revision, image);
    let error = connection
        .write(&roll_back(), WriteMode::Commit)
        .await
        .expect_err("the roll back is refused");
    (error, api.requests())
}

fn assert_nothing_was_sent(requests: &[RecordedRequest]) {
    assert!(
        requests.iter().all(|request| request.method == "GET"),
        "{requests:?}"
    );
}

#[tokio::test]
async fn rollback_refuses_a_foreign_replica_set() {
    let (error, requests) = refused_roll_back("another-uid", "37", "api:1").await;
    assert!(matches!(error, WriteError::NotFound), "{error:?}");
    assert_nothing_was_sent(&requests);
}

#[tokio::test]
async fn rollback_refuses_a_changed_revision() {
    let (error, requests) = refused_roll_back("dep-uid", "38", "api:1").await;
    assert!(matches!(error, WriteError::NotFound), "{error:?}");
    assert_nothing_was_sent(&requests);
}

#[tokio::test]
async fn rollback_refuses_the_current_template() {
    let (error, requests) = refused_roll_back("dep-uid", "37", "api:2").await;
    assert!(
        matches!(&error, WriteError::Invalid { message, fields }
            if message == "revision 37 has the same template as the current one" && fields.is_empty()),
        "{error:?}"
    );
    assert_nothing_was_sent(&requests);
}

#[tokio::test]
async fn rollback_uid_test_failure_is_conflict() {
    let message = format!("the server said {ENV_LITERAL} was not equal");
    let reply_body = status_with_reason(422, &message, &["spec.template"], "Conflict");
    let (connection, _api) = cluster(
        vec![
            (DEPLOYMENT_PATH, deployment_json("api:2")),
            (REPLICA_SET_PATH, replica_set_json("dep-uid", "37", "api:1")),
        ],
        move |_| (422, reply_body.clone()),
    );
    let error = connection
        .write(&roll_back(), WriteMode::Commit)
        .await
        .expect_err("the test op failed");
    assert!(
        matches!(&error, WriteError::Conflict { message, managers }
            if message == "the deployment was replaced since it was read" && managers.is_empty()),
        "{error:?}"
    );
    assert!(!format!("{error} {error:?}").contains(ENV_LITERAL));
}

#[tokio::test]
async fn trigger_builds_a_job_from_the_template() {
    let (connection, api) = cluster(vec![(CRON_JOB_PATH, cron_job_json())], ok_reply);
    connection
        .write(&trigger(), WriteMode::Commit)
        .await
        .expect("the job is created");
    let requests = api.requests();
    let methods: Vec<&str> = requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    assert_eq!(methods, ["GET", "POST"]);
    let post = &requests[1];
    assert_eq!(post.path, JOBS_PATH);
    assert_eq!(query_pairs(post), ["fieldManager=k8sboard"]);
    assert_eq!(post.content_type.as_deref(), Some("application/json"));
    let body = body_of(post);
    assert_eq!(body["apiVersion"], "batch/v1");
    assert_eq!(body["kind"], "Job");
    assert_eq!(body["metadata"]["generateName"], "nightly-manual-");
    assert_eq!(body["metadata"]["namespace"], NS);
    assert_eq!(body["metadata"]["labels"], json!({"app": "nightly"}));
    assert_eq!(
        body["metadata"]["annotations"],
        json!({"note": "keep", "cronjob.kubernetes.io/instantiate": "manual"})
    );
    assert_eq!(
        body["metadata"]["ownerReferences"],
        json!([{"apiVersion": "batch/v1", "kind": "CronJob", "name": "nightly",
            "uid": "cron-uid", "controller": true}])
    );
    assert!(!post.body.contains("blockOwnerDeletion"));
    assert_eq!(body["spec"], cron_job_json()["spec"]["jobTemplate"]["spec"]);
}

#[test]
fn trigger_name_base_fits_the_job_limit() {
    let long = "c".repeat(60);
    let name = generate_name(&long, TRIGGER_BASE_CHARS, TRIGGER_SUFFIX);
    assert_eq!(name, format!("{}-manual-", "c".repeat(50)));
    // The server appends 5 characters; a Job name is at most 63.
    assert_eq!(name.len() + 5, 63);
    let rerun = generate_name(&"j".repeat(60), RERUN_BASE_CHARS, RERUN_SUFFIX);
    assert_eq!(rerun.len() + 5, 63);
    assert_eq!(
        generate_name("short", TRIGGER_BASE_CHARS, TRIGGER_SUFFIX),
        "short-manual-"
    );
}

#[tokio::test]
async fn creates_report_the_created_name() {
    let (connection, _api) = cluster(vec![(CRON_JOB_PATH, cron_job_json())], ok_reply);
    let committed = connection
        .write(&trigger(), WriteMode::Commit)
        .await
        .expect("the job is created");
    assert_eq!(committed.effect, WriteEffect::Created);
    assert_eq!(
        committed.created_name.as_deref(),
        Some("nightly-manual-x7k2p")
    );
    let dry_run = connection
        .write(&trigger(), WriteMode::DryRun)
        .await
        .expect("the dry-run is accepted");
    assert_eq!(dry_run.effect, WriteEffect::Created);
    assert_eq!(dry_run.created_name, None);
}

#[tokio::test]
async fn rerun_strips_controller_fields() {
    let (connection, api) = cluster(vec![(JOB_PATH, job_json())], ok_reply);
    connection
        .write(&rerun(), WriteMode::DryRun)
        .await
        .expect("the dry-run is accepted");
    let requests = api.requests();
    let post = &requests[1];
    assert_eq!(post.method, "POST");
    assert_eq!(post.path, JOBS_PATH);
    assert_eq!(query_pairs(post), ["dryRun=All", "fieldManager=k8sboard"]);
    let body = body_of(post);
    assert_eq!(body["metadata"]["generateName"], "etl-1-rerun-");
    assert_eq!(body["metadata"]["labels"], json!({"app": "etl"}));
    assert!(body["metadata"].get("ownerReferences").is_none());
    assert!(body["metadata"].get("annotations").is_none());
    assert!(body["spec"].get("selector").is_none());
    assert!(body["spec"].get("manualSelector").is_none());
    assert_eq!(body["spec"]["suspend"], false);
    assert_eq!(body["spec"]["completions"], 4);
    assert_eq!(
        body["spec"]["template"]["metadata"]["labels"],
        json!({"app": "etl"})
    );
    for label in JOB_CONTROLLER_LABELS {
        assert!(!post.body.contains(label), "{label}");
    }
}

#[tokio::test]
async fn create_422_keeps_field_paths_only() {
    let message = format!("env TOKEN {ENV_LITERAL} is invalid");
    let reply_body = status_json(422, &message, &["spec.template.spec.containers[0].image"]);
    for (request, path, object) in [
        (trigger(), CRON_JOB_PATH, cron_job_json()),
        (rerun(), JOB_PATH, job_json()),
    ] {
        let reply_body = reply_body.clone();
        let (connection, _api) = cluster(vec![(path, object)], move |_| (422, reply_body.clone()));
        let error = connection
            .write(&request, WriteMode::Commit)
            .await
            .expect_err("the server rejected the job");
        assert!(
            matches!(&error, WriteError::Invalid { message, fields }
                if message == "the server rejected the generated object"
                    && fields == &["spec.template.spec.containers[0].image".to_owned()]),
            "{error:?}"
        );
        assert!(!format!("{error} {error:?}").contains(ENV_LITERAL));
    }
}

#[tokio::test]
async fn read_failure_before_send_is_never_outcome_unknown() {
    for request in [trigger(), rerun(), roll_back()] {
        let body = status_json(500, "etcd is unavailable", &[]);
        let (connection, api) =
            FakeApi::connection(WritePolicy::Allowed, move |_| (500, body.clone()));
        let error = connection
            .write(&request, WriteMode::Commit)
            .await
            .expect_err("the read failed");
        assert!(
            matches!(error, WriteError::Cluster(_)),
            "{request:?}: {error:?}"
        );
        assert_nothing_was_sent(&api.requests());
    }
}

#[tokio::test]
async fn an_unusable_object_sends_nothing() {
    let mut cron_job = cron_job_json();
    cron_job["spec"]
        .as_object_mut()
        .expect("spec")
        .remove("jobTemplate");
    let (connection, api) = cluster(vec![(CRON_JOB_PATH, cron_job)], ok_reply);
    let error = connection
        .write(&trigger(), WriteMode::Commit)
        .await
        .expect_err("no template to copy");
    assert!(matches!(error, WriteError::Cluster(_)), "{error:?}");
    assert_nothing_was_sent(&api.requests());
}

#[test]
fn access_check_matches_each_operation() {
    let restart = || WriteOperation::RestartRollout {
        restarted_at: restarted_at(),
    };
    let cases = [
        (
            ObjectKind::Deployment,
            WriteOperation::ScaleWorkload {
                replicas: 1,
                previous: 2,
            },
            "patch deployments/scale",
        ),
        (
            ObjectKind::StatefulSet,
            WriteOperation::ScaleWorkload {
                replicas: 1,
                previous: 2,
            },
            "patch statefulsets/scale",
        ),
        (ObjectKind::Deployment, restart(), "patch deployments"),
        (ObjectKind::StatefulSet, restart(), "patch statefulsets"),
        (ObjectKind::DaemonSet, restart(), "patch daemonsets"),
        (
            ObjectKind::Deployment,
            WriteOperation::SetRolloutPaused { paused: true },
            "patch deployments",
        ),
        (
            ObjectKind::Deployment,
            WriteOperation::RollBackDeployment {
                replica_set: "rs".to_owned(),
                revision: 1,
            },
            "patch deployments",
        ),
        (
            ObjectKind::CronJob,
            WriteOperation::SetCronJobSuspended { suspended: true },
            "patch cronjobs",
        ),
        (
            ObjectKind::CronJob,
            WriteOperation::TriggerCronJob,
            "create jobs",
        ),
        (ObjectKind::Job, WriteOperation::RerunJob, "create jobs"),
    ];
    for (kind, operation, shown) in cases {
        let request = WriteRequest::new(object(kind, "x"), operation).expect("fits");
        assert_eq!(request.access_check().to_string(), shown, "{request:?}");
    }
}

#[test]
fn changed_fields_hold_names_and_numbers_only() {
    let field = |request: &WriteRequest| {
        let fields = request.changed_fields();
        assert_eq!(fields.len(), 1);
        (
            fields[0].path.to_string(),
            fields[0].value.clone().expect("a value"),
        )
    };
    assert_eq!(
        field(&request(
            ObjectKind::Deployment,
            "api",
            WriteOperation::ScaleWorkload {
                replicas: 5,
                previous: 2
            }
        )),
        ("spec.replicas".to_owned(), "5".to_owned())
    );
    assert_eq!(
        field(&request(
            ObjectKind::Deployment,
            "api",
            WriteOperation::RestartRollout {
                restarted_at: restarted_at()
            }
        )),
        (
            "spec.template.metadata.annotations[kubectl.kubernetes.io/restartedAt]".to_owned(),
            "2026-10-02T09:12:03Z".to_owned()
        )
    );
    assert_eq!(
        field(&request(
            ObjectKind::Deployment,
            "api",
            WriteOperation::SetRolloutPaused { paused: false }
        )),
        ("spec.paused".to_owned(), "false".to_owned())
    );
    assert_eq!(
        field(&roll_back()),
        ("spec.template".to_owned(), "rev 37 (api-6c1e2a)".to_owned())
    );
    assert_eq!(
        field(&request(
            ObjectKind::CronJob,
            "nightly",
            WriteOperation::SetCronJobSuspended { suspended: true }
        )),
        ("spec.suspend".to_owned(), "true".to_owned())
    );
    assert_eq!(
        field(&trigger()),
        (
            "metadata.generateName".to_owned(),
            "nightly-manual-".to_owned()
        )
    );
    assert_eq!(
        field(&rerun()),
        (
            "metadata.generateName".to_owned(),
            "etl-1-rerun-".to_owned()
        )
    );
}

#[test]
fn manual_debug_shows_names_only() {
    let restart = request(
        ObjectKind::Deployment,
        "api",
        WriteOperation::RestartRollout {
            restarted_at: restarted_at(),
        },
    );
    assert_eq!(
        format!("{restart:?}"),
        r#"WriteRequest { operation: RestartRollout, kind: Deployment, namespace: Some("payments"), name: "api" }"#
    );
    let shown = format!(
        "{:?} {:?}",
        roll_back(),
        request(
            ObjectKind::Deployment,
            "api",
            WriteOperation::ScaleWorkload {
                replicas: 7,
                previous: 2
            }
        )
    );
    for hidden in ["2026", "api-6c1e2a", "37", "7 }"] {
        assert!(!shown.contains(hidden), "{hidden}: {shown}");
    }
    assert_eq!(
        format!("{:?}", WriteOperation::TriggerCronJob),
        "TriggerCronJob"
    );
    assert_eq!(format!("{:?}", WriteOperation::RerunJob), "RerunJob");
}

#[tokio::test]
async fn every_new_operation_supports_dry_run() {
    for request in [roll_back(), trigger(), rerun()] {
        assert!(request.supports_dry_run(), "{request:?}");
    }
}

#[tokio::test]
async fn blocked_policy_blocks_every_new_operation() {
    let restart = WriteOperation::RestartRollout {
        restarted_at: restarted_at(),
    };
    let requests = [
        request(
            ObjectKind::Deployment,
            "api",
            WriteOperation::ScaleWorkload {
                replicas: 1,
                previous: 2,
            },
        ),
        request(ObjectKind::Deployment, "api", restart),
        request(
            ObjectKind::Deployment,
            "api",
            WriteOperation::SetRolloutPaused { paused: true },
        ),
        roll_back(),
        request(
            ObjectKind::CronJob,
            "nightly",
            WriteOperation::SetCronJobSuspended { suspended: true },
        ),
        trigger(),
        rerun(),
    ];
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, ok_reply);
    for request in &requests {
        for mode in [WriteMode::DryRun, WriteMode::Commit] {
            let error = connection.write(request, mode).await.expect_err("blocked");
            assert!(
                matches!(error, WriteError::WritesBlocked),
                "{request:?}: {error:?}"
            );
        }
    }
    assert!(api.requests().is_empty());
}

/// The query pairs in order; kube starts its query with an empty one.
fn query_pairs(request: &RecordedRequest) -> Vec<&str> {
    request
        .query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .collect()
}

/// The rows that read first: the last request is the write, and its shape is pinned here.
#[tokio::test]
async fn allow_list_matches_the_read_then_send_operations() {
    let (connection, api) = rollback_cluster("dep-uid", "37", "api:1");
    connection
        .write(&roll_back(), WriteMode::DryRun)
        .await
        .expect("accepted");
    let (cron_connection, cron_api) = cluster(vec![(CRON_JOB_PATH, cron_job_json())], ok_reply);
    cron_connection
        .write(&trigger(), WriteMode::DryRun)
        .await
        .expect("accepted");
    let (job_connection, job_api) = cluster(vec![(JOB_PATH, job_json())], ok_reply);
    job_connection
        .write(&rerun(), WriteMode::DryRun)
        .await
        .expect("accepted");
    let rows = [
        (
            api.requests(),
            "PATCH",
            DEPLOYMENT_PATH,
            "application/json-patch+json",
        ),
        (cron_api.requests(), "POST", JOBS_PATH, "application/json"),
        (job_api.requests(), "POST", JOBS_PATH, "application/json"),
    ];
    for (requests, method, path, content_type) in rows {
        let write = requests.last().expect("a request");
        assert!(
            requests[..requests.len() - 1]
                .iter()
                .all(|read| read.method == "GET")
        );
        assert_eq!(write.method, method);
        assert_eq!(write.path, path);
        assert_eq!(query_pairs(write), ["dryRun=All", "fieldManager=k8sboard"]);
        assert_eq!(write.content_type.as_deref(), Some(content_type));
    }
}

#[tokio::test]
async fn rollback_422_that_is_invalid_keeps_field_paths_only() {
    let message = format!("template has {ENV_LITERAL}");
    let reply_body = status_json(422, &message, &["spec.template.spec.containers[0].image"]);
    let (connection, _api) = cluster(
        vec![
            (DEPLOYMENT_PATH, deployment_json("api:2")),
            (REPLICA_SET_PATH, replica_set_json("dep-uid", "37", "api:1")),
        ],
        move |_| (422, reply_body.clone()),
    );
    let error = connection
        .write(&roll_back(), WriteMode::Commit)
        .await
        .expect_err("the template was rejected");
    assert!(
        matches!(&error, WriteError::Invalid { message, fields }
            if message == "the server rejected the template of that revision"
                && fields == &["spec.template.spec.containers[0].image".to_owned()]),
        "{error:?}"
    );
    assert!(!format!("{error} {error:?}").contains(ENV_LITERAL));
}

#[test]
fn scale_above_the_int32_range_is_refused() {
    let scale = |replicas| {
        WriteRequest::new(
            object(ObjectKind::Deployment, "api"),
            WriteOperation::ScaleWorkload {
                replicas,
                previous: 2,
            },
        )
    };
    assert!(scale(i32::MAX as u32).is_some());
    assert!(scale(i32::MAX as u32 + 1).is_none());
    assert!(scale(u32::MAX).is_none());
}

#[tokio::test]
async fn a_commit_of_a_create_reports_the_uid_and_a_dry_run_does_not() {
    let created = json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "nightly-manual-x7k2p", "uid": "job-uid-1"},
    })
    .to_string();
    let (connection, _api) = cluster(vec![(CRON_JOB_PATH, cron_job_json())], move |_| {
        (201, created.clone())
    });
    let committed = connection
        .write(&trigger(), WriteMode::Commit)
        .await
        .expect("created");
    assert_eq!(committed.uid.as_deref(), Some("job-uid-1"));
    let dry_run = connection
        .write(&trigger(), WriteMode::DryRun)
        .await
        .expect("accepted");
    assert_eq!(dry_run.uid, None);
}

#[tokio::test]
async fn a_patch_reports_no_uid() {
    let (connection, _api) = cluster(Vec::new(), ok_reply);
    let scale = request(
        ObjectKind::Deployment,
        "api",
        WriteOperation::ScaleWorkload {
            replicas: 1,
            previous: 2,
        },
    );
    let outcome = connection
        .write(&scale, WriteMode::Commit)
        .await
        .expect("patched");
    assert_eq!(outcome.uid, None);
}

#[tokio::test]
async fn the_raw_query_carries_no_other_parameter() {
    let scale = request(
        ObjectKind::Deployment,
        "api",
        WriteOperation::ScaleWorkload {
            replicas: 1,
            previous: 2,
        },
    );
    let requests = written(&scale, WriteMode::DryRun).await;
    let post = requests.last().expect("a request");
    // kube starts its query with an empty pair, which `query_pairs` drops.
    assert_eq!(
        post.query.trim_start_matches('&'),
        "dryRun=All&fieldManager=k8sboard"
    );
}

fn set_image(kind: ObjectKind, change_cause: Option<&str>) -> WriteRequest {
    request(
        kind,
        "api",
        WriteOperation::SetContainerImage {
            container: "web".to_owned(),
            image: "nginx:1.26-alpine".to_owned(),
            previous_image: "nginx:1.27-alpine".to_owned(),
            change_cause: change_cause.map(str::to_owned),
        },
    )
}

#[tokio::test]
async fn set_image_sends_a_strategic_patch_to_each_workload_kind() {
    for (kind, path) in [
        (
            ObjectKind::Deployment,
            "/apis/apps/v1/namespaces/payments/deployments/api",
        ),
        (
            ObjectKind::StatefulSet,
            "/apis/apps/v1/namespaces/payments/statefulsets/api",
        ),
        (
            ObjectKind::DaemonSet,
            "/apis/apps/v1/namespaces/payments/daemonsets/api",
        ),
    ] {
        let requests = written(&set_image(kind, Some("release test")), WriteMode::Commit).await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "PATCH");
        assert_eq!(requests[0].path, path);
        assert_eq!(
            requests[0].content_type.as_deref(),
            Some("application/strategic-merge-patch+json")
        );
        let body = body_of(&requests[0]);
        assert_eq!(
            body["spec"]["template"]["spec"]["containers"],
            json!([{"name": "web", "image": "nginx:1.26-alpine"}])
        );
        assert_eq!(
            body["metadata"]["annotations"]["kubernetes.io/change-cause"],
            "release test"
        );
        // The previous image is for the summary only.
        assert!(!requests[0].body.contains("1.27"));
    }
}

#[tokio::test]
async fn set_image_dry_run_asks_the_server_to_check_only() {
    let requests = written(&set_image(ObjectKind::Deployment, None), WriteMode::DryRun).await;
    assert_eq!(
        query_pairs(&requests[0]),
        vec!["dryRun=All", "fieldManager=k8sboard"]
    );
}

#[test]
fn set_image_fits_the_three_pod_template_kinds_only() {
    for kind in [
        ObjectKind::Deployment,
        ObjectKind::StatefulSet,
        ObjectKind::DaemonSet,
    ] {
        assert!(WriteRequest::new(object(kind, "api"), set_image_operation()).is_some());
    }
    for kind in [ObjectKind::CronJob, ObjectKind::Job, ObjectKind::Pod] {
        assert!(WriteRequest::new(object(kind, "api"), set_image_operation()).is_none());
    }
}

fn set_image_operation() -> WriteOperation {
    WriteOperation::SetContainerImage {
        container: "web".to_owned(),
        image: "nginx:1.26-alpine".to_owned(),
        previous_image: String::new(),
        change_cause: None,
    }
}

#[test]
fn set_image_refuses_a_container_or_image_the_server_would_refuse() {
    let refused = |container: &str, image: &str, cause: Option<&str>| {
        WriteRequest::new(
            object(ObjectKind::Deployment, "api"),
            WriteOperation::SetContainerImage {
                container: container.to_owned(),
                image: image.to_owned(),
                previous_image: String::new(),
                change_cause: cause.map(str::to_owned),
            },
        )
        .is_none()
    };
    assert!(!refused("web", "nginx:1", None));
    assert!(refused("Web", "nginx:1", None));
    assert!(refused("", "nginx:1", None));
    assert!(refused("web", "", None));
    assert!(refused("web", "nginx 1", None));
    assert!(refused("web", "nginx:1", Some("a\nb")));
}

#[test]
fn set_image_stores_the_cause_trimmed_and_an_empty_one_as_none() {
    let cause_of = |text: &str| match set_image(ObjectKind::Deployment, Some(text)).operation() {
        WriteOperation::SetContainerImage { change_cause, .. } => change_cause.clone(),
        _ => unreachable!("built as a Set image"),
    };
    assert_eq!(cause_of("  release test "), Some("release test".to_owned()));
    assert_eq!(cause_of("   "), None);
}

#[test]
fn set_image_summary_names_the_container_path_and_the_cause() {
    let fields = set_image(ObjectKind::Deployment, Some("release test")).changed_fields();
    let listed: Vec<_> = fields
        .iter()
        .map(|field| (field.path.to_string(), field.value.clone()))
        .collect();
    assert_eq!(
        listed,
        vec![
            (
                "spec.template.spec.containers[web].image".to_owned(),
                Some("nginx:1.26-alpine".to_owned())
            ),
            (
                "metadata.annotations[kubernetes.io/change-cause]".to_owned(),
                Some("release test".to_owned())
            ),
        ]
    );
    assert_eq!(
        set_image(ObjectKind::Deployment, None)
            .changed_fields()
            .len(),
        1
    );
}

#[test]
fn a_scale_records_the_old_count_beside_the_new_one() {
    let fields = request(
        ObjectKind::Deployment,
        "api",
        WriteOperation::ScaleWorkload {
            replicas: 0,
            previous: 3,
        },
    )
    .changed_fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].path, "spec.replicas");
    assert_eq!(fields[0].from.as_deref(), Some("3"));
    assert_eq!(fields[0].value.as_deref(), Some("0"));
}
