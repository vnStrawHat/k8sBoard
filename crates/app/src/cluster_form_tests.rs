use std::path::PathBuf;

use super::*;
use crate::cluster_registry::ClusterEntry;
use crate::kubeconfig_import::PASTED_DIR;

const AUTH_YAML: &str = "\
clusters:
  - name: c
    cluster: { server: 'https://127.0.0.1:1' }
users:
  - name: u
    user:
      exec: { command: /usr/bin/aws }
";

/// A kubeconfig whose contexts all come from `source` (a path string, so Windows-shaped
/// paths work on every host).
fn kubeconfig(source: &str, contexts: &[&str]) -> Kubeconfig {
    let mut yaml = AUTH_YAML.to_owned();
    yaml.push_str("contexts:\n");
    for name in contexts {
        yaml.push_str(&format!(
            "  - name: {name}\n    context: {{ cluster: c, user: u }}\n"
        ));
    }
    Kubeconfig::parse(&yaml, Path::new(source)).expect("fixture parses")
}

fn groups_of(
    kubeconfigs: &[&Kubeconfig],
    registry: &ClusterRegistry,
    chain: &[&str],
    owned: Option<&Path>,
) -> Vec<ClusterGroup> {
    cluster_groups(
        kubeconfigs,
        registry,
        |path| chain.iter().any(|file| path == Path::new(file)),
        |_| None,
        owned,
    )
}

fn rows_of(groups: Vec<ClusterGroup>) -> Vec<ClusterRow> {
    groups.into_iter().flat_map(|group| group.rows).collect()
}

fn titles(groups: &[ClusterGroup]) -> Vec<&str> {
    groups.iter().map(|group| &*group.title).collect()
}

fn labels(group: &ClusterGroup) -> Vec<&str> {
    group.rows.iter().map(|row| row.label.as_str()).collect()
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
        proxy: None,
        metrics: None,
    }
}

#[test]
fn groups_follow_env_order_and_merge_dev_and_local() {
    let file = kubeconfig(
        "a.yaml",
        &["kind-local", "dev-1", "stage-1", "prod-1", "prod-2"],
    );
    let groups = groups_of(&[&file], &ClusterRegistry::default(), &[], None);
    assert_eq!(
        titles(&groups),
        ["Production", "Staging", "Development · Local"]
    );
    assert_eq!(labels(&groups[0]), ["prod-1", "prod-2"]);
    assert_eq!(labels(&groups[2]), ["kind-local", "dev-1"]);
}

#[test]
fn empty_groups_are_skipped() {
    let file = kubeconfig("a.yaml", &["prod-1"]);
    let groups = groups_of(&[&file], &ClusterRegistry::default(), &[], None);
    assert_eq!(titles(&groups), ["Production"]);
}

/// A registry with the custom environments `names` (all Staging tier) and the given
/// `(context, custom name)` entries.
fn registry_with_custom(names: &[&str], on_custom: &[(&str, &str)]) -> ClusterRegistry {
    let mut registry = ClusterRegistry {
        environments: names
            .iter()
            .map(|name| CustomEnvironment {
                name: (*name).to_owned(),
                color: crate::environment::EnvironmentColor::Teal,
                tier: EnvironmentTier::Staging,
            })
            .collect(),
        ..ClusterRegistry::default()
    };
    for (context, name) in on_custom {
        registry.entry_mut(&cluster(context, "a.yaml")).environment = Some(
            crate::environment::EnvironmentKey::Custom((*name).to_owned()),
        );
    }
    registry
}

#[test]
fn custom_groups_follow_builtins_in_list_order() {
    let file = kubeconfig("a.yaml", &["prod-1", "stage-1", "dev-1", "x-qa", "x-dr"]);
    let registry = registry_with_custom(&["QA", "Idle", "DR"], &[("x-qa", "QA"), ("x-dr", "DR")]);
    let groups = groups_of(&[&file], &registry, &[], None);
    assert_eq!(
        titles(&groups),
        ["Production", "Staging", "Development · Local", "QA", "DR"]
    );
    assert_eq!(labels(&groups[3]), ["x-qa"]);
    assert_eq!(labels(&groups[4]), ["x-dr"]);
}

#[test]
fn filter_matches_custom_badge() {
    let file = kubeconfig("a.yaml", &["prod-1", "x-1"]);
    let registry = registry_with_custom(&["QA"], &[("x-1", "QA")]);
    let groups = groups_of(&[&file], &registry, &[], None);
    let found = filter_groups(&groups, "qa");
    assert_eq!(titles(&found), ["QA"]);
}

#[test]
fn registered_rows_come_first_in_registry_order() {
    let file = kubeconfig("a.yaml", &["prod-1", "prod-2", "prod-3"]);
    let registry = ClusterRegistry {
        clusters: vec![entry("prod-3", "a.yaml"), entry("prod-2", "a.yaml")],
        ..ClusterRegistry::default()
    };
    let groups = groups_of(&[&file], &registry, &[], None);
    assert_eq!(labels(&groups[0]), ["prod-3", "prod-2", "prod-1"]);
}

#[test]
fn row_meta_shows_auth_kind_and_file_name() {
    for source in [
        "/home/u/.kube/config",
        "C:\x5cUsers\x5cu\x5c.kube\x5cconfig",
    ] {
        let file = kubeconfig(source, &["prod-1"]);
        let groups = groups_of(&[&file], &ClusterRegistry::default(), &[], None);
        assert_eq!(groups[0].rows[0].meta, "exec: aws · config", "{source}");
    }
}

#[test]
fn duplicate_context_names_get_file_labels() {
    let first = kubeconfig("a.yaml", &["prod-1"]);
    let second = kubeconfig("b.yaml", &["prod-1"]);
    let groups = groups_of(&[&first, &second], &ClusterRegistry::default(), &[], None);
    assert_eq!(labels(&groups[0]), ["prod-1 · a.yaml", "prod-1 · b.yaml"]);
}

#[test]
fn origin_is_chain_registry_or_app_owned() {
    let chain = kubeconfig("chain.yaml", &["prod-chain"]);
    let added = kubeconfig("added.yaml", &["prod-added"]);
    let owned_dir = PathBuf::from("config");
    let pasted_source = owned_dir.join(PASTED_DIR).join("pasted.yaml");
    let pasted = kubeconfig(&pasted_source.to_string_lossy(), &["prod-pasted"]);
    let groups = groups_of(
        &[&chain, &added, &pasted],
        &ClusterRegistry::default(),
        &["chain.yaml"],
        Some(&owned_dir),
    );
    let origins: Vec<RowOrigin> = groups[0].rows.iter().map(|row| row.origin).collect();
    assert_eq!(
        origins,
        [RowOrigin::Chain, RowOrigin::Registry, RowOrigin::AppOwned]
    );
}

#[test]
fn count_text_uses_singular_and_plural() {
    assert_eq!(count_text(1, 1), "1 cluster · 1 kubeconfig file");
    assert_eq!(count_text(10, 2), "10 clusters · 2 kubeconfig files");
    assert_eq!(count_text(0, 0), "0 clusters · 0 kubeconfig files");
}

/// Rows of one file with the given `(context, label)` pairs.
fn rows_with(pairs: &[(&str, &str)]) -> Vec<ClusterRow> {
    let names: Vec<&str> = pairs.iter().map(|(context, _)| *context).collect();
    let file = kubeconfig("a.yaml", &names);
    let mut rows = rows_of(groups_of(&[&file], &ClusterRegistry::default(), &[], None));
    for row in &mut rows {
        if let Some((_, label)) = pairs
            .iter()
            .find(|(context, _)| *context == row.cluster.context)
        {
            row.label = (*label).to_owned();
        }
    }
    rows
}

#[test]
fn display_name_table() {
    let rows = rows_with(&[("one", "Alpha"), ("two", "Beta")]);
    let me = cluster("one", "a.yaml");
    let check =
        |text: &str| validate_display_name(text, &me, &rows).map_err(|error| error.0.to_string());
    assert_eq!(check("  "), Ok(None));
    assert_eq!(check(" Gamma "), Ok(Some("Gamma".to_owned())));
    assert_eq!(
        check(&"x".repeat(65)),
        Err("Use at most 64 characters.".to_owned())
    );
    assert_eq!(check(&"x".repeat(64)), Ok(Some("x".repeat(64))));
    assert_eq!(
        check("a\tb"),
        Err("Remove line breaks and tabs.".to_owned())
    );
    assert_eq!(
        check("BETA"),
        Err("Another cluster is already shown as 'BETA'.".to_owned())
    );
}

#[test]
fn display_name_may_equal_its_own_label() {
    let rows = rows_with(&[("one", "Alpha"), ("two", "Beta")]);
    let me = cluster("one", "a.yaml");
    assert_eq!(
        validate_display_name("alpha", &me, &rows),
        Ok(Some("alpha".to_owned()))
    );
}

#[test]
fn namespace_table() {
    let message =
        "Use 1–63 lowercase letters, digits, or '-', starting and ending with a letter or digit.";
    for good in ["a", "kube-system", "a1-b2", &"a".repeat(63)] {
        assert_eq!(
            validate_namespace(good),
            Ok(Some(good.to_owned())),
            "{good}"
        );
    }
    for bad in ["-a", "a-", "A", "a_b", "a b", &"a".repeat(64)] {
        assert_eq!(
            validate_namespace(bad).map_err(|error| error.0.to_string()),
            Err(message.to_owned()),
            "{bad}"
        );
    }
    assert_eq!(validate_namespace(""), Ok(None));
    assert_eq!(validate_namespace("  "), Ok(None));
}

#[test]
fn reset_removes_only_that_entry() {
    let mut registry = ClusterRegistry {
        clusters: vec![entry("one", "a.yaml"), entry("two", "a.yaml")],
        ..ClusterRegistry::default()
    };
    reset_entry(&mut registry, &cluster("one", "a.yaml"));
    assert_eq!(registry.clusters, [entry("two", "a.yaml")]);
}

#[test]
fn remove_drops_path_entries_and_last_used() {
    let mut registry = ClusterRegistry {
        kubeconfigs: vec![PathBuf::from("a.yaml"), PathBuf::from("b.yaml")],
        clusters: vec![entry("one", "a.yaml"), entry("two", "b.yaml")],
        last_used: Some(cluster("one", "a.yaml")),
        ..ClusterRegistry::default()
    };
    remove_kubeconfig(&mut registry, Path::new("a.yaml"));
    assert_eq!(registry.kubeconfigs, [PathBuf::from("b.yaml")]);
    assert_eq!(registry.clusters, [entry("two", "b.yaml")]);
    assert_eq!(registry.last_used, None);
}

#[test]
fn remove_keeps_a_last_used_of_another_file() {
    let mut registry = ClusterRegistry {
        kubeconfigs: vec![PathBuf::from("a.yaml")],
        last_used: Some(cluster("two", "b.yaml")),
        ..ClusterRegistry::default()
    };
    remove_kubeconfig(&mut registry, Path::new("a.yaml"));
    assert_eq!(registry.last_used, Some(cluster("two", "b.yaml")));
}

#[test]
fn edit_entry_drops_an_entry_edited_back_to_defaults() {
    let mut registry = ClusterRegistry::default();
    let target = cluster("one", "a.yaml");
    edit_entry(&mut registry, &target, |entry| {
        entry.display_name = Some("Alpha".to_owned());
    });
    assert_eq!(registry.clusters.len(), 1);
    edit_entry(&mut registry, &target, |entry| entry.display_name = None);
    assert!(registry.clusters.is_empty());
}

#[test]
fn edit_entry_keeps_entries_of_other_clusters() {
    let mut registry = ClusterRegistry {
        clusters: vec![entry("other", "a.yaml")],
        ..ClusterRegistry::default()
    };
    edit_entry(&mut registry, &cluster("one", "a.yaml"), |_| {});
    assert_eq!(registry.clusters, [entry("other", "a.yaml")]);
}

#[test]
fn selection_keeps_a_present_row_then_prefers_then_first() {
    let file = kubeconfig("a.yaml", &["prod-1", "prod-2"]);
    let groups = groups_of(&[&file], &ClusterRegistry::default(), &[], None);
    let one = cluster("prod-1", "a.yaml");
    let two = cluster("prod-2", "a.yaml");
    let gone = cluster("gone", "a.yaml");
    assert_eq!(
        resolve_selection(Some(&two), Some(&one), &groups),
        Some(two.clone())
    );
    assert_eq!(
        resolve_selection(Some(&gone), Some(&two), &groups),
        Some(two.clone())
    );
    assert_eq!(
        resolve_selection(Some(&gone), Some(&gone), &groups),
        Some(one)
    );
    assert_eq!(resolve_selection(None, None, &[]), None);
}

#[test]
fn remove_text_names_the_file_and_the_clusters() {
    let file = kubeconfig("C:\x5cUsers\x5cu\x5cextra.yaml", &["prod-1", "prod-2"]);
    let rows = rows_of(groups_of(&[&file], &ClusterRegistry::default(), &[], None));
    let path = &rows[0].cluster.kubeconfig;
    let (title, body) = remove_dialog_text(path, &rows, RowOrigin::Registry);
    assert_eq!(title, "Remove extra.yaml from k8sBoard?");
    assert_eq!(
        body,
        "2 clusters from this file leave the list: prod-1, prod-2. The file itself is not changed."
    );
    let (_, owned) = remove_dialog_text(path, &rows[..1], RowOrigin::AppOwned);
    assert!(
        owned.starts_with("1 cluster from this file leaves the list"),
        "{owned}"
    );
    assert!(owned.ends_with("it will be deleted."), "{owned}");
}

#[test]
fn test_connection_reports_unreachable_server() {
    let yaml = "\
clusters:
  - name: c
    cluster: { server: 'https://127.0.0.1:1' }
users:
  - name: u
    user: { token: fixture-token-value }
contexts:
  - name: ctx
    context: { cluster: c, user: u }
";
    let kubeconfig = Arc::new(Kubeconfig::parse(yaml, Path::new("fixture.yaml")).expect("parses"));
    // Real time: gpui's `advance_clock` does not drive tokio timers.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let state = runtime.block_on(test_connection(
        kubeconfig,
        "ctx".to_owned(),
        Ok(ProxyChoice::Kubeconfig),
        Duration::from_secs(2),
    ));
    let TestState::Failed(message) = state else {
        panic!("an unreachable server must fail, got {state:?}");
    };
    let text = message.to_string();
    assert!(text.contains("ctx"), "{text}");
    assert!(!text.contains("fixture-token-value"), "{text}");
}

/// Live check against a real cluster: `K8SBOARD_LIVE_KUBECONFIG` names the kubeconfig and
/// `K8SBOARD_LIVE_CONTEXT` the context. Sends one `GET /version`.
#[test]
#[ignore = "needs a reachable cluster"]
fn live_test_connection_reports_the_server_version() {
    let (Some(path), Some(context)) = (
        std::env::var_os("K8SBOARD_LIVE_KUBECONFIG"),
        std::env::var("K8SBOARD_LIVE_CONTEXT").ok(),
    ) else {
        panic!("set K8SBOARD_LIVE_KUBECONFIG and K8SBOARD_LIVE_CONTEXT");
    };
    let loaded = Kubeconfig::load(&[PathBuf::from(path)]).expect("the kubeconfig loads");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let state = runtime.block_on(test_connection(
        Arc::new(loaded.kubeconfig),
        context,
        Ok(ProxyChoice::Kubeconfig),
        TEST_CONNECTION_TIMEOUT,
    ));
    let TestState::Connected {
        version,
        latency_ms,
    } = state
    else {
        panic!("not connected: {state:?}");
    };
    eprintln!("Connected · {version} · {latency_ms} ms");
}

#[test]
fn remove_matches_the_path_text_not_the_exact_buffer() {
    let mut registry = ClusterRegistry {
        kubeconfigs: vec![PathBuf::from("dir//sub/./a.yaml"), PathBuf::from("b.yaml")],
        clusters: vec![entry("one", "dir/sub/a.yaml")],
        last_used: Some(cluster("one", "dir/./sub/a.yaml")),
        ..ClusterRegistry::default()
    };
    remove_kubeconfig(&mut registry, Path::new("dir/sub/a.yaml"));
    assert_eq!(registry.kubeconfigs, [PathBuf::from("b.yaml")]);
    assert!(registry.clusters.is_empty());
    assert_eq!(registry.last_used, None);
}

#[test]
fn edit_entry_keeps_an_entry_that_only_sets_the_node_shell() {
    let mut registry = ClusterRegistry::default();
    let target = cluster("one", "a.yaml");
    edit_entry(&mut registry, &target, |entry| {
        entry.allow_node_shell = Some(true);
    });
    assert_eq!(registry.clusters.len(), 1);
    edit_entry(&mut registry, &target, |entry| {
        entry.allow_node_shell = None;
        entry.debug_image = Some("registry.local/busybox:1".to_owned());
    });
    assert_eq!(registry.clusters.len(), 1);
    edit_entry(&mut registry, &target, |entry| {
        entry.debug_image = None;
        entry.node_shell_namespace = Some("debug".to_owned());
    });
    assert_eq!(registry.clusters.len(), 1);
    edit_entry(&mut registry, &target, |entry| {
        entry.node_shell_namespace = None
    });
    assert!(registry.clusters.is_empty());
}

// ---- Spec 0043 step 3: search, order ----

fn reordered(registry: &ClusterRegistry, file: &Kubeconfig) -> Vec<ClusterGroup> {
    groups_of(&[file], registry, &[], None)
}

fn first_group_clusters(groups: &[ClusterGroup]) -> Vec<ClusterRef> {
    groups[0]
        .rows
        .iter()
        .map(|row| row.cluster.clone())
        .collect()
}

#[test]
fn move_cluster_reorders_inside_the_group() {
    let file = kubeconfig("a.yaml", &["prod-a", "prod-b", "prod-c"]);
    let mut registry = ClusterRegistry::default();
    let groups = reordered(&registry, &file);
    move_cluster(
        &mut registry,
        &groups[0],
        &cluster("prod-c", "a.yaml"),
        &cluster("prod-a", "a.yaml"),
    );
    let groups = reordered(&registry, &file);
    assert_eq!(labels(&groups[0]), ["prod-c", "prod-a", "prod-b"]);
    // Dropping on a later row moves the row down to that place.
    let groups = reordered(&registry, &file);
    move_cluster(
        &mut registry,
        &groups[0],
        &cluster("prod-c", "a.yaml"),
        &cluster("prod-b", "a.yaml"),
    );
    let groups = reordered(&registry, &file);
    assert_eq!(labels(&groups[0]), ["prod-a", "prod-b", "prod-c"]);
}

#[test]
fn move_cluster_registers_the_group_only() {
    let file = kubeconfig("a.yaml", &["prod-a", "prod-b", "stg-a", "stg-b"]);
    let mut registry = ClusterRegistry {
        clusters: vec![entry("stg-b", "a.yaml"), entry("stg-a", "a.yaml")],
        ..ClusterRegistry::default()
    };
    let groups = reordered(&registry, &file);
    move_cluster(
        &mut registry,
        &groups[0],
        &cluster("prod-b", "a.yaml"),
        &cluster("prod-a", "a.yaml"),
    );
    // The two Production rows got entries; the Staging entries keep their order.
    let order: Vec<&str> = registry
        .clusters
        .iter()
        .map(|entry| entry.cluster.context.as_str())
        .collect();
    assert_eq!(order, ["stg-b", "stg-a", "prod-b", "prod-a"]);
    let groups = reordered(&registry, &file);
    assert_eq!(labels(&groups[1]), ["stg-b", "stg-a"]);
    assert_eq!(labels(&groups[0]), ["prod-b", "prod-a"]);
}

#[test]
fn move_cluster_across_groups_is_a_no_op() {
    let file = kubeconfig("a.yaml", &["prod-a", "dev-a", "dev-b"]);
    let mut registry = ClusterRegistry::default();
    let groups = reordered(&registry, &file);
    move_cluster(
        &mut registry,
        &groups[0],
        &cluster("prod-a", "a.yaml"),
        &cluster("dev-a", "a.yaml"),
    );
    assert_eq!(registry, ClusterRegistry::default());
}

#[test]
fn step_cluster_stops_at_the_ends() {
    let file = kubeconfig("a.yaml", &["prod-a", "prod-b", "prod-c"]);
    let mut registry = ClusterRegistry::default();
    let groups = reordered(&registry, &file);
    let before = first_group_clusters(&groups);
    step_cluster(&mut registry, &groups[0], &before[0], MoveStep::Up);
    step_cluster(&mut registry, &groups[0], &before[2], MoveStep::Down);
    assert_eq!(registry, ClusterRegistry::default());
    step_cluster(&mut registry, &groups[0], &before[1], MoveStep::Up);
    let groups = reordered(&registry, &file);
    assert_eq!(labels(&groups[0]), ["prod-b", "prod-a", "prod-c"]);
    step_cluster(&mut registry, &groups[0], &before[1], MoveStep::Down);
    let groups = reordered(&registry, &file);
    assert_eq!(labels(&groups[0]), ["prod-a", "prod-b", "prod-c"]);
}

#[test]
fn shortcut_numbers_follow_the_new_order() {
    use crate::cluster_health::HealthBoard;
    use crate::cluster_switcher_rows::{nth_cluster, switcher_sections};
    let file = kubeconfig("a.yaml", &["prod-a", "prod-b", "prod-c"]);
    let mut registry = ClusterRegistry::default();
    let groups = reordered(&registry, &file);
    move_cluster(
        &mut registry,
        &groups[0],
        &cluster("prod-c", "a.yaml"),
        &cluster("prod-a", "a.yaml"),
    );
    let groups = reordered(&registry, &file);
    let sections = switcher_sections(&groups, &HealthBoard::default(), &[]);
    assert_eq!(
        nth_cluster(&sections, 1),
        Some(&cluster("prod-c", "a.yaml"))
    );
    assert_eq!(
        nth_cluster(&sections, 3),
        Some(&cluster("prod-b", "a.yaml"))
    );
}

#[test]
fn cluster_search_matches_label_context_env_and_file() {
    let file = kubeconfig("team-one.yaml", &["prod-eu-1", "stg-us"]);
    let rows = rows_of(groups_of(&[&file], &ClusterRegistry::default(), &[], None));
    let matches = |text: &str| -> Vec<&str> {
        rows.iter()
            .filter(|row| cluster_matches(row, text))
            .map(|row| row.cluster.context.as_str())
            .collect()
    };
    assert_eq!(matches("prod-eu"), ["prod-eu-1"]);
    // Whitespace is ignored, as in the switcher.
    assert_eq!(matches("p r o d - e u"), ["prod-eu-1"]);
    assert_eq!(matches("STG"), ["stg-us"]);
    assert_eq!(matches("team-one"), ["prod-eu-1", "stg-us"]);
    assert_eq!(matches("PROD"), ["prod-eu-1"]);
    assert!(matches("xyz").is_empty());
    assert_eq!(matches("").len(), 2);
}

#[test]
fn filter_groups_keeps_matching_rows_and_drops_empty_groups() {
    let file = kubeconfig("a.yaml", &["prod-a", "prod-b", "stg-a"]);
    let groups = groups_of(&[&file], &ClusterRegistry::default(), &[], None);
    let found = filter_groups(&groups, "prod-b");
    assert_eq!(titles(&found), ["Production"]);
    assert_eq!(labels(&found[0]), ["prod-b"]);
    assert!(filter_groups(&groups, "zzz").is_empty());
    assert_eq!(filter_groups(&groups, "").len(), 2);
}

// ---- Spec 0043 step 4: the proxy control ----

#[test]
fn proxy_form_rejects_credentials() {
    let Err(error) = validate_proxy_url("http://user:pw@proxy:3128") else {
        panic!("userinfo must be refused");
    };
    assert_eq!(
        error.0,
        "Leave out the user name and password: k8sBoard does not store proxy credentials. Put them in the kubeconfig proxy-url instead."
    );
    assert!(!error.0.contains("pw"));
}

#[test]
fn proxy_form_messages_follow_the_rule() {
    let message = |text: &str| validate_proxy_url(text).expect_err("rejected").0;
    assert_eq!(message("https://p:1"), "Use http:// or socks5://.");
    assert_eq!(message("http://:1"), "Add the proxy host.");
    assert_eq!(message("http://p:0"), "Use a port from 1 to 65535.");
    assert_eq!(
        message("http://p/x"),
        "Remove the path; a proxy URL is scheme://host:port."
    );
    assert_eq!(
        message("http://p q"),
        "Enter a URL such as http://proxy.example:3128."
    );
}

#[test]
fn a_valid_proxy_is_stored_without_its_case_or_slash() {
    assert_eq!(
        validate_proxy_url("  HTTP://Proxy:3128/ "),
        Ok(Some(ClusterProxy::Url("http://Proxy:3128".to_owned())))
    );
    assert_eq!(validate_proxy_url("   "), Ok(None));
}

#[test]
fn the_proxy_control_shows_the_stored_choice() {
    let custom = ClusterProxy::Url("http://p:3128".to_owned());
    assert_eq!(proxy_mode(None, false), ProxyMode::FromKubeconfig);
    assert_eq!(
        proxy_mode(Some(&ClusterProxy::Direct), false),
        ProxyMode::Direct
    );
    assert_eq!(proxy_mode(Some(&custom), false), ProxyMode::Custom);
    // Picking Custom shows the input before anything is stored.
    assert_eq!(proxy_mode(None, true), ProxyMode::Custom);
    assert_eq!(
        proxy_mode_label(ProxyMode::FromKubeconfig, Some("http://k:1")),
        "From kubeconfig (http://k:1)"
    );
    assert_eq!(
        proxy_mode_label(ProxyMode::FromKubeconfig, None),
        "From kubeconfig (none)"
    );
    assert_eq!(proxy_mode_label(ProxyMode::Direct, None), "None (direct)");
    assert_eq!(proxy_mode_label(ProxyMode::Custom, None), "Custom URL");
}

#[test]
fn proxy_input_shows_only_a_parsable_value() {
    let stored = |text: &str| ClusterProxy::Url(text.to_owned());
    assert_eq!(
        proxy_input_prefill(Some(&stored("HTTP://p:3128/"))),
        "http://p:3128"
    );
    // A hand-edited value with userinfo is never echoed.
    assert_eq!(proxy_input_prefill(Some(&stored("http://u:p@x"))), "");
    assert_eq!(proxy_input_prefill(Some(&ClusterProxy::Direct)), "");
    assert_eq!(proxy_input_prefill(None), "");
}

#[test]
fn not_applied_shows_until_the_typed_url_is_stored() {
    let stored = ClusterProxy::Url("http://p:3128".to_owned());
    assert!(is_proxy_pending(None, "http://p:3128"));
    assert!(is_proxy_pending(None, ""));
    assert!(!is_proxy_pending(Some(&stored), "http://p:3128"));
    assert!(!is_proxy_pending(Some(&stored), "HTTP://p:3128/"));
    assert!(is_proxy_pending(Some(&stored), "http://other:1"));
    assert!(is_proxy_pending(Some(&stored), "not a url"));
    let unparsable = ClusterProxy::Url("http://u:p@x".to_owned());
    assert!(is_proxy_pending(Some(&unparsable), ""));
}

#[test]
fn folder_rows_cannot_be_removed() {
    let file = kubeconfig("watched/a.yaml", &["prod-1"]);
    let groups = cluster_groups(
        &[&file],
        &ClusterRegistry::default(),
        |_| false,
        |path| (path == Path::new("watched/a.yaml")).then(|| PathBuf::from("watched")),
        None,
    );
    let row = &groups[0].rows[0];
    assert_eq!(row.origin, RowOrigin::Folder);
    assert_eq!(
        remove_block_reason(row, Some(Path::new("watched"))).as_deref(),
        Some("Comes from the watched folder watched; stop watching it or delete the file.")
    );
    // The dialog text for it never promises a file change.
    let (_, body) = remove_dialog_text(
        Path::new("watched/a.yaml"),
        std::slice::from_ref(row),
        row.origin,
    );
    assert!(body.ends_with("The file itself is not changed."), "{body}");
}

#[test]
fn only_chain_and_folder_rows_are_blocked_from_removal() {
    let file = kubeconfig("a.yaml", &["prod-1"]);
    let groups = groups_of(&[&file], &ClusterRegistry::default(), &["a.yaml"], None);
    let chain = &groups[0].rows[0];
    assert_eq!(
        remove_block_reason(chain, None).as_deref(),
        Some("Comes from KUBECONFIG or ~/.kube/config; edit that instead.")
    );
    let groups = groups_of(&[&file], &ClusterRegistry::default(), &[], None);
    assert_eq!(remove_block_reason(&groups[0].rows[0], None), None);
}

#[test]
fn watched_folders_are_added_once_and_stopped_without_touching_clusters() {
    let mut registry = ClusterRegistry {
        clusters: vec![entry("one", "a.yaml")],
        ..ClusterRegistry::default()
    };
    assert!(add_watched_folder(&mut registry, PathBuf::from("watched")));
    assert!(!add_watched_folder(
        &mut registry,
        PathBuf::from("watched/")
    ));
    assert!(!add_watched_folder(
        &mut registry,
        PathBuf::from("./watched")
    ));
    assert_eq!(registry.kubeconfig_folders.len(), 1);
    stop_watching_folder(&mut registry, Path::new("watched"));
    assert!(registry.kubeconfig_folders.is_empty());
    // The overrides of a vanished file stay (0024 open item 4).
    assert_eq!(registry.clusters, [entry("one", "a.yaml")]);
}

#[test]
fn a_folder_row_says_what_its_file_reads_and_runs() {
    let info = |files: &[&str], exec: Option<&str>| ConnectionInfo {
        server: None,
        auth: cluster::AuthKind::None,
        proxy: None,
        credential_files: files.iter().map(|file| (*file).to_owned()).collect(),
        exec_command: exec.map(str::to_owned),
    };
    let folder = Path::new("watched");
    assert_eq!(
        folder_trust_note(folder, &info(&["/etc/secret/token", "/k/c.pem"], None)),
        "From watched folder watched: reads /etc/secret/token, /k/c.pem"
    );
    assert_eq!(
        folder_trust_note(folder, &info(&[], Some("/opt/bin/aws-iam"))),
        "From watched folder watched: runs /opt/bin/aws-iam"
    );
    assert_eq!(
        folder_trust_note(folder, &info(&["/t"], Some("aws"))),
        "From watched folder watched: reads /t / runs aws"
    );
    assert_eq!(
        folder_trust_note(folder, &info(&[], None)),
        "From watched folder watched: reads no credential file, runs no command"
    );
}

#[test]
fn only_folder_rows_carry_the_trust_note() {
    let yaml = "\
clusters:
  - name: c
    cluster: { server: 'https://127.0.0.1:1' }
users:
  - name: u
    user: { tokenFile: /etc/secret/token }
contexts:
  - name: prod-1
    context: { cluster: c, user: u }
";
    let file = Kubeconfig::parse(yaml, Path::new("watched/a.yaml")).expect("parses");
    let note_of = |is_folder: bool| {
        let groups = cluster_groups(
            &[&file],
            &ClusterRegistry::default(),
            |_| false,
            |_| is_folder.then(|| PathBuf::from("watched")),
            None,
        );
        groups[0].rows[0].trust_note.clone()
    };
    let note = note_of(true).expect("a folder row has a note");
    assert!(
        note.starts_with("From watched folder watched: reads /etc/secret/token"),
        "{note}"
    );
    assert_eq!(note_of(false), None);
}

#[test]
fn reset_clears_metrics() {
    let mut registry = ClusterRegistry::default();
    let target = ClusterRef {
        kubeconfig: PathBuf::from("a.yaml"),
        context: "one".to_owned(),
    };
    edit_entry(&mut registry, &target, |entry| {
        entry.metrics = Some(crate::cluster_registry::StoredMetrics::Fields(
            cluster::MetricsSourceFields {
                namespace: "monitoring".to_owned(),
                service: "vmselect".to_owned(),
                port: "8481".to_owned(),
                scheme: cluster::MetricsScheme::Http,
                prefix: String::new(),
            },
        ));
    });
    assert_eq!(registry.clusters.len(), 1, "a metrics-only entry is kept");
    reset_entry(&mut registry, &target);
    assert!(registry.clusters.is_empty());
}
