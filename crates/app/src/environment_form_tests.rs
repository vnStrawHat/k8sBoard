use super::*;
use crate::cluster_registry::{ClusterEntry, ClusterRef};
use crate::environment::Environment;
use crate::write_guard::ConfirmMode;

fn custom(name: &str, tier: EnvironmentTier) -> CustomEnvironment {
    CustomEnvironment {
        name: name.to_owned(),
        color: EnvironmentColor::Teal,
        tier,
    }
}

fn entry_on(context: &str, key: EnvironmentKey) -> ClusterEntry {
    ClusterEntry {
        cluster: ClusterRef {
            kubeconfig: "a.yaml".into(),
            context: context.to_owned(),
        },
        display_name: None,
        environment: Some(key),
        read_only: None,
        confirm: None,
        default_namespace: None,
        allow_node_shell: None,
        debug_image: None,
        node_shell_namespace: None,
        proxy: None,
        metrics: None,
    }
}

fn on(name: &str) -> EnvironmentKey {
    EnvironmentKey::Custom(name.to_owned())
}

fn message(text: &str, own: Option<usize>, list: &[CustomEnvironment]) -> String {
    validate_environment_name(text, own, list)
        .expect_err("rejected")
        .0
        .to_string()
}

#[test]
fn name_rules() {
    let list = [custom("QA", EnvironmentTier::Staging)];
    let long = "x".repeat(17);
    let cases = [
        ("", "Enter a name."),
        ("   ", "Enter a name."),
        (long.as_str(), "Use at most 16 characters."),
        ("a\tb", "Remove line breaks and tabs."),
        (
            "production",
            "'production' is used by a built-in environment.",
        ),
        ("PROD", "'PROD' is used by a built-in environment."),
        ("Auto", "'Auto' is used by a built-in environment."),
        (
            "Development · Local",
            "'Development · Local' is used by a built-in environment.",
        ),
        ("qa", "Another environment is already named 'qa'."),
    ];
    for (text, expected) in cases {
        assert_eq!(message(text, None, &list), expected, "{text:?}");
    }
    assert_eq!(
        validate_environment_name(" QA2 ", None, &list),
        Ok("QA2".to_owned())
    );
    assert_eq!(
        validate_environment_name(&"x".repeat(16), None, &list),
        Ok("x".repeat(16))
    );
}

#[test]
fn rename_to_other_case_is_allowed() {
    let list = [custom("qa", EnvironmentTier::Staging)];
    assert_eq!(
        validate_environment_name("QA", Some(0), &list),
        Ok("QA".to_owned())
    );
}

#[test]
fn add_uses_purple_and_production() {
    let mut registry = ClusterRegistry::default();
    add_environment(&mut registry, "QA".to_owned());
    add_environment(&mut registry, "DR".to_owned());
    assert_eq!(registry.environments[1].name, "DR");
    assert_eq!(registry.environments[1].color, EnvironmentColor::Purple);
    assert_eq!(registry.environments[1].tier, EnvironmentTier::Production);
}

#[test]
fn rename_rewrites_references() {
    let mut registry = ClusterRegistry {
        environments: vec![custom("QA", EnvironmentTier::Staging)],
        clusters: vec![
            entry_on("a", on("QA")),
            entry_on("b", on("QA")),
            entry_on("c", EnvironmentKey::BuiltIn(EnvironmentTier::Local)),
        ],
        ..ClusterRegistry::default()
    };
    rename_environment(&mut registry, 0, "QA2".to_owned());
    assert_eq!(registry.environments[0].name, "QA2");
    assert_eq!(registry.clusters[0].environment, Some(on("QA2")));
    assert_eq!(registry.clusters[1].environment, Some(on("QA2")));
    assert_eq!(
        registry.clusters[2].environment,
        Some(EnvironmentKey::BuiltIn(EnvironmentTier::Local))
    );
}

#[test]
fn delete_moves_clusters_to_the_tier() {
    let mut registry = ClusterRegistry {
        environments: vec![custom("QA", EnvironmentTier::Staging)],
        clusters: vec![entry_on("a", on("QA"))],
        ..ClusterRegistry::default()
    };
    let summary = cluster::ContextSummary {
        name: "a".to_owned(),
        cluster: "a-cluster".to_owned(),
        user: None,
        namespace: None,
        source: "a.yaml".into(),
    };
    let before = registry.profile(&summary);
    delete_environment(&mut registry, 0);
    assert!(registry.environments.is_empty());
    assert_eq!(
        registry.clusters[0].environment,
        Some(EnvironmentKey::BuiltIn(EnvironmentTier::Staging))
    );
    let after = registry.profile(&summary);
    assert_eq!(after.environment, Environment::STAGING);
    assert_eq!(after.confirm, before.confirm);
    assert_eq!(after.read_only, before.read_only);
    assert_eq!(after.confirm, ConfirmMode::Click);
}

#[test]
fn is_weaker_follows_risk_order() {
    assert!(is_weaker(
        EnvironmentTier::Production,
        EnvironmentTier::Staging
    ));
    assert!(!is_weaker(
        EnvironmentTier::Local,
        EnvironmentTier::Development
    ));
    assert!(!is_weaker(
        EnvironmentTier::Staging,
        EnvironmentTier::Staging
    ));
}

#[test]
fn weaken_dialog_text_counts() {
    let dr = custom("DR", EnvironmentTier::Production);
    let (title, one) = weaken_dialog_text(&dr, EnvironmentTier::Staging, 1);
    assert_eq!(title, "Change DR to Staging rules?");
    assert_eq!(
        one,
        "1 cluster using DR will follow the Staging rules instead of the Production rules."
    );
    let (_, three) = weaken_dialog_text(&dr, EnvironmentTier::Staging, 3);
    assert_eq!(
        three,
        "3 clusters using DR will follow the Staging rules instead of the Production rules."
    );
}

#[test]
fn deleting_a_skipped_row_keeps_references() {
    // `qa` repeats `QA`, so no reference ever resolved to it.
    let mut registry = ClusterRegistry {
        environments: vec![
            custom("QA", EnvironmentTier::Staging),
            custom("qa", EnvironmentTier::Local),
        ],
        clusters: vec![entry_on("a", on("qa")), entry_on("b", on("QA"))],
        ..ClusterRegistry::default()
    };
    delete_environment(&mut registry, 1);
    assert_eq!(registry.environments.len(), 1);
    assert_eq!(registry.clusters[0].environment, Some(on("qa")));
    assert_eq!(registry.clusters[1].environment, Some(on("QA")));
}

#[test]
fn edits_match_references_exactly() {
    let mut registry = ClusterRegistry {
        environments: vec![custom("QA", EnvironmentTier::Staging)],
        clusters: vec![entry_on("a", on("QA")), entry_on("b", on("qa"))],
        ..ClusterRegistry::default()
    };
    assert_eq!(clusters_using(&registry, "QA"), 1);
    rename_environment(&mut registry, 0, "QA2".to_owned());
    assert_eq!(registry.clusters[0].environment, Some(on("QA2")));
    assert_eq!(registry.clusters[1].environment, Some(on("qa")));
}

#[test]
fn clusters_using_counts_unloaded_entries() {
    let mut other_file = entry_on("z", on("QA"));
    other_file.cluster.kubeconfig = "never-loaded.yaml".into();
    let registry = ClusterRegistry {
        environments: vec![custom("QA", EnvironmentTier::Staging)],
        clusters: vec![entry_on("a", on("QA")), other_file],
        ..ClusterRegistry::default()
    };
    assert_eq!(clusters_using(&registry, "QA"), 2);
    assert_eq!(clusters_using(&registry, "DR"), 0);
}

#[test]
fn delete_dialog_text_counts() {
    let qa = custom("QA", EnvironmentTier::Staging);
    let (title, none) = delete_dialog_text(&qa, 0);
    assert_eq!(title, "Delete environment QA?");
    assert_eq!(none, "No cluster uses it.");
    let (_, one) = delete_dialog_text(&qa, 1);
    assert_eq!(
        one,
        "1 cluster uses it and moves to Staging, the built-in environment with the same confirm rules."
    );
    let (_, three) = delete_dialog_text(&qa, 3);
    assert_eq!(
        three,
        "3 clusters use it and move to Staging, the built-in environment with the same confirm rules."
    );
}

#[test]
fn edit_environment_ignores_a_missing_row() {
    let mut registry = ClusterRegistry::default();
    edit_environment(&mut registry, 3, |environment| {
        environment.color = EnvironmentColor::Red;
    });
    assert!(registry.environments.is_empty());
    rename_environment(&mut registry, 3, "X".to_owned());
    delete_environment(&mut registry, 3);
    assert!(registry.environments.is_empty());
}
