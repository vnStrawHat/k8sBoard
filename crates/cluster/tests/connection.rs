use std::future::Future;
use std::path::PathBuf;

use cluster::{
    AccessReport, ClusterConnection, ClusterError, ContextOrigin, EnvValues, HelmReleaseDetail,
    HelmReleaseSummary, HelmRevisionRef, HelmValuesDiff, Kubeconfig, KubeconfigError, MetricsApi,
    NamespaceScope, NamespaceSummary, NodeSummary, PodSummary, ProxyChoice, ServerVersion,
    ValueVisibility,
};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/kubeconfig.yaml"
);

fn fixture() -> Kubeconfig {
    match Kubeconfig::load(&[PathBuf::from(FIXTURE)]) {
        Ok(loaded) => loaded.kubeconfig,
        Err(error) => panic!("fixture kubeconfig must load: {error}"),
    }
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

fn assert_send<T: Send>(_future: T) {}

fn assert_send_future<F: Future + Send>(_future: &F) {}

#[tokio::test]
async fn open_fixture_context_succeeds_without_network() {
    let kubeconfig = fixture();
    let alpha = ClusterConnection::open(&kubeconfig, "alpha", &ProxyChoice::Kubeconfig)
        .await
        .expect("alpha opens");
    assert_eq!(alpha.context(), "alpha");
    assert_eq!(alpha.default_namespace(), "team-a");

    let beta = ClusterConnection::open(&kubeconfig, "beta", &ProxyChoice::Kubeconfig)
        .await
        .expect("beta opens");
    assert_eq!(beta.default_namespace(), "default");
}

#[tokio::test]
async fn open_unknown_context_returns_kubeconfig_error() {
    let error = ClusterConnection::open(&fixture(), "nope", &ProxyChoice::Kubeconfig)
        .await
        .expect_err("unknown context");
    assert!(
        matches!(
            error,
            ClusterError::Kubeconfig(KubeconfigError::ContextNotFound {
                origin: ContextOrigin::Requested,
                ..
            })
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn connection_and_query_futures_are_send() {
    assert_send_sync_static::<Kubeconfig>();
    assert_send_sync_static::<ClusterConnection>();
    assert_send_sync_static::<ClusterError>();
    assert_send_sync_static::<ServerVersion>();
    assert_send_sync_static::<NamespaceSummary>();
    assert_send_sync_static::<NodeSummary>();
    assert_send_sync_static::<PodSummary>();
    assert_send_sync_static::<AccessReport>();
    assert_send_sync_static::<MetricsApi>();
    assert_send_sync_static::<HelmReleaseSummary>();
    assert_send_sync_static::<HelmReleaseDetail>();
    assert_send_sync_static::<HelmValuesDiff>();

    let kubeconfig = fixture();
    let connection = ClusterConnection::open(&kubeconfig, "alpha", &ProxyChoice::Kubeconfig)
        .await
        .expect("alpha opens");
    assert_send(ClusterConnection::open(
        &kubeconfig,
        "alpha",
        &ProxyChoice::Kubeconfig,
    ));
    // Created and dropped unpolled: no request is sent.
    assert_send_future(&connection.server_version());
    assert_send_future(&connection.list_namespaces());
    assert_send_future(&connection.list_nodes());
    assert_send_future(&connection.metrics_api());
    assert_send_future(&connection.list_pods(NamespaceScope::All));
    assert_send_future(&connection.review_access(NamespaceScope::All));
    let revision = HelmRevisionRef {
        namespace: "shop".to_owned(),
        release: "api".to_owned(),
        revision: 2,
    };
    assert_send_future(&connection.helm_release_detail(&revision, EnvValues::Hidden));
    assert_send_future(&connection.helm_revealed(&revision));
    assert_send_future(&connection.helm_values_diff(&revision, &revision, ValueVisibility::Masked));
}

#[tokio::test]
async fn connection_debug_hides_credentials() {
    let connection = ClusterConnection::open(&fixture(), "alpha", &ProxyChoice::Kubeconfig)
        .await
        .expect("alpha opens");
    let debug = format!("{connection:?}");
    assert!(!debug.contains("do-not-print"), "{debug}");
    assert!(debug.contains("alpha"), "{debug}");
}

#[tokio::test]
async fn server_version_against_closed_port_is_unreachable() {
    let connection = ClusterConnection::open(&fixture(), "alpha", &ProxyChoice::Kubeconfig)
        .await
        .expect("alpha opens");
    let error = connection
        .server_version()
        .await
        .expect_err("nothing listens on port 1");
    assert!(
        matches!(error, ClusterError::Unreachable { .. }),
        "{error:?}"
    );
}

/// Fakes only: a kubeconfig whose server is a local listener, and a proxy that refuses. The server
/// must never see a connection, so a bad proxy cannot fall back to a direct one, and nothing in
/// the error names a credential.
#[tokio::test]
async fn a_refusing_proxy_fails_closed_and_the_server_is_never_reached() {
    let server = std::net::TcpListener::bind("127.0.0.1:0").expect("a local server");
    server.set_nonblocking(true).expect("non-blocking");
    let port = server.local_addr().expect("an address").port();
    // A port that was free a moment ago: nothing listens on it.
    let refusing = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let proxy_port = refusing.local_addr().expect("an address").port();
    drop(refusing);
    let yaml = format!(
        "clusters:\n  - name: c\n    cluster: {{ server: \"http://127.0.0.1:{port}\" }}\nusers:\n  - name: u\n    user: {{ token: fixture-token-do-not-print }}\ncontexts:\n  - name: ctx\n    context: {{ cluster: c, user: u }}\n"
    );
    let kubeconfig =
        Kubeconfig::parse(&yaml, std::path::Path::new("fixture.yaml")).expect("parses");
    let proxy = ProxyChoice::Url(
        cluster::ProxyUrl::parse(&format!("http://127.0.0.1:{proxy_port}")).expect("valid"),
    );
    let connection = ClusterConnection::open(&kubeconfig, "ctx", &proxy)
        .await
        .expect("the client builds");
    let error = connection
        .server_version()
        .await
        .expect_err("the proxy refuses the connection");
    let mut text = format!("{error} {error:?}");
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        text.push_str(&format!(" {cause} {cause:?}"));
        source = cause.source();
    }
    assert!(!text.contains("fixture-token-do-not-print"), "{text}");
    // The attempt went to the proxy; the server saw nothing.
    assert!(
        matches!(server.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "the API server must not be contacted when the proxy fails"
    );
}
