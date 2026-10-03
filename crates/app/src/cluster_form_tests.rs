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
        owned,
    )
}

fn rows_of(groups: Vec<ClusterGroup>) -> Vec<ClusterRow> {
    groups.into_iter().flat_map(|group| group.rows).collect()
}

fn titles(groups: &[ClusterGroup]) -> Vec<&str> {
    groups.iter().map(|group| group.title).collect()
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
        default_namespace: None,
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
    };
    remove_kubeconfig(&mut registry, Path::new("dir/sub/a.yaml"));
    assert_eq!(registry.kubeconfigs, [PathBuf::from("b.yaml")]);
    assert!(registry.clusters.is_empty());
    assert_eq!(registry.last_used, None);
}
