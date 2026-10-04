//! The metrics source state of the open cluster (spec 0048): the check follows the stored entry,
//! and the Settings window can read the live connection. Offline: a fake API server answers.

use cluster::fake_api::FakeApi;
use cluster::{MetricsScheme, MetricsSourceFields, WritePolicy};
use gpui_kit::TestAppContext;

use super::app_shell_switch_tests::{SwitchFixture, open_switch_fixture};
use super::*;
use crate::cluster_form::edit_entry;
use crate::cluster_metrics::SourceState;

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
                stored.metrics = entry
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
