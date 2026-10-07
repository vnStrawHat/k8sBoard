//! The sidebar numbers of kinds without a watch (one `limit=1` list each) are recounted on their
//! own, so a number that moves with no navigation does not stay stale.

use std::time::Duration;

use cluster::fake_api::{FakeApi, RecordedRequest};
use gpui_kit::TestAppContext;

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{go_live_answering, switch_to};
use crate::cluster_session::CountTrigger;

const NOT_FOUND: &str = r#"{"kind":"Status","status":"Failure","code":404}"#;

fn count_lists(api: &FakeApi) -> usize {
    api.requests()
        .iter()
        .filter(|request| request.method == "GET" && request.has_query("limit", "1"))
        .count()
}

#[gpui_kit::test]
fn the_sidebar_counts_run_again_every_thirty_seconds_without_a_navigation(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("kind-count-ticker", cx);
    switch_to(&fixture, "stg-b", cx);
    let stg = fixture.cluster("stg-b", cx);
    let api = go_live_answering(
        &fixture,
        &stg,
        "node-b",
        |_: &RecordedRequest| (404, NOT_FOUND.to_owned()),
        cx,
    );
    fixture.shell.update(cx, |shell, cx| {
        if let Some(session) = shell.session_of(&stg).cloned() {
            session.update(cx, |session, cx| {
                session.refresh_kind_counts(CountTrigger::Review, cx);
            });
        }
    });
    // The first run: one tiny list per countable kind.
    for _ in 0..1_500 {
        cx.run_until_parked();
        if count_lists(&api) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // The first list is only the start of the run: one list per kind follows on the tokio thread, so
    // wait until the count holds still before treating it as the whole run.
    let mut first = count_lists(&api);
    let mut quiet_polls = 0;
    for _ in 0..1_500 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
        let now = count_lists(&api);
        quiet_polls = if now == first { quiet_polls + 1 } else { 0 };
        first = now;
        if quiet_polls >= 30 {
            break;
        }
    }
    assert!(first > 0, "the review counts once");
    // A moment later nothing counts again.
    cx.executor().advance_clock(Duration::from_secs(20));
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(count_lists(&api), first);
    // Past 30 seconds the ticker counts again, with no screen change in between.
    cx.executor().advance_clock(Duration::from_secs(15));
    for _ in 0..1_500 {
        cx.run_until_parked();
        if count_lists(&api) > first {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(count_lists(&api) > first, "the counts run again");
}
