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
        confirm: None,
        default_namespace: None,
        allow_node_shell: None,
        debug_image: None,
        node_shell_namespace: None,
        color: None,
        proxy: None,
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
            confirm: ConfirmMode::TypeName,
            allow_node_shell: false,
            debug_image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
            node_shell_namespace: "kube-system".to_owned(),
            color: crate::environment::ClusterColor::Red,
            proxy: Ok(cluster::ProxyChoice::Kubeconfig),
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

fn profile_with_default(default_namespace: Option<&str>) -> ClusterProfile {
    let mut entry = entry("ctx", "a.yaml");
    entry.default_namespace = default_namespace.map(str::to_owned);
    registry_with(entry).profile(&summary("ctx", "a.yaml"))
}

#[test]
fn start_scope_prefers_memory() {
    let target = cluster("ctx", "a.yaml");
    let mut memory = ScopeMemory::new();
    remember_scope(
        &mut memory,
        target.clone(),
        NamespaceScope::Named("kube-system".to_owned()),
    );
    assert_eq!(
        start_scope(&memory, &target, &profile_with_default(Some("shop"))),
        Some(NamespaceScope::Named("kube-system".to_owned()))
    );
}

#[test]
fn start_scope_uses_default_namespace() {
    let target = cluster("ctx", "a.yaml");
    assert_eq!(
        start_scope(
            &ScopeMemory::new(),
            &target,
            &profile_with_default(Some("shop"))
        ),
        Some(NamespaceScope::Named("shop".to_owned()))
    );
}

#[test]
fn start_scope_is_none_without_memory_or_default() {
    let target = cluster("ctx", "a.yaml");
    assert_eq!(
        start_scope(&ScopeMemory::new(), &target, &profile_with_default(None)),
        None
    );
}

#[test]
fn start_scope_ignores_the_memory_of_another_cluster() {
    let mut memory = ScopeMemory::new();
    remember_scope(&mut memory, cluster("ctx", "b.yaml"), NamespaceScope::All);
    assert_eq!(
        start_scope(
            &memory,
            &cluster("ctx", "a.yaml"),
            &profile_with_default(None)
        ),
        None
    );
}

#[test]
fn remember_scope_replaces_previous_value() {
    let target = cluster("ctx", "a.yaml");
    let mut memory = ScopeMemory::new();
    remember_scope(&mut memory, target.clone(), NamespaceScope::All);
    remember_scope(
        &mut memory,
        target.clone(),
        NamespaceScope::Named("web".to_owned()),
    );
    assert_eq!(
        memory.get(&target),
        Some(&NamespaceScope::Named("web".to_owned()))
    );
}

#[test]
fn confirm_defaults_follow_the_environment() {
    let registry = ClusterRegistry::default();
    assert_eq!(
        registry.profile(&summary("prod-eu", "a.yaml")).confirm,
        ConfirmMode::TypeName
    );
    assert_eq!(
        registry.profile(&summary("dev-1", "a.yaml")).confirm,
        ConfirmMode::Click
    );
}

#[test]
fn stored_confirm_overrides_the_environment_default() {
    let mut typed = entry("dev-1", "a.yaml");
    typed.confirm = Some(ConfirmMode::TypeName);
    let profile = registry_with(typed).profile(&summary("dev-1", "a.yaml"));
    assert_eq!(profile.confirm, ConfirmMode::TypeName);
    let mut clicked = entry("prod-eu", "a.yaml");
    clicked.confirm = Some(ConfirmMode::Click);
    let profile = registry_with(clicked).profile(&summary("prod-eu", "a.yaml"));
    assert_eq!(profile.confirm, ConfirmMode::Click);
}

#[test]
fn confirm_round_trips() {
    let mut typed = entry("dev-1", "a.yaml");
    typed.confirm = Some(ConfirmMode::TypeName);
    let registry = registry_with(typed);
    let text = serde_json::to_string(&registry).expect("serializes");
    assert!(text.contains(r#""confirm":"type-name""#), "{text}");
    let back: ClusterRegistry = serde_json::from_str(&text).expect("parses");
    assert_eq!(back, registry);
}

#[test]
fn an_entry_without_confirm_omits_the_key() {
    let text = serde_json::to_string(&registry_with(entry("dev-1", "a.yaml"))).expect("serializes");
    assert!(!text.contains("confirm"), "{text}");
}

fn allows_node_shell(
    context: &str,
    environment: Option<Environment>,
    stored: Option<bool>,
) -> bool {
    let mut overrides = entry(context, "a.yaml");
    overrides.environment = environment;
    overrides.allow_node_shell = stored;
    registry_with(overrides)
        .profile(&summary(context, "a.yaml"))
        .allow_node_shell
}

#[test]
fn allow_node_shell_defaults_by_environment() {
    // Production is off, set or guessed.
    assert!(!allows_node_shell("prod-eu", None, None));
    assert!(!allows_node_shell(
        "anything",
        Some(Environment::Production),
        None
    ));
    // Development is on only when it is set (a `dev` in a name is a guess a production cluster can
    // share, like `devops-core`); Local is on, set or guessed.
    assert!(!allows_node_shell("dev-1", None, None));
    assert!(!allows_node_shell("devops-core", None, None));
    assert!(allows_node_shell(
        "anything",
        Some(Environment::Development),
        None
    ));
    assert!(allows_node_shell("kind-local", None, None));
    assert!(allows_node_shell(
        "anything",
        Some(Environment::Local),
        None
    ));
    // Staging is on only when the entry sets it; a guessed or unknown one is off.
    assert!(allows_node_shell(
        "anything",
        Some(Environment::Staging),
        None
    ));
    assert!(!allows_node_shell("stg-1", None, None));
    assert!(!allows_node_shell("mystery", None, None));
    assert!(
        !ClusterRegistry::default()
            .profile(&summary("mystery", "a.yaml"))
            .allow_node_shell
    );
}

#[test]
fn an_explicit_allow_node_shell_wins() {
    assert!(allows_node_shell("prod-eu", None, Some(true)));
    assert!(!allows_node_shell("dev-1", None, Some(false)));
    assert!(!allows_node_shell(
        "anything",
        Some(Environment::Local),
        Some(false)
    ));
}

#[test]
fn debug_image_and_node_shell_namespace_default_and_override() {
    let defaults = ClusterRegistry::default().profile(&summary("dev-1", "a.yaml"));
    assert_eq!(defaults.debug_image, cluster::DEFAULT_DEBUG_IMAGE);
    assert_eq!(defaults.node_shell_namespace, "kube-system");
    let mut overrides = entry("dev-1", "a.yaml");
    overrides.debug_image = Some(" registry.local/busybox:1 ".to_owned());
    overrides.node_shell_namespace = Some("debug".to_owned());
    let profile = registry_with(overrides).profile(&summary("dev-1", "a.yaml"));
    assert_eq!(profile.debug_image, "registry.local/busybox:1");
    assert_eq!(profile.node_shell_namespace, "debug");
    // A blank value is no value.
    let mut blank = entry("dev-1", "a.yaml");
    blank.debug_image = Some("  ".to_owned());
    blank.node_shell_namespace = Some(String::new());
    let profile = registry_with(blank).profile(&summary("dev-1", "a.yaml"));
    assert_eq!(profile.debug_image, cluster::DEFAULT_DEBUG_IMAGE);
    assert_eq!(profile.node_shell_namespace, "kube-system");
}

#[test]
fn profile_color_prefers_the_entry() {
    let mut teal = entry("prod-eu", "a.yaml");
    teal.color = Some(ClusterColor::Teal);
    let profile = registry_with(teal).profile(&summary("prod-eu", "a.yaml"));
    assert_eq!(profile.color, ClusterColor::Teal);
    // The badge follows the environment, not the stored color.
    assert_eq!(profile.environment, Environment::Production);
    let plain = ClusterRegistry::default().profile(&summary("prod-eu", "a.yaml"));
    assert_eq!(plain.color, ClusterColor::Red);
}

#[test]
fn a_color_follows_the_environment_when_none_is_stored() {
    let mut moved = entry("dev-1", "a.yaml");
    moved.environment = Some(Environment::Staging);
    let profile = registry_with(moved).profile(&summary("dev-1", "a.yaml"));
    assert_eq!(profile.color, ClusterColor::Amber);
}

// ---- Spec 0043 step 4: the proxy ----

#[test]
fn cluster_proxy_json_shape() {
    assert_eq!(
        serde_json::to_value(ClusterProxy::Direct).expect("serializes"),
        serde_json::json!("direct")
    );
    assert_eq!(
        serde_json::to_value(ClusterProxy::Url("http://proxy:3128".to_owned()))
            .expect("serializes"),
        serde_json::json!({ "url": "http://proxy:3128" })
    );
    let back: ClusterProxy =
        serde_json::from_value(serde_json::json!({ "url": "socks5://p:1" })).expect("parses");
    assert_eq!(back, ClusterProxy::Url("socks5://p:1".to_owned()));
    // Absent means the kubeconfig's own proxy.
    let plain = ClusterRegistry::default().profile(&summary("prod-eu", "a.yaml"));
    assert_eq!(plain.proxy, Ok(ProxyChoice::Kubeconfig));
    let mut direct = entry("prod-eu", "a.yaml");
    direct.proxy = Some(ClusterProxy::Direct);
    let profile = registry_with(direct).profile(&summary("prod-eu", "a.yaml"));
    assert_eq!(profile.proxy, Ok(ProxyChoice::Direct));
}

#[test]
fn a_stored_url_becomes_a_parsed_choice() {
    let mut custom = entry("prod-eu", "a.yaml");
    custom.proxy = Some(ClusterProxy::Url("HTTP://p:3128/".to_owned()));
    let profile = registry_with(custom).profile(&summary("prod-eu", "a.yaml"));
    let Ok(ProxyChoice::Url(url)) = profile.proxy else {
        panic!("a valid URL parses");
    };
    assert_eq!(url.display(), "http://p:3128");
}

#[tokio::test]
async fn invalid_stored_proxy_fails_closed() {
    let mut bad = entry("prod-eu", "a.yaml");
    bad.proxy = Some(ClusterProxy::Url("http://u:p@x".to_owned()));
    let profile = registry_with(bad).profile(&summary("prod-eu", "a.yaml"));
    assert_eq!(profile.proxy, Err(ProxyUrlError::Credentials));
    let yaml = "clusters:\n  - name: c\n    cluster: { server: \"https://127.0.0.1:1\" }\ncontexts:\n  - name: ctx\n    context: { cluster: c }\n";
    let kubeconfig = Kubeconfig::parse(yaml, std::path::Path::new("a.yaml")).expect("parses");
    let Err(error) = open_cluster(&kubeconfig, "ctx", &profile.proxy).await else {
        panic!("a bad proxy must not open a client");
    };
    assert!(matches!(
        error,
        ClusterError::InvalidProxy {
            source: ProxyUrlError::Credentials,
            ..
        }
    ));
    let text = format!("{error} {error:?}");
    assert!(!text.contains("u:p"), "{text}");
    // A good choice opens a client without a round trip.
    open_cluster(&kubeconfig, "ctx", &Ok(ProxyChoice::Direct))
        .await
        .expect("a direct client builds");
}

#[test]
fn debug_of_cluster_proxy_hides_userinfo() {
    let hand_edited = ClusterProxy::Url("http://u:p@x:1".to_owned());
    assert_eq!(format!("{hand_edited:?}"), "Url(<invalid>)");
    let valid = ClusterProxy::Url("http://x:1".to_owned());
    assert_eq!(format!("{valid:?}"), "Url(http://x:1)");
    assert_eq!(format!("{:?}", ClusterProxy::Direct), "Direct");
    // The same holds through the entry that holds it.
    let mut stored = entry("ctx", "a.yaml");
    stored.proxy = Some(hand_edited);
    let text = format!("{stored:?}");
    assert!(!text.contains("u:p"), "{text}");
}
