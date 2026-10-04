use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::cluster_form::RowOrigin;

fn cluster(context: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("a.yaml"),
        context: context.to_owned(),
    }
}

fn candidate(context: &str, auth: AuthKind, is_active: bool) -> ProbeCandidate {
    from_origin(context, auth, is_active, RowOrigin::Registry)
}

fn from_origin(
    context: &str,
    auth: AuthKind,
    is_active: bool,
    origin: RowOrigin,
) -> ProbeCandidate {
    ProbeCandidate {
        cluster: cluster(context),
        auth,
        is_active,
        origin,
    }
}

fn reachable(millis: u64) -> ProbeResult {
    ProbeResult::Reachable {
        latency: Duration::from_millis(millis),
    }
}

fn unreachable() -> ProbeResult {
    ProbeResult::Unreachable {
        reason: "refused".to_owned(),
    }
}

#[test]
fn exec_and_auth_provider_are_not_auto_probed() {
    let exec = AuthKind::Exec {
        command: "aws".to_owned(),
    };
    let provider = AuthKind::AuthProvider {
        name: "gcp".to_owned(),
    };
    assert!(!is_probed_automatically(&exec, RowOrigin::Registry));
    assert!(!is_probed_automatically(&provider, RowOrigin::Registry));
    for auth in [
        AuthKind::Token,
        AuthKind::TokenFile,
        AuthKind::ClientCertificate,
        AuthKind::Basic,
        AuthKind::None,
    ] {
        assert!(
            is_probed_automatically(&auth, RowOrigin::Registry),
            "{auth}"
        );
    }
}

#[test]
fn due_skips_active_running_and_fresh() {
    let now = Instant::now();
    let mut board = HealthBoard::default();
    board.record(cluster("fresh"), reachable(5), now);
    board.mark_running(&[cluster("busy")]);
    let rows = [
        candidate("active", AuthKind::Token, true),
        candidate("busy", AuthKind::Token, false),
        candidate("fresh", AuthKind::Token, false),
        candidate("new", AuthKind::Token, false),
        candidate(
            "exec",
            AuthKind::Exec {
                command: "aws".to_owned(),
            },
            false,
        ),
    ];
    assert_eq!(board.due(&rows, now), [cluster("new")]);
}

#[test]
fn stale_entry_is_due_again() {
    let now = Instant::now();
    let mut board = HealthBoard::default();
    board.record(cluster("a"), reachable(5), now);
    let rows = [candidate("a", AuthKind::Token, false)];
    let just_inside = now + PROBE_TTL - Duration::from_secs(1);
    assert!(board.due(&rows, just_inside).is_empty());
    assert_eq!(
        board.due(&rows, now + Duration::from_secs(61)),
        [cluster("a")]
    );
}

#[test]
fn record_clears_running() {
    let now = Instant::now();
    let mut board = HealthBoard::default();
    board.mark_running(&[cluster("a")]);
    assert!(board.is_running(&cluster("a")));
    board.record(cluster("a"), unreachable(), now);
    assert!(!board.is_running(&cluster("a")));
}

#[test]
fn clear_running_makes_aborted_rows_due() {
    let now = Instant::now();
    let mut board = HealthBoard::default();
    let rows = [candidate("a", AuthKind::Token, false)];
    board.mark_running(&[cluster("a")]);
    assert!(board.due(&rows, now).is_empty());
    board.clear_running();
    assert_eq!(board.due(&rows, now), [cluster("a")]);
}

#[test]
fn retry_on_running_row_is_noop() {
    let mut board = HealthBoard::default();
    board.mark_running(&[cluster("a")]);
    // The shell asks `is_running` before it starts a one-row probe.
    assert!(board.is_running(&cluster("a")));
    assert_eq!(board.row_health(&cluster("a")), RowHealth::Checking);
}

#[test]
fn row_health_maps_results() {
    let now = Instant::now();
    let mut board = HealthBoard::default();
    board.record(cluster("up"), reachable(42), now);
    board.record(cluster("down"), unreachable(), now);
    board.mark_running(&[cluster("busy")]);
    assert_eq!(
        board.row_health(&cluster("up")),
        RowHealth::Reachable(Duration::from_millis(42))
    );
    assert_eq!(board.row_health(&cluster("down")), RowHealth::Unreachable);
    assert_eq!(board.row_health(&cluster("busy")), RowHealth::Checking);
    assert_eq!(board.row_health(&cluster("new")), RowHealth::NotChecked);
}

#[test]
fn a_running_reprobe_shows_checking_over_an_old_answer() {
    let now = Instant::now();
    let mut board = HealthBoard::default();
    board.record(cluster("a"), unreachable(), now);
    board.mark_running(&[cluster("a")]);
    assert_eq!(board.row_health(&cluster("a")), RowHealth::Checking);
}

#[test]
fn failure_reason_is_kept_for_unreachable_rows_only() {
    let now = Instant::now();
    let mut board = HealthBoard::default();
    board.record(cluster("down"), unreachable(), now);
    board.record(cluster("up"), reachable(1), now);
    assert_eq!(board.failure_reason(&cluster("down")), Some("refused"));
    assert_eq!(board.failure_reason(&cluster("up")), None);
    assert_eq!(board.failure_reason(&cluster("new")), None);
}

fn target(context: &str) -> ProbeTarget {
    let yaml = "\
clusters:
  - name: c
    cluster: { server: 'https://127.0.0.1:1' }
contexts:
  - name: ctx
    context: { cluster: c }
";
    ProbeTarget {
        cluster: cluster(context),
        kubeconfig: Arc::new(Kubeconfig::parse(yaml, Path::new("a.yaml")).expect("parses")),
        context: "ctx".to_owned(),
        proxy: Ok(cluster::ProxyChoice::Kubeconfig),
    }
}

#[tokio::test]
async fn probe_all_limits_concurrency() {
    static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    let targets: Vec<_> = (0..12).map(|index| target(&format!("c{index}"))).collect();
    let probe = |_: ProbeTarget| async {
        let now = IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
        PEAK.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(15)).await;
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        reachable(1)
    };
    let results: Vec<_> = probe_all(targets, probe).collect().await;
    assert_eq!(results.len(), 12);
    let peak = PEAK.load(Ordering::SeqCst);
    assert!(peak <= PROBE_CONCURRENCY, "peak {peak}");
    assert!(peak > 1, "the probes never overlapped: peak {peak}");
}

#[tokio::test]
async fn probe_timeout_is_unreachable() {
    // Real time: the crate does not enable tokio's paused clock.
    let result = bounded(
        Duration::from_millis(20),
        std::future::pending::<Result<Duration, ClusterError>>(),
    )
    .await;
    assert!(
        matches!(result, ProbeResult::Unreachable { .. }),
        "{result:?}"
    );
}

#[tokio::test]
async fn an_unreachable_server_is_unreachable_without_credentials() {
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
    let target = ProbeTarget {
        cluster: cluster("ctx"),
        kubeconfig: Arc::new(Kubeconfig::parse(yaml, Path::new("a.yaml")).expect("parses")),
        context: "ctx".to_owned(),
        proxy: Ok(cluster::ProxyChoice::Kubeconfig),
    };
    let results: Vec<_> = probe_stream(vec![target]).collect().await;
    let [(probed, ProbeResult::Unreachable { reason })] = results.as_slice() else {
        panic!("an unreachable server must fail, got {results:?}");
    };
    assert_eq!(probed, &cluster("ctx"));
    assert!(!reason.contains("fixture-token-value"), "{reason}");
}

#[test]
fn is_probing_follows_running() {
    let mut board = HealthBoard::default();
    assert!(!board.is_probing());
    board.mark_running(&[cluster("a")]);
    assert!(board.is_probing());
    board.record(cluster("a"), reachable(1), Instant::now());
    assert!(!board.is_probing());
}

#[tokio::test]
async fn a_probe_with_an_invalid_proxy_fails_closed_and_names_no_url() {
    let mut target = target("ctx");
    target.proxy = Err(cluster::ProxyUrlError::Credentials);
    let results: Vec<_> = probe_stream(vec![target]).collect().await;
    let [(_, ProbeResult::Unreachable { reason })] = results.as_slice() else {
        panic!("a bad proxy must not probe directly, got {results:?}");
    };
    assert_eq!(
        reason,
        "context 'ctx': the proxy URL in Settings is not valid: the proxy URL must not carry a user name or password"
    );
}

#[test]
fn folder_rows_are_never_probed_automatically() {
    // A dropped file can aim `tokenFile` at a real token and `server` at any host, so no auth kind
    // makes a folder row due; the same rows from a registry file are.
    let auths = [
        AuthKind::TokenFile,
        AuthKind::ClientCertificate,
        AuthKind::Token,
        AuthKind::Basic,
        AuthKind::None,
        AuthKind::Exec {
            command: "aws".to_owned(),
        },
    ];
    let now = Instant::now();
    let board = HealthBoard::default();
    for auth in auths {
        let folder = from_origin("ctx", auth.clone(), false, RowOrigin::Folder);
        assert!(board.due(&[folder], now).is_empty(), "{auth}");
        assert!(!is_probed_automatically(&auth, RowOrigin::Folder), "{auth}");
    }
    for auth in [
        AuthKind::TokenFile,
        AuthKind::ClientCertificate,
        AuthKind::Token,
    ] {
        let registry = from_origin("ctx", auth.clone(), false, RowOrigin::Registry);
        assert_eq!(board.due(&[registry], now), [cluster("ctx")], "{auth}");
    }
}
