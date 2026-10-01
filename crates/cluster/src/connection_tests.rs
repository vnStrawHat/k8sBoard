use std::cell::RefCell;
use std::collections::VecDeque;

use futures::executor::block_on;
use kube::core::Status;

use super::*;

fn api_error(code: u16, message: &str) -> kube::Error {
    kube::Error::Api(Box::new(Status::failure(message, "Reason").with_code(code)))
}

fn api_cluster_error(code: u16) -> ClusterError {
    ClusterError::Api {
        context: "ctx".to_owned(),
        action: "listing things",
        code,
        message: "boom".to_owned(),
    }
}

fn page(items: &[i32], token: Option<&str>) -> Result<Page<i32>, ClusterError> {
    Ok(Page {
        items: items.to_vec(),
        continue_token: token.map(str::to_owned),
    })
}

/// Replays scripted responses and records the params of every request.
struct Script {
    responses: RefCell<VecDeque<Result<Page<i32>, ClusterError>>>,
    requests: RefCell<Vec<ListParams>>,
}

impl Script {
    fn new(responses: Vec<Result<Page<i32>, ClusterError>>) -> Self {
        Self {
            responses: RefCell::new(responses.into()),
            requests: RefCell::new(Vec::new()),
        }
    }

    fn collect(&self) -> Result<Vec<i32>, ClusterError> {
        block_on(collect_pages(|params| {
            self.requests.borrow_mut().push(params);
            let response = self.responses.borrow_mut().pop_front();
            async move { response.expect("script has a response for every request") }
        }))
    }
}

#[test]
fn api_status_401_maps_to_unauthorized() {
    let error = classify_error("ctx", "listing pods", api_error(401, "expired"));
    assert!(
        matches!(error, ClusterError::Unauthorized { .. }),
        "{error:?}"
    );
}

#[test]
fn api_status_403_maps_to_forbidden() {
    let error = classify_error("ctx", "listing pods", api_error(403, "pods is forbidden"));
    let ClusterError::Forbidden { message, .. } = error else {
        panic!("expected Forbidden, got {error:?}");
    };
    assert_eq!(message, "pods is forbidden");
}

#[test]
fn api_status_other_maps_to_api_with_code() {
    let error = classify_error("ctx", "listing pods", api_error(503, "unavailable"));
    let ClusterError::Api { code, message, .. } = error else {
        panic!("expected Api, got {error:?}");
    };
    assert_eq!(code, 503);
    assert_eq!(message, "unavailable");
}

#[test]
fn auth_error_maps_to_credentials_unavailable() {
    let auth = kube::Error::Auth(kube::client::AuthError::ExecPluginFailed);
    let error = classify_error("ctx", "listing pods", auth);
    assert!(
        matches!(error, ClusterError::CredentialsUnavailable { .. }),
        "{error:?}"
    );
    assert!(!error.to_string().contains("exec-plugin"), "{error}");
}

#[test]
fn cluster_error_message_names_context_and_action() {
    let message = classify_error("prod", "listing pods", api_error(403, "no access")).to_string();
    assert!(message.contains("prod"), "{message}");
    assert!(message.contains("listing pods"), "{message}");
    assert!(message.contains("no access"), "{message}");
}

#[test]
fn list_all_follows_continue_tokens_until_empty() {
    let script = Script::new(vec![
        page(&[1, 2], Some("t1")),
        page(&[3], Some("t2")),
        page(&[4], Some("")),
    ]);
    assert_eq!(script.collect().expect("pages collect"), [1, 2, 3, 4]);

    let requests = script.requests.borrow();
    let tokens: Vec<_> = requests
        .iter()
        .map(|params| params.continue_token.as_deref())
        .collect();
    assert_eq!(tokens, [None, Some("t1"), Some("t2")]);
    assert!(requests.iter().all(|params| params.limit == Some(500)));
    assert_eq!(LIST_PAGE_SIZE, 500);
}

#[test]
fn list_all_restarts_once_when_continue_token_expires() {
    let script = Script::new(vec![
        page(&[1, 2], Some("t1")),
        Err(api_cluster_error(410)),
        page(&[1, 2], Some("t1")),
        page(&[3], None),
    ]);
    assert_eq!(script.collect().expect("restart succeeds"), [1, 2, 3]);

    let requests = script.requests.borrow();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[2].continue_token, None);
}

#[test]
fn list_all_surfaces_second_expired_continue_token() {
    let script = Script::new(vec![
        page(&[1], Some("t1")),
        Err(api_cluster_error(410)),
        page(&[1], Some("t2")),
        Err(api_cluster_error(410)),
    ]);
    let error = script.collect().expect_err("second 410 is returned");
    assert!(
        matches!(error, ClusterError::Api { code: 410, .. }),
        "{error:?}"
    );
}

#[test]
fn list_all_does_not_retry_first_page_error() {
    let script = Script::new(vec![Err(api_cluster_error(410))]);
    let error = script.collect().expect_err("first page error is returned");
    assert!(
        matches!(error, ClusterError::Api { code: 410, .. }),
        "{error:?}"
    );
    assert_eq!(script.requests.borrow().len(), 1);
}

/// The full text a caller could print: `Display` and `Debug` of the error and every source.
fn visible_text(error: &ClusterError) -> String {
    let mut text = format!("{error} {error:?}");
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(&format!(" {cause} {cause:?}"));
        source = cause.source();
    }
    text
}

fn kubeconfig_with_proxy(proxy_url: &str) -> kube::config::Kubeconfig {
    let yaml = format!(
        "\
current-context: ctx
clusters:
  - name: c
    cluster: {{ server: 'https://127.0.0.1:1', proxy-url: '{proxy_url}' }}
users:
  - name: u
    user: {{ token: fixture-token-do-not-print }}
contexts:
  - name: ctx
    context: {{ cluster: c, user: u }}
"
    );
    kube::config::Kubeconfig::from_yaml(&yaml).expect("test YAML parses")
}

#[tokio::test]
async fn proxy_url_userinfo_never_appears_in_invalid_config_error() {
    for proxy_url in [
        "http://user:secret@proxy:3128",
        "socks5://user:secret@proxy:1080",
        "ftp://user:secret@proxy:21",
    ] {
        let options = KubeConfigOptions::default();
        let config =
            kube::Config::from_custom_kubeconfig(kubeconfig_with_proxy(proxy_url), &options)
                .await
                .expect("config builds");
        let Err(kube_error) = kube::Client::try_from(config) else {
            panic!("proxy support is disabled, so building the client must fail");
        };
        let text = visible_text(&invalid_config("ctx", kube_error));
        assert!(text.contains("proxy-url"), "{text}");
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("user:"), "{text}");
    }
}

#[test]
fn auth_error_from_client_build_is_redacted() {
    let error = invalid_config(
        "ctx",
        kube::Error::Auth(kube::client::AuthError::ExecPluginFailed),
    );
    let text = visible_text(&error);
    assert!(!text.contains("exec-plugin"), "{text}");
    assert!(!text.contains("ExecPluginFailed"), "{text}");
}

#[test]
fn proxy_is_kept_only_when_kubeconfig_sets_proxy_url() {
    assert_eq!(proxy_for(true, Some("proxy")), Some("proxy"));
    assert_eq!(proxy_for(false, Some("proxy")), None);
    assert_eq!(proxy_for::<&str>(true, None), None);
}
