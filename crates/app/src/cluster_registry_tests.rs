use super::*;
use crate::settings_store::{SettingsNotice, load_settings};

fn summary(name: &str, source: &str) -> ContextSummary {
    ContextSummary {
        name: name.to_owned(),
        cluster: format!("{name}-cluster"),
        user: None,
        namespace: None,
        source: PathBuf::from(source),
    }
}

fn cluster(context: &str, source: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from(source),
        context: context.to_owned(),
    }
}

fn entry(context: &str, source: &str) -> ClusterEntry {
    ClusterEntry {
        cluster: cluster(context, source),
        display_name: None,
        environment: None,
        read_only: None,
        default_namespace: None,
    }
}

fn registry_with(entry: ClusterEntry) -> ClusterRegistry {
    ClusterRegistry {
        clusters: vec![entry],
        ..ClusterRegistry::default()
    }
}

#[test]
fn profile_of_unregistered_context_uses_name_and_guess() {
    let profile = ClusterRegistry::default().profile(&summary("prod-eu", "a.yaml"));
    assert_eq!(
        profile,
        ClusterProfile {
            display_name: "prod-eu".to_owned(),
            environment: Environment::Production,
            default_namespace: None,
            read_only: true,
        }
    );
}

#[test]
fn read_only_defaults_to_production() {
    let registry = ClusterRegistry::default();
    assert!(registry.profile(&summary("prod-eu", "a.yaml")).read_only);
    assert!(!registry.profile(&summary("dev-1", "a.yaml")).read_only);
    assert!(!registry.profile(&summary("stage", "a.yaml")).read_only);
}

#[test]
fn stored_read_only_overrides_the_environment_default() {
    let mut unlocked = entry("prod-eu", "a.yaml");
    unlocked.read_only = Some(false);
    let profile = registry_with(unlocked).profile(&summary("prod-eu", "a.yaml"));
    assert!(!profile.read_only);
    let mut locked = entry("dev-1", "a.yaml");
    locked.read_only = Some(true);
    assert!(
        registry_with(locked)
            .profile(&summary("dev-1", "a.yaml"))
            .read_only
    );
}

#[test]
fn entry_overrides_name_and_environment() {
    let mut overrides = entry("readonly@Monitor", "a.yaml");
    overrides.display_name = Some("uat-monitor".to_owned());
    overrides.environment = Some(Environment::Production);
    overrides.default_namespace = Some("monitoring".to_owned());
    let profile = registry_with(overrides).profile(&summary("readonly@Monitor", "a.yaml"));
    assert_eq!(profile.display_name, "uat-monitor");
    assert_eq!(profile.environment, Environment::Production);
    assert_eq!(profile.default_namespace.as_deref(), Some("monitoring"));
}

#[test]
fn blank_display_name_falls_back_to_context() {
    let mut overrides = entry("ctx", "a.yaml");
    overrides.display_name = Some("   ".to_owned());
    let profile = registry_with(overrides).profile(&summary("ctx", "a.yaml"));
    assert_eq!(profile.display_name, "ctx");
}

#[test]
fn entry_matches_only_the_same_source() {
    let mut overrides = entry("ctx", "a.yaml");
    overrides.display_name = Some("renamed".to_owned());
    let registry = registry_with(overrides);
    assert_eq!(
        registry.profile(&summary("ctx", "a.yaml")).display_name,
        "renamed"
    );
    assert_eq!(
        registry.profile(&summary("ctx", "b.yaml")).display_name,
        "ctx"
    );
}

#[test]
fn start_choice_prefers_requested_in_load_order() {
    let chain = summary("same", "chain.yaml");
    let registered = summary("same", "extra.yaml");
    let choice = start_choice(Some("same"), None, &[&chain, &registered]);
    assert_eq!(choice, StartChoice::Cluster(cluster("same", "chain.yaml")));
}

#[test]
fn requested_missing_is_reported() {
    let known = summary("known", "a.yaml");
    let choice = start_choice(Some("nope"), Some(&cluster("known", "a.yaml")), &[&known]);
    assert_eq!(choice, StartChoice::RequestedMissing);
}

#[test]
fn start_choice_uses_last_used_from_the_same_source() {
    let chain = summary("same", "chain.yaml");
    let registered = summary("same", "extra.yaml");
    let last_used = cluster("same", "extra.yaml");
    let choice = start_choice(None, Some(&last_used), &[&chain, &registered]);
    assert_eq!(choice, StartChoice::Cluster(last_used));
}

#[test]
fn last_used_beats_current_context_of_the_same_file() {
    let first = summary("first", "x.yaml");
    let second = summary("second", "x.yaml");
    // `first` would be the current-context; the saved cluster wins.
    let last_used = cluster("second", "x.yaml");
    let choice = start_choice(None, Some(&last_used), &[&first, &second]);
    assert_eq!(choice, StartChoice::Cluster(last_used));
}

#[test]
fn stale_last_used_is_ignored() {
    let known = summary("known", "a.yaml");
    let gone = cluster("gone", "a.yaml");
    assert_eq!(
        start_choice(None, Some(&gone), &[&known]),
        StartChoice::CurrentContext
    );
}

#[test]
fn no_last_used_gives_the_current_context() {
    let known = summary("known", "a.yaml");
    assert_eq!(
        start_choice(None, None, &[&known]),
        StartChoice::CurrentContext
    );
}

#[test]
fn switcher_label_adds_file_name_for_duplicate_names() {
    let context = summary("same", "dir/extra.yaml");
    let profile = ClusterRegistry::default().profile(&context);
    assert_eq!(switcher_label(&profile, &context, false), "same");
    assert_eq!(
        switcher_label(&profile, &context, true),
        "same · extra.yaml"
    );
}

#[test]
fn entry_without_context_is_corrupt() {
    let dir = std::env::temp_dir().join(format!("k8sboard-0024-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let text = r#"{"version": 1, "registry": {"clusters": [{"kubeconfig": "a.yaml"}]}}"#;
    std::fs::write(dir.join("settings.json"), text).expect("write fixture");
    let loaded = load_settings(&dir);
    assert!(
        matches!(loaded.notice, Some(SettingsNotice::Reset { .. })),
        "{:?}",
        loaded.notice
    );
    assert_eq!(loaded.settings, crate::settings::Settings::default());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn registry_serializes_only_what_is_set() {
    let mut overrides = entry("ctx", "a.yaml");
    overrides.environment = Some(Environment::Staging);
    let value = serde_json::to_value(registry_with(overrides)).expect("serializes");
    assert_eq!(
        value,
        serde_json::json!({
            "clusters": [{ "kubeconfig": "a.yaml", "context": "ctx", "environment": "staging" }]
        })
    );
}

#[test]
fn explicit_kubeconfig_ignores_last_used_from_other_files() {
    let saved = cluster("ctx", "registry.yaml");
    let named = [PathBuf::from("named.yaml")];
    assert_eq!(launch_last_used(Some(&saved), Some(&named)), None);
    let own = cluster("ctx", "named.yaml");
    assert_eq!(launch_last_used(Some(&own), Some(&named)), Some(&own));
}

#[test]
fn without_explicit_kubeconfig_any_last_used_counts() {
    let saved = cluster("ctx", "registry.yaml");
    assert_eq!(launch_last_used(Some(&saved), None), Some(&saved));
    assert_eq!(launch_last_used(None, None), None);
}

#[test]
fn entry_mut_appends_once() {
    let mut registry = ClusterRegistry::default();
    let target = cluster("ctx", "a.yaml");
    registry.entry_mut(&target).default_namespace = Some("monitoring".to_owned());
    registry.entry_mut(&target).display_name = Some("renamed".to_owned());
    assert_eq!(registry.clusters.len(), 1);
    let entry = &registry.clusters[0];
    assert_eq!(entry.default_namespace.as_deref(), Some("monitoring"));
    assert_eq!(entry.display_name.as_deref(), Some("renamed"));
    registry.entry_mut(&cluster("ctx", "b.yaml"));
    assert_eq!(registry.clusters.len(), 2);
}
