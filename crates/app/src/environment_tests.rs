use super::*;

use Environment::{Development, Local, Production, Staging};

#[test]
fn guess_table() {
    let rows = [
        ("prod-eu-1", "", Production),
        (
            "arn:aws:eks:eu-central-1:4471:cluster/prod-eu-1",
            "",
            Production,
        ),
        ("stg-us-1", "", Staging),
        ("uat", "", Staging),
        ("dev-shared", "", Development),
        ("kind-k8sboard", "", Local),
        ("minikube", "", Local),
        ("docker-desktop", "", Local),
        ("kind-prod", "", Production),
        ("latest", "", Staging),
        ("readonly@Monitor", "", Staging),
        ("admin", "prod-1", Production),
        ("staging", "", Staging),
        ("test-cluster", "", Development),
        ("k3d-dev", "", Development),
        ("localhost", "", Local),
    ];
    for (context, cluster, expected) in rows {
        assert_eq!(
            guess_environment(context, cluster),
            expected,
            "context {context:?}, cluster {cluster:?}"
        );
    }
}

#[test]
fn riskiest_match_wins() {
    assert_eq!(guess_environment("dev-prod", ""), Production);
    assert_eq!(guess_environment("dev-stg", ""), Staging);
    assert_eq!(guess_environment("kind-dev", ""), Development);
}

#[test]
fn tokens_ignore_substrings() {
    assert_eq!(guess_environment("latest", ""), Staging);
    assert_eq!(guess_environment("contest", ""), Staging);
}

#[test]
fn trailing_digits_are_trimmed() {
    assert_eq!(guess_environment("prod1", ""), Production);
    assert_eq!(guess_environment("dev01", ""), Development);
}

#[test]
fn cluster_name_counts_when_context_is_unknown() {
    assert_eq!(guess_environment("admin@x", "prod-eu"), Production);
    assert_eq!(guess_environment("admin@x", "dev-eu"), Development);
}

#[test]
fn uat_monitor_context_is_staging() {
    assert_eq!(
        guess_environment("readonly@Monitor", "cluster.local"),
        Staging
    );
}

#[test]
fn badge_text_per_environment() {
    assert_eq!(Local.badge(), "LOCAL");
    assert_eq!(Development.badge(), "DEV");
    assert_eq!(Staging.badge(), "STG");
    assert_eq!(Production.badge(), "PROD");
}

#[test]
fn environment_order_is_risk() {
    assert!(Local < Development && Development < Staging && Staging < Production);
    let riskiest = [Development, Production, Local].into_iter().max();
    assert_eq!(riskiest, Some(Production));
}

#[test]
fn environment_serializes_lowercase() {
    let text = serde_json::to_string(&Production).expect("serializes");
    assert_eq!(text, "\"production\"");
    let back: Environment = serde_json::from_str("\"local\"").expect("parses");
    assert_eq!(back, Local);
}
