use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{Container, ContainerPort as ApiContainerPort, EnvVar, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelectorRequirement, OwnerReference};

use super::*;

fn owner(kind: &str, name: &str, is_controller: Option<bool>) -> OwnerReference {
    OwnerReference {
        kind: kind.to_owned(),
        name: name.to_owned(),
        controller: is_controller,
        ..Default::default()
    }
}

fn expression(key: &str, operator: &str, values: &[&str]) -> LabelSelectorRequirement {
    LabelSelectorRequirement {
        key: key.to_owned(),
        operator: operator.to_owned(),
        values: Some(values.iter().map(|value| (*value).to_owned()).collect()),
    }
}

fn template_with(containers: Vec<Container>, init_containers: Vec<Container>) -> PodTemplateSpec {
    PodTemplateSpec {
        spec: Some(PodSpec {
            containers,
            init_containers: Some(init_containers),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn api_port(port: i32, name: Option<&str>, protocol: Option<&str>) -> ApiContainerPort {
    ApiContainerPort {
        container_port: port,
        name: name.map(str::to_owned),
        protocol: protocol.map(str::to_owned),
        ..Default::default()
    }
}

#[test]
fn selector_terms_format_labels_and_expressions() {
    let selector = LabelSelector {
        match_labels: Some(BTreeMap::from([
            ("tier".to_owned(), "web".to_owned()),
            ("app".to_owned(), "api".to_owned()),
        ])),
        match_expressions: Some(vec![
            expression("env", "In", &["prod", "stage"]),
            expression("zone", "NotIn", &["a"]),
            expression("canary", "Exists", &[]),
            expression("legacy", "DoesNotExist", &[]),
        ]),
    };
    assert_eq!(
        selector_terms(&selector),
        [
            "app=api",
            "tier=web",
            "env in (prod,stage)",
            "zone notin (a)",
            "canary",
            "!legacy",
        ]
    );
    assert!(selector_terms(&LabelSelector::default()).is_empty());
}

#[test]
fn label_terms_follow_key_order() {
    let metadata = ObjectMeta {
        labels: Some(BTreeMap::from([
            ("b".to_owned(), "2".to_owned()),
            ("a".to_owned(), "1".to_owned()),
        ])),
        ..Default::default()
    };
    assert_eq!(label_terms(&metadata), ["a=1", "b=2"]);
    assert!(label_terms(&ObjectMeta::default()).is_empty());
}

#[test]
fn controller_ref_is_owner_with_controller_true() {
    let metadata = |owners| ObjectMeta {
        owner_references: Some(owners),
        ..Default::default()
    };
    let owners = vec![
        owner("Node", "node-1", None),
        owner("ReplicaSet", "web-abc", Some(true)),
        owner("Other", "x", Some(false)),
    ];
    assert_eq!(
        controller_ref(&metadata(owners)),
        Some(ControllerRef {
            kind: "ReplicaSet".to_owned(),
            name: "web-abc".to_owned(),
        })
    );
    assert_eq!(
        controller_ref(&metadata(vec![owner("Node", "node-1", Some(false))])),
        None
    );
    assert_eq!(controller_ref(&ObjectMeta::default()), None);
}

#[test]
fn template_containers_read_main_images_and_ports() {
    let main = Container {
        name: "web".to_owned(),
        image: Some("nginx:1.27".to_owned()),
        ports: Some(vec![
            api_port(80, Some("http"), None),
            api_port(53, None, Some("UDP")),
            api_port(70_000, None, None),
        ]),
        ..Default::default()
    };
    let init = Container {
        name: "migrate".to_owned(),
        image: Some("migrate:1".to_owned()),
        ..Default::default()
    };
    let bare = Container {
        name: "bare".to_owned(),
        ..Default::default()
    };
    let containers = template_containers(&template_with(vec![main, bare], vec![init]));
    assert_eq!(
        containers,
        [
            TemplateContainer {
                name: "web".to_owned(),
                image: "nginx:1.27".to_owned(),
                ports: vec![
                    ContainerPort {
                        name: Some("http".to_owned()),
                        port: 80,
                        protocol: "TCP".to_owned(),
                    },
                    ContainerPort {
                        name: None,
                        port: 53,
                        protocol: "UDP".to_owned(),
                    },
                ],
            },
            TemplateContainer {
                name: "bare".to_owned(),
                image: String::new(),
                ports: Vec::new(),
            },
        ]
    );
    assert!(template_containers(&PodTemplateSpec::default()).is_empty());
}

#[test]
fn template_containers_keep_only_name_image_and_ports() {
    let container = Container {
        name: "app".to_owned(),
        image: Some("app:1".to_owned()),
        env: Some(vec![EnvVar {
            name: "DB_PASSWORD".to_owned(),
            value: Some("distinctive-env-value".to_owned()),
            ..Default::default()
        }]),
        args: Some(vec!["--token=distinctive-arg-value".to_owned()]),
        command: Some(vec!["distinctive-command-value".to_owned()]),
        ..Default::default()
    };
    let containers = template_containers(&template_with(vec![container], Vec::new()));
    let text = format!("{containers:?}");
    assert!(text.contains("app:1"));
    assert!(!text.contains("distinctive-env-value"));
    assert!(!text.contains("DB_PASSWORD"));
    assert!(!text.contains("distinctive-arg-value"));
    assert!(!text.contains("distinctive-command-value"));
}

#[test]
fn int_or_string_text_reads_percent_and_number() {
    assert_eq!(
        int_or_string_text(&IntOrString::String("25%".to_owned())),
        "25%"
    );
    assert_eq!(int_or_string_text(&IntOrString::Int(1)), "1");
}

#[test]
fn condition_reads_truth_and_drops_empty_reason() {
    let reasoned = condition("Available", "True", Some("MinimumReplicasAvailable"));
    assert!(reasoned.is_true);
    assert_eq!(reasoned.reason.as_deref(), Some("MinimumReplicasAvailable"));
    let unknown = condition("Progressing", "Unknown", Some(""));
    assert!(!unknown.is_true);
    assert_eq!(unknown.reason, None);
    assert!(!condition("Failed", "False", None).is_true);
}

#[test]
fn optional_count_reads_absent_and_negative_as_zero() {
    assert_eq!(optional_count(Some(3)), 3);
    assert_eq!(optional_count(Some(-1)), 0);
    assert_eq!(optional_count(None), 0);
}

#[test]
fn revision_keeps_only_the_deployment_revision_annotation() {
    let metadata = ObjectMeta {
        annotations: Some(BTreeMap::from([
            (REVISION_ANNOTATION.to_owned(), "12".to_owned()),
            (
                "kubectl.kubernetes.io/last-applied-configuration".to_owned(),
                "{}".to_owned(),
            ),
        ])),
        ..Default::default()
    };
    assert_eq!(revision(&metadata).as_deref(), Some("12"));
    assert_eq!(revision(&ObjectMeta::default()), None);
}
