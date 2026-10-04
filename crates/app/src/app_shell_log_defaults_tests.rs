//! The saved log defaults in a headless window: a new log tab takes them, and its own toggles
//! never write back. The pod's log request goes to a fake API server, so nothing leaves the
//! machine.

use cluster::fake_api::FakeApi;
use cluster::{NamespaceScope, WritePolicy};
use gpui_kit::TestAppContext;

use super::app_shell_switch_tests::{SwitchFixture, open_switch_fixture};
use super::app_shell_tests::logs_pod;
use super::*;
use crate::log_tab::LogTab;
use crate::settings::{AppSettings, LogSettings};

const LOG_PATH: &str = "/api/v1/namespaces/shop/pods/api-0/log";

/// A fixture with `logs` saved, one live pod, and a log tab open on it over a fake API server.
fn open_log_tab(
    name: &str,
    logs: LogSettings,
    cx: &mut TestAppContext,
) -> (SwitchFixture, FakeApi, Entity<LogTab>) {
    let fixture = open_switch_fixture(name, cx);
    cx.update(|cx| AppSettings::update(cx, |settings| settings.logs = logs));
    fixture.go_live(NamespaceScope::All, cx);
    fixture.session(cx).update(cx, |session, cx| {
        session.set_pods_for_test(vec![logs_pod()], cx);
    });
    cx.run_until_parked();
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, |_| {
            (200, "2024-05-01T10:00:00Z hello\n".to_owned())
        })
    };
    let cluster = fixture.cluster("prod-a", cx);
    fixture.with_window(cx, |window, cx| {
        let shell = fixture.shell.read(cx);
        let row = shell
            .slot_row_context(&cluster, cx)
            .expect("the cluster is viewed");
        let live = shell.slot_live(&cluster, cx).expect("a live slot");
        let pod = live.pods.items().first().expect("a pod").clone();
        let target = LogTarget::of_container(&pod, "app").expect("a log target");
        let dock = shell.dock.clone();
        dock.update(cx, |dock, cx| {
            dock.open(
                crate::dock::LogOrigin::new(&row, connection),
                target,
                window,
                cx,
            )
        });
    });
    let tab = fixture
        .shell
        .read_with(cx, |shell, cx| shell.dock.read(cx).log_tab_entities())
        .pop()
        .expect("a log tab opened");
    (fixture, api, tab)
}

#[gpui_kit::test]
fn new_log_tab_takes_the_log_defaults(cx: &mut TestAppContext) {
    let logs = LogSettings {
        show_timestamps: false,
        wrap_lines: true,
        show_json: true,
        ..LogSettings::default()
    };
    let (_fixture, _api, tab) = open_log_tab("log-defaults", logs.clone(), cx);
    assert_eq!(
        tab.read_with(cx, |tab, _| tab.toggles()),
        (false, true, true)
    );
    // A toggle on the tab leaves the saved defaults alone.
    tab.update(cx, |tab, _| tab.flip_toggles());
    assert_eq!(
        tab.read_with(cx, |tab, _| tab.toggles()),
        (true, false, false)
    );
    assert_eq!(cx.read(|cx| AppSettings::get(cx).logs.clone()), logs);
}

#[gpui_kit::test]
fn log_tab_requests_the_configured_tail(cx: &mut TestAppContext) {
    let logs = LogSettings {
        tail_lines: 500,
        ..LogSettings::default()
    };
    let (fixture, api, _tab) = open_log_tab("log-tail", logs, cx);
    fixture.wait_until("the log request", cx, |_, _| {
        api.requests()
            .iter()
            .any(|request| request.path == LOG_PATH)
    });
    let request = api
        .requests()
        .into_iter()
        .find(|request| request.path == LOG_PATH)
        .expect("a log request");
    assert!(request.has_query("tailLines", "500"), "{}", request.query);
}
