use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};
use crate::object_write::WritePolicy;
use crate::port_forward::LocalPort;

const NS: &str = "team-a";
const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"not found","reason":"NotFound","code":404}"#;

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

fn pod_gone(_request: &RecordedRequest) -> (u16, String) {
    (404, NOT_FOUND.to_owned())
}

struct PodSpec<'a> {
    name: &'a str,
    labels: &'a [(&'a str, &'a str)],
    is_running: bool,
    is_ready: bool,
    is_deleting: bool,
    ports: &'a [(&'a str, i32)],
}

impl<'a> PodSpec<'a> {
    fn ready(name: &'a str) -> Self {
        Self {
            name,
            labels: &[],
            is_running: true,
            is_ready: true,
            is_deleting: false,
            ports: &[],
        }
    }

    fn json(&self) -> Value {
        let labels: serde_json::Map<String, Value> = self
            .labels
            .iter()
            .map(|(key, value)| ((*key).to_owned(), json!(value)))
            .collect();
        let ports: Vec<Value> = self
            .ports
            .iter()
            .map(|(name, port)| json!({ "name": name, "containerPort": port }))
            .collect();
        let mut metadata = json!({ "name": self.name, "namespace": NS, "labels": labels });
        if self.is_deleting {
            metadata["deletionTimestamp"] = json!("2026-01-01T00:00:00Z");
        }
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": metadata,
            "spec": { "containers": [{ "name": "app", "ports": ports }] },
            "status": {
                "phase": if self.is_running { "Running" } else { "Pending" },
                "conditions": [{ "type": "Ready", "status": if self.is_ready { "True" } else { "False" } }],
            },
        })
    }

    fn pod(&self) -> Pod {
        serde_json::from_value(self.json()).expect("a valid pod")
    }
}

fn pod_list(pods: &[PodSpec<'_>]) -> String {
    let items: Vec<Value> = pods.iter().map(PodSpec::json).collect();
    json!({ "apiVersion": "v1", "kind": "PodList", "metadata": {}, "items": items }).to_string()
}

#[test]
fn ready_pod_skips_unready_and_deleting() {
    let pending = PodSpec {
        is_running: false,
        is_ready: false,
        ..PodSpec::ready("a-pending")
    };
    let not_ready = PodSpec {
        is_ready: false,
        ..PodSpec::ready("a-not-ready")
    };
    let deleting = PodSpec {
        is_deleting: true,
        ..PodSpec::ready("a-deleting")
    };
    let pods = [
        pending.pod(),
        not_ready.pod(),
        deleting.pod(),
        PodSpec::ready("c-ready").pod(),
        PodSpec::ready("b-ready").pod(),
    ];
    let picked = ready_pod(&pods).expect("two pods are ready");
    assert_eq!(picked.metadata.name.as_deref(), Some("b-ready"));
    assert!(ready_pod(&pods[..3]).is_none());
    assert!(ready_pod(&[]).is_none());
}

fn service_port(port: i32, target: Option<IntOrString>) -> ServicePort {
    ServicePort {
        port,
        target_port: target,
        ..ServicePort::default()
    }
}

#[test]
fn service_target_port_number_name_and_default() {
    let pod = PodSpec {
        ports: &[("http", 8080), ("metrics", 9090)],
        ..PodSpec::ready("api-0")
    }
    .pod();
    let resolve = |target| service_target_port(&service_port(80, target), &pod);
    assert!(matches!(resolve(Some(IntOrString::Int(8000))), Ok(8000)));
    assert!(matches!(
        resolve(Some(IntOrString::String("metrics".to_owned()))),
        Ok(9090)
    ));
    assert!(matches!(resolve(None), Ok(80)));
    let unknown = resolve(Some(IntOrString::String("grpc".to_owned())));
    assert!(matches!(&unknown, Err(ForwardError::PortNotDeclared(name)) if name == "grpc"));
}

fn service_json(selector: Option<Value>, service_type: &str, targets: Value) -> String {
    let mut spec = json!({ "type": service_type, "ports": targets });
    if let Some(selector) = selector {
        spec["selector"] = selector;
    }
    json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": { "name": "web", "namespace": NS },
        "spec": spec,
    })
    .to_string()
}

async fn resolve(
    target: ForwardTarget,
    respond: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static,
) -> Result<ResolvedPod, ForwardError> {
    let (connection, _api) = FakeApi::connection(WritePolicy::Allowed, respond);
    resolve_target(&connection, &request(target, LocalPort::Auto(18080))).await
}

fn service_target() -> ForwardTarget {
    ForwardTarget::Service {
        name: "web".to_owned(),
    }
}

#[tokio::test]
async fn service_resolves_to_a_ready_pod_and_its_target_port() {
    let resolved = resolve(service_target(), |request| {
        if request.path.ends_with("/services/web") {
            let ports = json!([{ "port": 8080, "targetPort": "http" }]);
            return (
                200,
                service_json(Some(json!({ "app": "api" })), "ClusterIP", ports),
            );
        }
        let pods = [
            PodSpec {
                labels: &[("app", "api")],
                ports: &[("http", 3000)],
                ..PodSpec::ready("api-1")
            },
            PodSpec {
                labels: &[("app", "other")],
                ports: &[("http", 4000)],
                ..PodSpec::ready("a-other")
            },
        ];
        (200, pod_list(&pods))
    })
    .await;
    assert_eq!(
        resolved.expect("a ready pod backs the service"),
        ResolvedPod {
            pod: "api-1".to_owned(),
            pod_port: 3000
        }
    );
}

#[tokio::test]
async fn service_without_selector_is_unsupported() {
    let no_selector = resolve(service_target(), |_| {
        (
            200,
            service_json(None, "ClusterIP", json!([{ "port": 8080 }])),
        )
    })
    .await;
    assert!(matches!(no_selector, Err(ForwardError::UnsupportedService)));
    let external_name = resolve(service_target(), |_| {
        let selector = Some(json!({ "app": "api" }));
        (
            200,
            service_json(selector, "ExternalName", json!([{ "port": 8080 }])),
        )
    })
    .await;
    assert!(matches!(
        external_name,
        Err(ForwardError::UnsupportedService)
    ));
}

fn workload_json(kind: &str, selector: Value) -> String {
    json!({
        "apiVersion": "apps/v1",
        "kind": kind,
        "metadata": { "name": "api", "namespace": NS },
        "spec": { "selector": selector, "template": { "spec": { "containers": [] } } },
    })
    .to_string()
}

fn workload_pods() -> String {
    pod_list(&[
        PodSpec {
            labels: &[("app", "api"), ("tier", "web")],
            ..PodSpec::ready("api-web")
        },
        PodSpec {
            labels: &[("app", "api"), ("tier", "db")],
            ..PodSpec::ready("a-db")
        },
        PodSpec {
            labels: &[("app", "other"), ("tier", "web")],
            ..PodSpec::ready("a-other")
        },
    ])
}

fn workload_selector() -> Value {
    json!({
        "matchLabels": { "app": "api" },
        "matchExpressions": [{ "key": "tier", "operator": "In", "values": ["web"] }],
    })
}

fn expected_workload_pod() -> ResolvedPod {
    ResolvedPod {
        pod: "api-web".to_owned(),
        pod_port: 8080,
    }
}

#[tokio::test]
async fn deployment_resolves_through_its_selector() {
    let resolved = resolve(
        ForwardTarget::Deployment {
            name: "api".to_owned(),
        },
        |request| {
            if request.path.ends_with("/deployments/api") {
                (200, workload_json("Deployment", workload_selector()))
            } else {
                (200, workload_pods())
            }
        },
    )
    .await;
    assert_eq!(resolved.expect("a pod matches"), expected_workload_pod());
}

#[tokio::test]
async fn stateful_set_resolves_through_its_selector() {
    let resolved = resolve(
        ForwardTarget::StatefulSet {
            name: "api".to_owned(),
        },
        |request| {
            if request.path.ends_with("/statefulsets/api") {
                (200, workload_json("StatefulSet", workload_selector()))
            } else {
                (200, workload_pods())
            }
        },
    )
    .await;
    assert_eq!(resolved.expect("a pod matches"), expected_workload_pod());
}

#[tokio::test]
async fn workload_without_a_ready_pod_or_selector_resolves_nothing() {
    let target = || ForwardTarget::Deployment {
        name: "api".to_owned(),
    };
    let no_pod = resolve(target(), |request| {
        if request.path.ends_with("/deployments/api") {
            (200, workload_json("Deployment", workload_selector()))
        } else {
            (200, pod_list(&[]))
        }
    })
    .await;
    assert!(matches!(no_pod, Err(ForwardError::NoReadyPod)));
    // An empty selector would match every pod of the namespace.
    let no_selector = resolve(target(), |request| {
        if request.path.ends_with("/deployments/api") {
            (200, workload_json("Deployment", json!({})))
        } else {
            (200, workload_pods())
        }
    })
    .await;
    assert!(matches!(no_selector, Err(ForwardError::NoReadyPod)));
}

#[tokio::test]
async fn missing_target_reads_as_target_not_found() {
    let resolved = resolve(pod_target(), pod_gone).await;
    assert!(matches!(resolved, Err(ForwardError::TargetNotFound)));
}
