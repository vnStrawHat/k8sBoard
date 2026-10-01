use std::future::Future;
use std::path::Path;

use cluster::{
    AccessReport, ClusterConnection, ClusterError, ContextOrigin, Kubeconfig, KubeconfigError,
    MetricsApi, NamespaceScope, NamespaceSummary, NodeSummary, PodSummary, ServerVersion,
};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/kubeconfig.yaml"
);

fn fixture() -> Kubeconfig {
    match Kubeconfig::load(Path::new(FIXTURE)) {
        Ok(kubeconfig) => kubeconfig,
        Err(error) => panic!("fixture kubeconfig must load: {error}"),
    }
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

fn assert_send<T: Send>(_future: T) {}

fn assert_send_future<F: Future + Send>(_future: &F) {}

#[tokio::test]
async fn open_fixture_context_succeeds_without_network() {
    let kubeconfig = fixture();
    let alpha = ClusterConnection::open(&kubeconfig, "alpha")
        .await
        .expect("alpha opens");
    assert_eq!(alpha.context(), "alpha");
    assert_eq!(alpha.default_namespace(), "team-a");

    let beta = ClusterConnection::open(&kubeconfig, "beta")
        .await
        .expect("beta opens");
    assert_eq!(beta.default_namespace(), "default");
}

#[tokio::test]
async fn open_unknown_context_returns_kubeconfig_error() {
    let error = ClusterConnection::open(&fixture(), "nope")
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

    let kubeconfig = fixture();
    let connection = ClusterConnection::open(&kubeconfig, "alpha")
        .await
        .expect("alpha opens");
    assert_send(ClusterConnection::open(&kubeconfig, "alpha"));
    // Created and dropped unpolled: no request is sent.
    assert_send_future(&connection.server_version());
    assert_send_future(&connection.list_namespaces());
    assert_send_future(&connection.list_nodes());
    assert_send_future(&connection.metrics_api());
    assert_send_future(&connection.list_pods(NamespaceScope::All));
    assert_send_future(&connection.review_access(NamespaceScope::All));
}

#[tokio::test]
async fn connection_debug_hides_credentials() {
    let connection = ClusterConnection::open(&fixture(), "alpha")
        .await
        .expect("alpha opens");
    let debug = format!("{connection:?}");
    assert!(!debug.contains("do-not-print"), "{debug}");
    assert!(debug.contains("alpha"), "{debug}");
}

#[tokio::test]
async fn server_version_against_closed_port_is_unreachable() {
    let connection = ClusterConnection::open(&fixture(), "alpha")
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
