use cluster::LeftoverPhase;

use super::*;

fn leftover(instance: Option<&str>) -> NodeShellLeftover {
    NodeShellLeftover {
        namespace: "kube-system".to_owned(),
        name: "k8sboard-node-shell-wk-03-x7k2q".to_owned(),
        uid: "uid".to_owned(),
        node: None,
        instance: instance.map(str::to_owned),
        phase: LeftoverPhase::Running,
        created_at: None,
    }
}

const HISTORY: &str = "aaaaaaaaaa started 2026-10-06T07:30:00Z\n\
                       aaaaaaaaaa quit 2026-10-06T07:41:12Z\n\
                       bbbbbbbbbb started 2026-10-06T08:00:00Z\n\
                       broken line\n\
                       cccccccccc quit not-a-time\n";

#[test]
fn a_pod_of_a_recorded_run_is_owned_and_names_the_quit_time() {
    let runs = PastRuns::parse(HISTORY);
    let pod = leftover(Some("aaaaaaaaaa"));
    assert!(runs.owns(&pod));
    assert_eq!(
        runs.note(&pod, &jiff::tz::TimeZone::UTC).as_deref(),
        Some("left by your session, quit at 07:41")
    );
}

#[test]
fn a_run_that_never_quit_says_only_that_it_is_yours() {
    let runs = PastRuns::parse(HISTORY);
    assert_eq!(
        runs.note(&leftover(Some("bbbbbbbbbb")), &jiff::tz::TimeZone::UTC)
            .as_deref(),
        Some("left by your session")
    );
}

#[test]
fn a_pod_of_an_unknown_run_or_with_no_run_label_is_not_owned() {
    let runs = PastRuns::parse(HISTORY);
    for pod in [
        leftover(Some("dddddddddd")),
        leftover(None),
        leftover(Some("cccccccccc")),
    ] {
        assert!(!runs.owns(&pod));
        assert_eq!(runs.note(&pod, &jiff::tz::TimeZone::UTC), None);
    }
}

#[test]
fn events_round_trip_through_the_file() {
    let dir = std::env::temp_dir().join(format!("k8sboard-runs-{}", cluster::random_suffix()));
    std::fs::create_dir_all(&dir).expect("a temp folder");
    record_run_event(&dir, "eeeeeeeeee", RunEvent::Started).expect("start is written");
    assert!(PastRuns::load(&dir).owns(&leftover(Some("eeeeeeeeee"))));
    record_run_event(&dir, "eeeeeeeeee", RunEvent::Quit).expect("quit is written");
    let note = PastRuns::load(&dir).note(&leftover(Some("eeeeeeeeee")), &jiff::tz::TimeZone::UTC);
    assert!(note.is_some_and(|note| note.contains("quit at")));
    let _ = std::fs::remove_dir_all(&dir);
}
