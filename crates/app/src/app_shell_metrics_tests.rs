//! The metrics source state of the open cluster (spec 0048): the check follows the stored entry,
//! and the Settings window can read the live connection. Offline: a fake API server answers.

use cluster::fake_api::FakeApi;
use cluster::{MetricsScheme, MetricsSourceFields, WritePolicy};
use gpui_kit::TestAppContext;

use super::app_shell_switch_tests::{SwitchFixture, open_switch_fixture};
use super::*;
use crate::cluster_form::edit_entry;
use crate::cluster_metrics::SourceState;
use crate::cluster_registry::StoredMetrics;

const EMPTY_LIST: &str =
    r#"{"apiVersion":"v1","kind":"List","metadata":{"resourceVersion":"1"},"items":[]}"#;
const CPU_COUNT: &str = r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[1700000000,"42"]}]}}"#;

fn fields(prefix: &str) -> MetricsSourceFields {
    MetricsSourceFields {
        namespace: "monitoring".to_owned(),
        service: "vmselect".to_owned(),
        port: "8481".to_owned(),
        scheme: MetricsScheme::Http,
        prefix: prefix.to_owned(),
    }
}

fn live_fixture(name: &str, cx: &mut TestAppContext) -> (SwitchFixture, FakeApi) {
    let fixture = open_switch_fixture(name, cx);
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, |request| {
            if request.path.contains("/proxy/") {
                (200, CPU_COUNT.to_owned())
            } else {
                (200, EMPTY_LIST.to_owned())
            }
        })
    };
    let session = fixture.session(cx);
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
    });
    cx.run_until_parked();
    (fixture, api)
}

fn save_metrics(
    fixture: &SwitchFixture,
    entry: Option<MetricsSourceFields>,
    cx: &mut TestAppContext,
) {
    let cluster = fixture.cluster("prod-a", cx);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            edit_entry(&mut settings.registry, &cluster, |stored| {
                stored.metrics = entry.map(StoredMetrics::Fields)
            });
        });
    });
    cx.run_until_parked();
}

fn state_name(fixture: &SwitchFixture, cx: &mut TestAppContext) -> &'static str {
    let session = fixture.session(cx);
    session.read_with(cx, |session, _| {
        match session.live().map(|live| &live.metrics.source) {
            Some(SourceState::None) => "none",
            Some(SourceState::Invalid) => "invalid",
            Some(SourceState::Checking { .. }) => "checking",
            Some(SourceState::Ready { .. }) => "ready",
            Some(SourceState::Failed { .. }) => "failed",
            None => "not live",
        }
    })
}

fn proxy_requests(api: &FakeApi) -> usize {
    api.requests()
        .iter()
        .filter(|request| request.path.contains("/proxy/"))
        .count()
}

fn wait_for_state(fixture: &SwitchFixture, wanted: &'static str, cx: &mut TestAppContext) {
    for _ in 0..1_500 {
        cx.run_until_parked();
        if state_name(fixture, cx) == wanted {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for the {wanted} state");
}

#[gpui_kit::test]
fn a_session_without_an_entry_has_no_source_and_sends_nothing(cx: &mut TestAppContext) {
    let (fixture, api) = live_fixture("metrics-none", cx);
    assert_eq!(state_name(&fixture, cx), "none");
    assert_eq!(proxy_requests(&api), 0);
}

#[gpui_kit::test]
fn settings_change_restarts_the_source_check(cx: &mut TestAppContext) {
    let (fixture, api) = live_fixture("metrics-restart", cx);
    save_metrics(&fixture, Some(fields("/select/0/prometheus")), cx);
    wait_for_state(&fixture, "ready", cx);
    let requests = api.requests();
    let check = requests
        .iter()
        .find(|request| request.path.contains("/proxy/"))
        .expect("the check ran");
    assert_eq!(check.method, "GET");
    assert!(
        check
            .path
            .starts_with("/api/v1/namespaces/monitoring/services/http:vmselect:8481/proxy/select/0/prometheus/api/v1/query")
    );
    assert_eq!(proxy_requests(&api), 1);
    // The same entry written again changes nothing and sends nothing.
    save_metrics(&fixture, Some(fields("/select/0/prometheus")), cx);
    assert_eq!(state_name(&fixture, cx), "ready");
    assert_eq!(proxy_requests(&api), 1);
    // Another entry checks again; none goes back to metrics-server only.
    save_metrics(&fixture, Some(fields("")), cx);
    wait_for_state(&fixture, "ready", cx);
    assert_eq!(proxy_requests(&api), 2);
    save_metrics(&fixture, None, cx);
    assert_eq!(state_name(&fixture, cx), "none");
}

#[gpui_kit::test]
fn an_invalid_stored_entry_sends_no_request(cx: &mut TestAppContext) {
    let (fixture, api) = live_fixture("metrics-invalid", cx);
    save_metrics(&fixture, Some(fields("/a/../b")), cx);
    assert_eq!(state_name(&fixture, cx), "invalid");
    assert_eq!(proxy_requests(&api), 0);
}

#[gpui_kit::test]
fn the_live_connection_is_published_and_removed(cx: &mut TestAppContext) {
    let (fixture, _api) = live_fixture("metrics-connection", cx);
    let published = cx.update(|cx| {
        cx.try_global::<ActiveConnection>()
            .map(|active| active.cluster.context.clone())
    });
    assert_eq!(published.as_deref(), Some("prod-a"));
    fixture.shell.update(cx, |shell, cx| shell.release_all(cx));
    assert!(!cx.update(|cx| cx.has_global::<ActiveConnection>()));
}

// ---- The Monitor query of the open drawer ----

use super::app_shell_switch_tests::{palette_pod, palette_pod_object};
use crate::drawer::{DrawerTab, MonitorRange, MonitorScope};
use crate::monitor_source::SourceView;

const REFUSED: &str = r#"{"status":"error","errorType":"bad_data","error":"too many series"}"#;
const NO_CPU: &str = r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;

/// How the fake source answers.
#[derive(Clone, Copy)]
struct Backend {
    /// The answer to the check query.
    count: &'static str,
    /// Every range query is refused with a backend error.
    is_range_refused: bool,
}

impl Backend {
    const HEALTHY: Self = Self {
        count: CPU_COUNT,
        is_range_refused: false,
    };
}

/// A fake API: a matrix for every `query_range` (one point per step), a vector for `query`, and an
/// empty list for the rest.
fn monitor_answer(request: &cluster::fake_api::RecordedRequest, backend: Backend) -> (u16, String) {
    // The session's own pods watch must keep the pod the test shows.
    if request.path == "/api/v1/pods" {
        let pod =
            r#"{"metadata":{"name":"api-0","namespace":"shop"},"status":{"phase":"Running"}}"#;
        return (
            200,
            format!(
                r#"{{"apiVersion":"v1","kind":"PodList","metadata":{{"resourceVersion":"1"}},"items":[{pod}]}}"#
            ),
        );
    }
    if !request.path.contains("/proxy/") {
        return (200, EMPTY_LIST.to_owned());
    }
    if request.path.ends_with("/query") {
        return (200, backend.count.to_owned());
    }
    if backend.is_range_refused {
        return (400, REFUSED.to_owned());
    }
    // The numbers are plain digits, so the raw query needs no decoding.
    let number = |key: &str| {
        request
            .query
            .split('&')
            .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let (start, end, step) = (number("start"), number("end"), number("step"));
    let mut values = Vec::new();
    let mut at = start;
    while at <= end {
        values.push(serde_json::json!([at, "2"]));
        at += step;
    }
    (
        200,
        serde_json::json!({"status":"success","data":{"resultType":"matrix","result":[{"metric":{},"values":values}]}})
            .to_string(),
    )
}

fn monitor_fixture(name: &str, cx: &mut TestAppContext) -> (SwitchFixture, FakeApi) {
    monitor_fixture_with(name, Backend::HEALTHY, cx)
}

fn monitor_fixture_with(
    name: &str,
    backend: Backend,
    cx: &mut TestAppContext,
) -> (SwitchFixture, FakeApi) {
    let fixture = open_switch_fixture(name, cx);
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, move |request| {
            monitor_answer(request, backend)
        })
    };
    let session = fixture.session(cx);
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
        session.set_pods_for_test(vec![palette_pod("api-0")], cx);
    });
    cx.run_until_parked();
    save_metrics(&fixture, Some(fields("/select/0/prometheus")), cx);
    wait_for_state(&fixture, "ready", cx);
    // The selection is kept only while its row shows, so the table draws first.
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    let pod = palette_pod_object(&fixture, "api-0", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(Some(pod), cx);
        shell.set_drawer_open(true, cx);
        shell.set_drawer_tab(DrawerTab::Monitor, cx);
    });
    fixture.draw_twice(cx);
    (fixture, api)
}

fn range_requests(api: &FakeApi) -> usize {
    api.requests()
        .iter()
        .filter(|request| request.path.ends_with("/query_range"))
        .count()
}

fn wait_for_view(fixture: &SwitchFixture, cx: &mut TestAppContext) {
    fixture.wait_until("the source answer", cx, |shell, _| {
        shell
            .drawer
            .monitor
            .source
            .as_ref()
            .is_some_and(|fetch| fetch.view.is_some())
    });
}

#[gpui_kit::test]
fn an_open_monitor_queries_the_source_once(cx: &mut TestAppContext) {
    let (fixture, api) = monitor_fixture("monitor-once", cx);
    wait_for_view(&fixture, cx);
    assert_eq!(range_requests(&api), 6, "one query per metric");
    fixture.shell.read_with(cx, |shell, _| {
        let fetch = shell.drawer.monitor.source.as_ref().expect("a fetch");
        assert!(matches!(fetch.view, Some(SourceView::Charts { .. })));
    });
    // Renders with the same key and a fresh answer send nothing more.
    fixture.draw_twice(cx);
    fixture.draw_twice(cx);
    assert_eq!(
        range_requests(&api),
        6,
        "same key, fresh answer: no refetch"
    );
    let pods = api
        .requests()
        .into_iter()
        .filter(|request| request.path.ends_with("/query_range"))
        .map(|request| request.method)
        .collect::<Vec<_>>();
    assert!(pods.iter().all(|method| method == "GET"));
}

#[gpui_kit::test]
fn a_scope_change_drops_the_fetch_and_queries_again(cx: &mut TestAppContext) {
    let (fixture, api) = monitor_fixture("monitor-scope", cx);
    wait_for_view(&fixture, cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.set_monitor_scope(MonitorScope::Part("app".to_owned()), cx);
    });
    fixture.draw_twice(cx);
    let scope_of_fetch = fixture.shell.read_with(cx, |shell, _| {
        shell
            .drawer
            .monitor
            .source
            .as_ref()
            .map(|fetch| fetch.key.scope.clone())
    });
    assert_eq!(scope_of_fetch, Some(MonitorScope::Part("app".to_owned())));
    wait_for_view(&fixture, cx);
    assert_eq!(range_requests(&api), 12);
}

#[gpui_kit::test]
fn a_hidden_monitor_drops_the_fetch(cx: &mut TestAppContext) {
    let (fixture, _api) = monitor_fixture("monitor-hidden", cx);
    wait_for_view(&fixture, cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.set_drawer_tab(DrawerTab::Overview, cx);
    });
    fixture.draw_twice(cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.monitor.source.is_none());
    });
}

#[gpui_kit::test]
fn leaving_ready_resets_long_ranges(cx: &mut TestAppContext) {
    let (fixture, _api) = monitor_fixture("monitor-reset", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.set_monitor_range(MonitorRange::Days30, cx);
    });
    fixture.draw_twice(cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert_eq!(shell.drawer.monitor.range, MonitorRange::Days30);
        assert!(shell.drawer.monitor.source.is_some());
    });
    // The source is removed from the settings: the long range goes back to 24h.
    save_metrics(&fixture, None, cx);
    fixture.draw_twice(cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert_eq!(shell.drawer.monitor.range, MonitorRange::Hours24);
        assert!(shell.drawer.monitor.source.is_none());
    });
}

#[gpui_kit::test]
fn a_long_range_builds_no_sampler_cache(cx: &mut TestAppContext) {
    let (fixture, _api) = monitor_fixture("monitor-no-cache", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.set_monitor_range(MonitorRange::Days7, cx);
    });
    fixture.draw_twice(cx);
    wait_for_view(&fixture, cx);
    fixture.draw_twice(cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert!(
            shell.drawer.monitor.cache.is_none(),
            "7d never reads the sampler"
        );
    });
}

#[gpui_kit::test]
fn a_source_without_cpu_series_offers_the_sampler_ranges_only(cx: &mut TestAppContext) {
    let backend = Backend {
        count: NO_CPU,
        ..Backend::HEALTHY
    };
    let (fixture, api) = monitor_fixture_with("monitor-no-cpu", backend, cx);
    let session = fixture.session(cx);
    session.read_with(cx, |session, _| {
        let live = session.live().expect("live");
        let (_, check) = live.metrics.source.ready().expect("a reachable source");
        assert_eq!(check.cpu_series, 0);
    });
    fixture.shell.update(cx, |shell, cx| {
        shell.set_monitor_range(MonitorRange::Days30, cx);
    });
    fixture.draw_twice(cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.monitor.source.is_none(), "no fetch is wanted");
        assert_eq!(
            shell.drawer.monitor.range,
            MonitorRange::Hours24,
            "the long range goes back to 24h"
        );
    });
    assert_eq!(range_requests(&api), 0);
}

#[gpui_kit::test]
fn a_refused_short_range_falls_back_to_the_sampler_with_its_reason(cx: &mut TestAppContext) {
    let backend = Backend {
        is_range_refused: true,
        ..Backend::HEALTHY
    };
    let (fixture, _api) = monitor_fixture_with("monitor-refused", backend, cx);
    wait_for_view(&fixture, cx);
    fixture.draw_twice(cx);
    fixture.shell.read_with(cx, |shell, _| {
        let fetch = shell.drawer.monitor.source.as_ref().expect("a fetch");
        assert_eq!(shell.drawer.monitor.range, MonitorRange::Minutes15);
        match &fetch.view {
            Some(SourceView::Fallback(reason)) => assert_eq!(
                crate::monitor_source::fallback_note(reason),
                "Metrics source: the metrics backend refused the query: too many series. Showing k8sBoard samples."
            ),
            _ => panic!("a refused query falls back"),
        }
    });
}

#[gpui_kit::test]
fn a_switch_drops_the_fetch_and_ignores_a_late_answer(cx: &mut TestAppContext) {
    let (fixture, _api) = monitor_fixture("monitor-switch", cx);
    wait_for_view(&fixture, cx);
    let old_key = fixture.shell.read_with(cx, |shell, _| {
        shell
            .drawer
            .monitor
            .source
            .as_ref()
            .expect("a fetch")
            .key
            .clone()
    });
    let other = fixture.cluster("stg-b", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&other, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.monitor.source.is_none());
    });
    // An answer of the old key that lands after the switch changes nothing.
    fixture.shell.update(cx, |shell, cx| {
        shell.finish_source_fetch(
            &old_key,
            (jiff::Timestamp::now(), std::time::Duration::from_secs(15)),
            Ok(Vec::new()),
            cx,
        );
    });
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.monitor.source.is_none());
    });
}

#[gpui_kit::test]
fn a_ready_source_survives_a_new_connection_with_the_same_entry(cx: &mut TestAppContext) {
    let (fixture, api) = live_fixture("metrics-keep", cx);
    save_metrics(&fixture, Some(fields("/select/0/prometheus")), cx);
    wait_for_state(&fixture, "ready", cx);
    assert_eq!(proxy_requests(&api), 1);
    let session = fixture.session(cx);
    let connection = session.read_with(cx, |session, _| {
        session.live().expect("live").connection().clone()
    });
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        state_name(&fixture, cx),
        "ready",
        "kept at once, not checking"
    );
    assert_eq!(proxy_requests(&api), 1, "no second check");
    // Another entry is not kept.
    save_metrics(&fixture, Some(fields("")), cx);
    wait_for_state(&fixture, "ready", cx);
    assert_eq!(proxy_requests(&api), 2);
}

#[gpui_kit::test]
fn an_unreadable_entry_is_invalid_and_sends_no_request(cx: &mut TestAppContext) {
    let (fixture, api) = live_fixture("metrics-unreadable", cx);
    let cluster = fixture.cluster("prod-a", cx);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            edit_entry(&mut settings.registry, &cluster, |stored| {
                stored.metrics = Some(StoredMetrics::Unreadable(
                    serde_json::json!({"namespace": "monitoring", "scheme": "ftp"}),
                ));
            });
        });
    });
    cx.run_until_parked();
    assert_eq!(state_name(&fixture, cx), "invalid");
    assert_eq!(proxy_requests(&api), 0);
}

#[gpui_kit::test]
fn releasing_the_shell_removes_the_published_connection(cx: &mut TestAppContext) {
    let (fixture, _api) = live_fixture("metrics-release", cx);
    assert!(cx.update(|cx| cx.has_global::<ActiveConnection>()));
    let shell = fixture.shell.downgrade();
    cx.update_window(fixture.window.into(), |_, window, _| window.remove_window())
        .expect("the window is open");
    cx.run_until_parked();
    drop(fixture);
    cx.run_until_parked();
    assert!(shell.upgrade().is_none(), "the shell is released");
    assert!(!cx.update(|cx| cx.has_global::<ActiveConnection>()));
}
