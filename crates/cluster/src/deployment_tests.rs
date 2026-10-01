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
