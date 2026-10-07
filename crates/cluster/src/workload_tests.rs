use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{Container, ContainerPort as ApiContainerPort, EnvVar, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;

use super::*;

fn owner(kind: &str, name: &str, is_controller: Option<bool>) -> OwnerReference {
    OwnerReference {
        kind: kind.to_owned(),
        name: name.to_owned(),
        controller: is_controller,
        ..Default::default()
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
                resources: Vec::new(),
                name: "web".to_owned(),
                image: "nginx:1.27".to_owned(),
                ports: vec![
                    ContainerPort {
                        name: Some("http".to_owned()),
                        port: 80,
                        protocol: "TCP".to_owned(),
                        host_port: None,
                    },
                    ContainerPort {
                        name: None,
                        port: 53,
                        protocol: "UDP".to_owned(),
                        host_port: None,
                    },
                ],
            },
            TemplateContainer {
                resources: Vec::new(),
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
    let reasoned = condition(
        "Available",
        "True",
        Some("MinimumReplicasAvailable"),
        None,
        None,
    );
    assert!(reasoned.is_true);
    assert_eq!(reasoned.reason.as_deref(), Some("MinimumReplicasAvailable"));
    let unknown = condition("Progressing", "Unknown", Some(""), None, None);
    assert!(!unknown.is_true);
    assert_eq!(unknown.reason, None);
    assert!(!condition("Failed", "False", None, None, None).is_true);
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

#[test]
fn condition_message_is_cut() {
    let short = condition("Failed", "True", None, Some("  quota exceeded \n"), None);
    assert_eq!(short.message.as_deref(), Some("quota exceeded"));
    assert_eq!(
        condition("Failed", "True", None, Some(""), None).message,
        None
    );
    assert_eq!(condition("Failed", "True", None, None, None).message, None);
    let long = "x".repeat(5_000);
    let cut = condition("Failed", "True", None, Some(&long), None);
    let message = cut.message.expect("message kept");
    assert!(message.len() < 1_100);
    assert!(message.ends_with('\u{2026}'));
}

#[test]
fn container_port_keeps_host_port() {
    let with_host = ApiContainerPort {
        host_port: Some(8080),
        ..api_port(80, None, None)
    };
    let out_of_range = ApiContainerPort {
        host_port: Some(70_000),
        ..api_port(81, None, None)
    };
    let container = Container {
        name: "web".to_owned(),
        ports: Some(vec![with_host, out_of_range, api_port(82, None, None)]),
        ..Default::default()
    };
    let host_ports: Vec<_> = container_ports(&container)
        .iter()
        .map(|port| port.host_port)
        .collect();
    assert_eq!(host_ports, [Some(8080), None, None]);
}

#[test]
fn condition_keeps_the_last_transition_time() {
    let at = jiff::Timestamp::UNIX_EPOCH;
    assert_eq!(
        condition("Available", "True", None, None, Some(at)).last_transition,
        Some(at)
    );
}

fn annotated(pairs: &[(&str, &str)]) -> ObjectMeta {
    ObjectMeta {
        annotations: Some(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        ),
        ..Default::default()
    }
}

#[test]
fn annotation_terms_list_the_keys_in_order() {
    let metadata = annotated(&[("team", "shop"), ("a.io/owner", "infra")]);
    assert_eq!(
        annotation_terms(&metadata).terms(),
        ["a.io/owner=infra", "team=shop"]
    );
    assert!(annotation_terms(&ObjectMeta::default()).terms().is_empty());
}

#[test]
fn annotation_terms_leave_out_an_applied_manifest() {
    let metadata = annotated(&[
        (
            "kubectl.kubernetes.io/last-applied-configuration",
            "{\"data\":{\"password\":\"s3cret\"}}",
        ),
        ("team", "shop"),
    ]);
    assert_eq!(annotation_terms(&metadata).terms(), ["team=shop"]);
}

#[test]
fn annotation_terms_hide_the_value_of_a_credential_key() {
    let metadata = annotated(&[("deploy-token", "abc123"), ("team", "shop")]);
    let terms = annotation_terms(&metadata);
    assert_eq!(terms.terms(), ["deploy-token=<hidden>", "team=shop"]);
}

#[test]
fn annotation_terms_cut_a_long_value_to_one_line() {
    let long = format!("first line\n{}", "x".repeat(300));
    let terms = annotation_terms(&annotated(&[("note", &long)]));
    let terms = terms.terms();
    assert!(terms[0].ends_with('…'));
    assert!(!terms[0].contains('\n'));
    assert!(terms[0].chars().count() < 220);
}

#[test]
fn annotation_terms_keep_at_most_fifty() {
    let pairs: Vec<(String, String)> = (0..80)
        .map(|n| (format!("k{n:02}"), "v".to_owned()))
        .collect();
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(annotation_terms(&annotated(&refs)).terms().len(), 50);
}

#[test]
fn debug_prints_the_count_and_no_value() {
    let terms = annotation_terms(&annotated(&[("owner", "SECRETANNOTATION")]));
    assert_eq!(format!("{terms:?}"), "AnnotationTerms(1)");
}
