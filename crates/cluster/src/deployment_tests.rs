use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{
    DeploymentCondition, DeploymentSpec, DeploymentStatus, DeploymentStrategy,
    RollingUpdateDeployment,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use super::*;

fn deployment_with_spec(spec: DeploymentSpec) -> Deployment {
    Deployment {
        spec: Some(spec),
        ..Default::default()
    }
}

#[test]
fn deployment_summary_reads_replica_counts() {
    let deployment = Deployment {
        metadata: ObjectMeta {
            namespace: Some("shop".to_owned()),
            name: Some("api".to_owned()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(3),
            ..Default::default()
        }),
        status: Some(DeploymentStatus {
            ready_replicas: Some(2),
            updated_replicas: Some(3),
            available_replicas: Some(1),
            conditions: Some(vec![DeploymentCondition {
                type_: "Available".to_owned(),
                status: "True".to_owned(),
                reason: Some(String::new()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
    };
    let summary = deployment_summary(&deployment);
    assert_eq!(
        (summary.namespace.as_str(), summary.name.as_str()),
        ("shop", "api")
    );
    assert_eq!(
        (
            summary.desired,
            summary.ready,
            summary.up_to_date,
            summary.available
        ),
        (3, 2, 3, 1)
    );
    assert_eq!(
        summary.conditions,
        [WorkloadCondition {
            name: "Available".to_owned(),
            is_true: true,
            reason: None,
            message: None,
            last_transition: None,
        }]
    );
}

#[test]
fn deployment_desired_defaults_to_one() {
    let summary = deployment_summary(&deployment_with_spec(DeploymentSpec::default()));
    assert_eq!(summary.desired, 1);
    assert_eq!((summary.ready, summary.available), (0, 0));
    assert_eq!(deployment_summary(&Deployment::default()).desired, 1);
}

#[test]
fn deployment_strategy_reads_surge_and_unavailable() {
    let summary = deployment_summary(&deployment_with_spec(DeploymentSpec {
        paused: Some(true),
        strategy: Some(DeploymentStrategy {
            type_: Some("RollingUpdate".to_owned()),
            rolling_update: Some(RollingUpdateDeployment {
                max_surge: Some(IntOrString::String("25%".to_owned())),
                max_unavailable: Some(IntOrString::Int(1)),
            }),
        }),
        ..Default::default()
    }));
    assert_eq!(summary.strategy, "RollingUpdate");
    assert_eq!(summary.max_surge.as_deref(), Some("25%"));
    assert_eq!(summary.max_unavailable.as_deref(), Some("1"));
    assert!(summary.is_paused);
}

#[test]
fn deployment_revision_reads_annotation() {
    let annotations = BTreeMap::from([
        (
            "deployment.kubernetes.io/revision".to_owned(),
            "7".to_owned(),
        ),
        ("other".to_owned(), "dropped".to_owned()),
    ]);
    let deployment = Deployment {
        metadata: ObjectMeta {
            annotations: Some(annotations),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        deployment_summary(&deployment).revision.as_deref(),
        Some("7")
    );
    assert_eq!(deployment_summary(&Deployment::default()).revision, None);
}

#[test]
fn progress_deadline_defaults_to_600() {
    let summary = deployment_summary(&Deployment::default());
    assert_eq!(summary.progress_deadline_seconds, 600);
    let configured = deployment_with_spec(DeploymentSpec {
        progress_deadline_seconds: Some(120),
        ..Default::default()
    });
    assert_eq!(
        deployment_summary(&configured).progress_deadline_seconds,
        120
    );
}

#[test]
fn condition_message_is_kept() {
    let deployment = Deployment {
        status: Some(DeploymentStatus {
            conditions: Some(vec![DeploymentCondition {
                type_: "ReplicaFailure".to_owned(),
                status: "True".to_owned(),
                message: Some("quota exceeded".to_owned()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let summary = deployment_summary(&deployment);
    assert_eq!(
        summary.conditions[0].message.as_deref(),
        Some("quota exceeded")
    );
}

fn managed_fields(entries: &[serde_json::Value]) -> Deployment {
    serde_json::from_value(serde_json::json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "api", "managedFields": entries},
    }))
    .expect("a deployment")
}

fn entry(manager: &str, time: &str, fields: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "manager": manager, "operation": "Update", "apiVersion": "apps/v1",
        "time": time, "fieldsType": "FieldsV1", "fieldsV1": fields,
    })
}

fn owns_template() -> serde_json::Value {
    serde_json::json!({"f:spec": {"f:template": {"f:spec": {"f:containers": {}}}}})
}

#[test]
fn template_writer_is_the_newest_template_owner() {
    let deployment = managed_fields(&[
        entry("ci-bot", "2026-10-04T10:00:00Z", owns_template()),
        entry("kubectl-edit", "2026-10-04T09:00:00Z", owns_template()),
    ]);
    let writer = deployment_summary(&deployment)
        .template_change
        .expect("a writer");
    assert_eq!(writer.manager, "ci-bot");
    assert_eq!(writer.at.to_string(), "2026-10-04T10:00:00Z");
}

#[test]
fn template_writer_ignores_status_and_non_template_entries() {
    let mut status = entry(
        "deployment-controller",
        "2026-10-04T11:00:00Z",
        owns_template(),
    );
    status["subresource"] = "status".into();
    let mut scale = entry("hpa", "2026-10-04T11:30:00Z", owns_template());
    scale["subresource"] = "scale".into();
    let metadata_only = entry(
        "labeler",
        "2026-10-04T11:45:00Z",
        serde_json::json!({"f:metadata": {"f:labels": {}}}),
    );
    let replicas_only = entry(
        "scaler",
        "2026-10-04T11:50:00Z",
        serde_json::json!({"f:spec": {"f:replicas": {}}}),
    );
    let deployment = managed_fields(&[
        status,
        scale,
        metadata_only,
        replicas_only,
        entry("argocd", "2026-10-04T08:00:00Z", owns_template()),
    ]);
    let writer = deployment_summary(&deployment)
        .template_change
        .expect("a writer");
    assert_eq!(writer.manager, "argocd");
}

#[test]
fn template_writer_is_none_without_managed_fields() {
    assert_eq!(
        deployment_summary(&Deployment::default()).template_change,
        None
    );
}
