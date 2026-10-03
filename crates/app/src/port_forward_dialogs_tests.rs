use super::*;
use crate::port_forwards::TargetKind;

fn input<'a>(
    namespace: &'a str,
    target: &'a str,
    remote_port: &'a str,
    local_port: &'a str,
) -> NewForwardInput<'a> {
    NewForwardInput {
        namespace,
        target,
        remote_port,
        local_port,
    }
}

#[test]
fn an_empty_local_port_is_automatic_and_a_typed_one_is_exact() {
    let auto =
        validate_new_forward(&input("payments", "svc/payments-api", "80", "")).expect("valid");
    assert_eq!(auto.local_port, LocalPortSpec::Auto);
    assert_eq!(auto.namespace, "payments");
    assert_eq!(auto.target.kind, TargetKind::Service);
    assert_eq!(auto.remote_port, 80);

    let exact = validate_new_forward(&input(" payments ", " pod/api-0 ", " 5432 ", " 15432 "))
        .expect("valid");
    assert_eq!(exact.local_port, LocalPortSpec::Exact(15432));
    assert_eq!(exact.target, TargetSpec::pod("api-0"));
}

#[test]
fn each_wrong_field_gets_its_own_error_and_none_starts_a_forward() {
    let errors = validate_new_forward(&input("", "api", "0", "70000")).expect_err("invalid");
    assert_eq!(errors.namespace, Some("Enter a namespace"));
    assert_eq!(
        errors.target,
        Some("Use pod/NAME, svc/NAME, deploy/NAME, or sts/NAME")
    );
    assert_eq!(errors.remote_port, Some("Enter a port from 1 to 65535"));
    assert_eq!(
        errors.local_port,
        Some("Leave empty for automatic, or enter 1 to 65535")
    );

    let errors =
        validate_new_forward(&input("Pay Ments", "pod/a_b", "80", "")).expect_err("invalid");
    assert_eq!(errors.namespace, Some("Not a valid namespace name"));
    assert_eq!(errors.target, Some("Not a valid object name"));
    assert_eq!(errors.remote_port, None);
    assert_eq!(errors.local_port, None);
}

#[test]
fn names_that_change_a_request_path_are_refused() {
    for namespace in ["a/b", "..", "a?b", "a#b"] {
        assert!(
            validate_new_forward(&input(namespace, "pod/api", "80", "")).is_err(),
            "{namespace}"
        );
    }
    assert!(validate_new_forward(&input("shop", "pod/../x", "80", "")).is_err());
}

#[test]
fn change_local_port_takes_1_to_65535() {
    assert_eq!(validate_local_port("8080"), Ok(8080));
    assert_eq!(validate_local_port(" 1 "), Ok(1));
    for text in ["", "0", "65536", "x"] {
        assert_eq!(
            validate_local_port(text),
            Err("Enter a port from 1 to 65535"),
            "{text}"
        );
    }
}

#[test]
fn the_chosen_cluster_is_the_selected_row_even_when_labels_repeat() {
    let entry = |context: &str, environment| FormCluster {
        cluster: ClusterRef {
            kubeconfig: std::path::PathBuf::from("a.yaml"),
            context: context.to_owned(),
        },
        // The same switcher text for two different clusters.
        label: "payments".to_owned(),
        environment,
    };
    let clusters = vec![
        entry("stg-payments", Environment::Staging),
        entry("prod-payments", Environment::Production),
    ];
    let second = cluster_at(&clusters, Some(IndexPath::default().row(1))).expect("a cluster");
    assert_eq!(second.cluster.context, "prod-payments");
    assert_eq!(second.environment, Environment::Production);
    assert!(cluster_at(&clusters, Some(IndexPath::default().row(2))).is_none());
    assert!(cluster_at(&clusters, None).is_none());
}
