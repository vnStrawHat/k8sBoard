use std::error::Error;

use super::*;

const FIXTURE: &str = include_str!("../tests/fixtures/kubeconfig.yaml");

fn from_yaml(yaml: &str) -> Kubeconfig {
    let document = kube::config::Kubeconfig::from_yaml(yaml).expect("test YAML parses");
    Kubeconfig::from_document(Path::new("fixture.yaml"), document)
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
    Kubeconfig::from_document(Path::new("fixture.yaml"), document)
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
fn load_reports_missing_file_with_path() {
    let error = Kubeconfig::load(Path::new("definitely-missing-kubeconfig.yaml"))
        .expect_err("missing file");
    assert!(matches!(error, KubeconfigError::Read { .. }), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("definitely-missing-kubeconfig.yaml"),
        "{error}"
    );
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
