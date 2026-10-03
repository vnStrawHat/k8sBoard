use std::error::Error;

use super::*;

const FIXTURE: &str = include_str!("../tests/fixtures/kubeconfig.yaml");

fn from_yaml(yaml: &str) -> Kubeconfig {
    let document = kube::config::Kubeconfig::from_yaml(yaml).expect("test YAML parses");
    Kubeconfig::from_document(vec!["fixture.yaml".into()], document, &HashMap::new())
}

fn fixture() -> Kubeconfig {
    from_yaml(FIXTURE)
}

fn names(kubeconfig: &Kubeconfig) -> Vec<&str> {
    kubeconfig
        .contexts()
        .iter()
        .map(|context| context.name.as_str())
        .collect()
}

fn with_current_context(value: &str) -> Kubeconfig {
    let mut document = kube::config::Kubeconfig::from_yaml(FIXTURE).expect("fixture parses");
    document.current_context = Some(value.to_owned());
    Kubeconfig::from_document(vec!["fixture.yaml".into()], document, &HashMap::new())
}

#[test]
fn contexts_list_usable_contexts_in_file_order() {
    assert_eq!(names(&fixture()), ["alpha", "beta"]);
}

#[test]
fn context_summary_carries_cluster_user_and_namespace() {
    let kubeconfig = fixture();
    assert_eq!(
        kubeconfig.contexts()[0],
        ContextSummary {
            name: "alpha".to_owned(),
            cluster: "alpha".to_owned(),
            user: Some("alpha-user".to_owned()),
            namespace: Some("team-a".to_owned()),
            source: PathBuf::from("fixture.yaml"),
        }
    );
    assert_eq!(kubeconfig.contexts()[1].user.as_deref(), Some("beta-user"));
    assert_eq!(kubeconfig.contexts()[1].namespace, None);
}

#[test]
fn contexts_keep_first_of_duplicate_names() {
    let yaml = "\
contexts:
  - name: dup
    context: { cluster: first }
  - name: dup
    context: { cluster: second }
";
    let kubeconfig = from_yaml(yaml);
    assert_eq!(kubeconfig.contexts().len(), 1);
    assert_eq!(kubeconfig.contexts()[0].cluster, "first");
}

#[test]
fn current_context_returns_raw_value_even_if_missing() {
    assert_eq!(fixture().current_context(), Some("missing-context"));
}

#[test]
fn current_context_treats_empty_value_as_unset() {
    assert_eq!(with_current_context("").current_context(), None);
}

#[test]
fn resolve_context_prefers_requested_name() {
    let kubeconfig = with_current_context("alpha");
    let resolved = kubeconfig
        .resolve_context(Some("beta"))
        .expect("beta exists");
    assert_eq!(resolved.name, "beta");
}

#[test]
fn resolve_context_falls_back_to_current_context() {
    let kubeconfig = with_current_context("alpha");
    let resolved = kubeconfig.resolve_context(None).expect("alpha is current");
    assert_eq!(resolved.name, "alpha");
}

#[test]
fn resolve_context_reports_unknown_requested_context() {
    let error = fixture()
        .resolve_context(Some("nope"))
        .expect_err("unknown");
    let KubeconfigError::ContextNotFound {
        requested,
        origin,
        available,
        ..
    } = error
    else {
        panic!("expected ContextNotFound, got {error:?}");
    };
    assert_eq!(requested, "nope");
    assert_eq!(origin, ContextOrigin::Requested);
    assert_eq!(available, ["alpha", "beta"]);
}

#[test]
fn resolve_context_reports_invalid_current_context() {
    let error = fixture()
        .resolve_context(None)
        .expect_err("invalid current-context");
    let KubeconfigError::ContextNotFound {
        requested,
        origin,
        available,
        ..
    } = error
    else {
        panic!("expected ContextNotFound, got {error:?}");
    };
    assert_eq!(requested, "missing-context");
    assert_eq!(origin, ContextOrigin::CurrentContext);
    assert_eq!(available, ["alpha", "beta"]);
}

#[test]
fn resolve_context_without_current_context_reports_no_context_selected() {
    let kubeconfig = with_current_context("");
    let error = kubeconfig
        .resolve_context(None)
        .expect_err("nothing selected");
    let KubeconfigError::NoContextSelected { available, .. } = error else {
        panic!("expected NoContextSelected, got {error:?}");
    };
    assert_eq!(available, ["alpha", "beta"]);
}

#[test]
fn context_not_found_message_names_context_and_lists_available() {
    let message = fixture()
        .resolve_context(None)
        .expect_err("invalid current-context")
        .to_string();
    assert!(
        message.contains("current-context 'missing-context'"),
        "{message}"
    );
    assert!(message.contains("alpha, beta"), "{message}");
}

#[test]
fn context_list_renders_none_when_empty() {
    assert_eq!(context_list(&[]), "(none)");
}

#[test]
fn debug_output_never_contains_credentials() {
    let debug = format!("{:?}", fixture());
    assert!(!debug.contains("do-not-print"), "{debug}");
    assert!(debug.contains("alpha"), "{debug}");
}

#[test]
fn parse_error_does_not_quote_file_content() {
    let broken = "token: fixture-token-do-not-print\n  : [unbalanced";
    let kube_error = kube::config::Kubeconfig::from_yaml(broken).expect_err("broken YAML");
    let error = load_error(Path::new("broken.yaml"), kube_error);
    assert!(matches!(error, KubeconfigError::Parse { .. }), "{error:?}");

    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    assert!(!text.contains("do-not-print"), "{text}");
}

#[test]
fn has_proxy_url_detects_only_explicit_cluster_proxy() {
    let yaml = "\
clusters:
  - name: proxied
    cluster: { server: 'https://127.0.0.1:1', proxy-url: 'socks5://127.0.0.1:2' }
  - name: direct
    cluster: { server: 'https://127.0.0.1:1' }
";
    let kubeconfig = from_yaml(yaml);
    assert!(kubeconfig.has_proxy_url("proxied"));
    assert!(!kubeconfig.has_proxy_url("direct"));
    assert!(!kubeconfig.has_proxy_url("missing"));
}

fn file_yaml(context: &str, cluster: &str, current: Option<&str>) -> String {
    let current = current
        .map(|name| format!("current-context: {name}\n"))
        .unwrap_or_default();
    format!(
        "apiVersion: v1\nkind: Config\n{current}clusters:\n  - name: {cluster}\n    cluster: {{ server: 'https://127.0.0.1:1' }}\ncontexts:\n  - name: {context}\n    context: {{ cluster: {cluster} }}\n"
    )
}

/// A fresh temp dir per test; the contents hold no credentials.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k8sboard-0024-kc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write fixture");
    path
}

fn load_ok(paths: &[PathBuf]) -> LoadedKubeconfig {
    Kubeconfig::load(paths).expect("at least one file loads")
}

#[test]
fn load_merges_contexts_of_all_files_in_order() {
    let dir = temp_dir("merge");
    let a = write_file(&dir, "a.yaml", &file_yaml("one", "c1", None));
    let b = write_file(&dir, "b.yaml", &file_yaml("two", "c2", None));
    let loaded = load_ok(&[a.clone(), b.clone()]);
    assert_eq!(names(&loaded.kubeconfig), ["one", "two"]);
    assert_eq!(loaded.kubeconfig.sources(), [a, b]);
    assert!(loaded.skipped.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn first_file_wins_for_duplicate_context_names() {
    let dir = temp_dir("dup");
    let a = write_file(&dir, "a.yaml", &file_yaml("same", "first", None));
    let b = write_file(&dir, "b.yaml", &file_yaml("same", "second", None));
    let loaded = load_ok(&[a.clone(), b]);
    let contexts = loaded.kubeconfig.contexts();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].cluster, "first");
    assert_eq!(contexts[0].source, a);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn current_context_comes_from_the_first_file_that_sets_it() {
    let dir = temp_dir("current");
    let a = write_file(&dir, "a.yaml", &file_yaml("one", "c1", None));
    let b = write_file(&dir, "b.yaml", &file_yaml("two", "c2", Some("two")));
    let c = write_file(&dir, "c.yaml", &file_yaml("three", "c3", Some("three")));
    let loaded = load_ok(&[a, b, c]);
    assert_eq!(loaded.kubeconfig.current_context(), Some("two"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn context_source_is_the_defining_file() {
    let dir = temp_dir("source");
    let a = write_file(&dir, "a.yaml", &file_yaml("one", "c1", None));
    let b = write_file(&dir, "b.yaml", &file_yaml("two", "c2", None));
    let loaded = load_ok(&[a.clone(), b.clone()]);
    let sources: Vec<&Path> = loaded
        .kubeconfig
        .contexts()
        .iter()
        .map(|context| context.source.as_path())
        .collect();
    assert_eq!(sources, [a.as_path(), b.as_path()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unreadable_file_is_skipped_and_reported() {
    let dir = temp_dir("unreadable");
    let a = write_file(&dir, "a.yaml", &file_yaml("one", "c1", None));
    let missing = dir.join("missing.yaml");
    let loaded = load_ok(&[a.clone(), missing]);
    assert_eq!(loaded.kubeconfig.sources(), [a]);
    assert!(matches!(
        loaded.skipped.as_slice(),
        [KubeconfigError::Read { .. }]
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn incompatible_file_is_skipped() {
    let dir = temp_dir("incompatible");
    let a = write_file(&dir, "a.yaml", &file_yaml("one", "c1", None));
    let other = file_yaml("two", "c2", None).replace("kind: Config", "kind: Other");
    let b = write_file(&dir, "b.yaml", &other);
    let loaded = load_ok(&[a, b.clone()]);
    assert_eq!(names(&loaded.kubeconfig), ["one"]);
    assert!(
        matches!(loaded.skipped.as_slice(), [KubeconfigError::Incompatible { path }] if *path == b),
        "{:?}",
        loaded.skipped
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn incompatible_file_keeps_earlier_files() {
    let dir = temp_dir("keeps");
    let a = write_file(&dir, "a.yaml", &file_yaml("one", "c1", None));
    let other = file_yaml("two", "c2", None).replace("apiVersion: v1", "apiVersion: v2");
    let b = write_file(&dir, "b.yaml", &other);
    let c = write_file(&dir, "c.yaml", &file_yaml("three", "c3", None));
    let loaded = load_ok(&[a, b, c]);
    assert_eq!(names(&loaded.kubeconfig), ["one", "three"]);
    assert_eq!(loaded.skipped.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_fails_when_no_file_loads() {
    let missing = PathBuf::from("definitely-missing-kubeconfig.yaml");
    let error = Kubeconfig::load(&[missing, PathBuf::from("other-missing.yaml")])
        .expect_err("no file loads");
    assert!(matches!(error, KubeconfigError::Read { .. }), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("definitely-missing-kubeconfig.yaml"),
        "{error}"
    );
    assert!(matches!(
        Kubeconfig::load(&[]),
        Err(KubeconfigError::NoFiles)
    ));
}

#[test]
fn context_not_found_lists_every_source() {
    let dir = temp_dir("not-found");
    let a = write_file(&dir, "a.yaml", &file_yaml("one", "c1", None));
    let b = write_file(&dir, "b.yaml", &file_yaml("two", "c2", None));
    let loaded = load_ok(&[a, b]);
    let message = loaded
        .kubeconfig
        .resolve_context(Some("nope"))
        .expect_err("unknown")
        .to_string();
    assert!(message.contains("a.yaml, "), "{message}");
    assert!(message.contains("b.yaml"), "{message}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- parse, connection info, entry names (spec 0025) ----

const TOKEN_FIXTURE: &str = "fixture-token-value";

/// One cluster plus one context per user entry, so each user shape has its own context.
fn kubeconfig_with_users(users: &str) -> Kubeconfig {
    let yaml = format!(
        "clusters:\n  - name: c\n    cluster: {{ server: 'https://u:p@h:6443/x?y' }}\nusers:\n{users}contexts:\n  - name: ctx\n    context: {{ cluster: c, user: u }}\n"
    );
    Kubeconfig::parse(&yaml, Path::new("fixture.yaml")).expect("fixture parses")
}

fn auth_of(users: &str) -> AuthKind {
    let kubeconfig = kubeconfig_with_users(users);
    kubeconfig.connection_info(&kubeconfig.contexts()[0]).auth
}

#[test]
fn parse_reads_contexts_from_text() {
    let kubeconfig =
        Kubeconfig::parse(FIXTURE, Path::new("clipboard")).expect("fixture text parses");
    assert_eq!(names(&kubeconfig), ["alpha", "beta"]);
    assert!(
        kubeconfig
            .contexts()
            .iter()
            .all(|context| context.source == Path::new("clipboard"))
    );
}

#[test]
fn parse_merges_multiple_documents() {
    let yaml = format!(
        "{}---\n{}",
        file_yaml("one", "c1", None),
        file_yaml("two", "c2", None)
    );
    let kubeconfig = Kubeconfig::parse(&yaml, Path::new("clipboard")).expect("both documents");
    assert_eq!(names(&kubeconfig), ["one", "two"]);
}

#[test]
fn parse_error_does_not_quote_the_text() {
    let broken = format!("token: {TOKEN_FIXTURE}\n  : [unbalanced");
    let error = Kubeconfig::parse(&broken, Path::new("clipboard")).expect_err("broken YAML");
    assert!(matches!(error, KubeconfigError::Parse { .. }), "{error:?}");
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    assert!(!text.contains(TOKEN_FIXTURE), "{text}");
}

#[test]
fn server_drops_userinfo_path_and_query() {
    let kubeconfig = kubeconfig_with_users("  - name: u\n    user: {}\n");
    let info = kubeconfig.connection_info(&kubeconfig.contexts()[0]);
    assert_eq!(info.server.as_deref(), Some("https://h:6443"));
}

#[test]
fn display_server_table() {
    let cases = [
        ("https://u:p@h:6443/x?y", "https://h:6443"),
        ("https://h:6443", "https://h:6443"),
        ("https://h:6443/k8s/clusters/c-1?x=1#f", "https://h:6443"),
        ("https://u:p@ss@h:6443/x", "https://h:6443"),
        ("h:6443/x", "h:6443"),
        // A raw password with a delimiter: no part of it may show, so no host either.
        ("https://u:pa/ss@h:6443", "https://…"),
        ("https://u:pa?ss@h:6443/x", "https://…"),
        ("https://u:pa#ss@h:6443", "https://…"),
        ("https://h:6443/a@b", "https://…"),
        ("u:pa/ss@h:6443", "…"),
    ];
    for (server, shown) in cases {
        assert_eq!(display_server(server), shown, "{server}");
    }
    for server in [
        "https://u:pa/ss@h:6443",
        "https://u:pa?ss@h:6443",
        "https://u:pa#ss@h:6443",
    ] {
        assert!(!display_server(server).contains("pa"), "{server}");
    }
}

#[test]
fn server_is_none_without_a_cluster_entry() {
    let kubeconfig = from_yaml("contexts:\n  - name: ctx\n    context: { cluster: missing }\n");
    let info = kubeconfig.connection_info(&kubeconfig.contexts()[0]);
    assert_eq!(info.server, None);
    assert_eq!(info.auth, AuthKind::None);
}

#[test]
fn auth_kind_per_user_shape() {
    let cases = [
        (
            "  - name: u\n    user:\n      exec: { command: aws, apiVersion: client.authentication.k8s.io/v1 }\n",
            AuthKind::Exec {
                command: "aws".to_owned(),
            },
        ),
        (
            "  - name: u\n    user:\n      auth-provider: { name: gcp }\n",
            AuthKind::AuthProvider {
                name: "gcp".to_owned(),
            },
        ),
        (
            "  - name: u\n    user:\n      client-certificate: cert.pem\n      client-key: key.pem\n",
            AuthKind::ClientCertificate,
        ),
        (
            "  - name: u\n    user:\n      client-certificate-data: Zm9v\n      client-key-data: Zm9v\n",
            AuthKind::ClientCertificate,
        ),
        (
            "  - name: u\n    user:\n      token: fixture-token-value\n",
            AuthKind::Token,
        ),
        (
            "  - name: u\n    user:\n      tokenFile: /var/token\n",
            AuthKind::TokenFile,
        ),
        (
            "  - name: u\n    user:\n      username: fixture-user\n      password: fixture-password\n",
            AuthKind::Basic,
        ),
        ("  - name: u\n    user: {}\n", AuthKind::None),
    ];
    for (users, expected) in cases {
        assert_eq!(auth_of(users), expected, "{users}");
    }
}

#[test]
fn exec_kind_keeps_only_the_command_file_name() {
    let unix = "  - name: u\n    user:\n      exec:\n        command: /usr/local/bin/aws\n        args: [eks, get-token, --secret-arg-value]\n        env: [{ name: SECRET_ENV, value: secret-env-value }]\n";
    let windows = "  - name: u\n    user:\n      exec:\n        command: 'C:\x5ctools\x5caws.exe'\n        args: [--secret-arg-value]\n";
    for (users, expected) in [(unix, "exec: aws"), (windows, "exec: aws.exe")] {
        let kubeconfig = kubeconfig_with_users(users);
        let info = kubeconfig.connection_info(&kubeconfig.contexts()[0]);
        assert_eq!(info.auth.to_string(), expected);
        let text = format!("{info:?} {}", info.auth);
        assert!(!text.contains("secret-arg-value"), "{text}");
        assert!(!text.contains("secret-env-value"), "{text}");
    }
}

#[test]
fn entry_names_list_contexts_clusters_users() {
    let names = fixture().entry_names();
    assert_eq!(names.contexts, ["alpha", "beta"]);
    assert_eq!(names.clusters, ["alpha", "beta"]);
    assert_eq!(names.users, ["alpha-user", "beta-user"]);
}

#[test]
fn connection_info_debug_has_no_credentials() {
    let kubeconfig = kubeconfig_with_users(&format!(
        "  - name: u\n    user:\n      token: {TOKEN_FIXTURE}\n"
    ));
    let info = kubeconfig.connection_info(&kubeconfig.contexts()[0]);
    let text = format!("{info:?} {}", info.auth);
    assert!(!text.contains(TOKEN_FIXTURE), "{text}");
    assert_eq!(info.auth, AuthKind::Token);
}
