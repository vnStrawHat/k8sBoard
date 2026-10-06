use cluster::NodeReadiness;

use super::*;
use crate::cluster_capacity::{FromPods, Layers};
use crate::issue::{IssueAction, IssueKey, IssueObject, IssueRule, IssueSeverity};
use crate::node_usage::NodeUsage;
use crate::recent_changes::{ActorSource, ChangeKind};

fn now() -> Timestamp {
    "2024-05-01T12:00:00Z".parse().unwrap()
}

fn issue(name: &str, cause: &str) -> Issue {
    let shown = IssueObject::pod("payments", name);
    Issue {
        key: IssueKey {
            rule: IssueRule::PodCrash,
            object: shown.clone(),
        },
        severity: IssueSeverity::Critical,
        reason: "CrashLoopBackOff".into(),
        cause: cause.to_owned(),
        subject: shown.clone(),
        target: shown.target(),
        shown,
        container: Some("api".to_owned()),
        count: 1,
        since: "2024-05-01T11:00:00Z".parse().unwrap(),
        onset: Some("2024-05-01T11:00:00Z".parse().unwrap()),
        action: IssueAction::Open,
    }
}

fn inputs<'a>() -> ReportInputs<'a> {
    ReportInputs {
        headline: "readonly@Monitor · Kubernetes v1.29.5",
        stats: "4 / 4 nodes ready · 102 / 104 pods running · 20 namespaces",
        checked: true,
        issues: &[],
        coverage_note: None,
        capacity: &[],
        cells: &[],
        changes: &[],
        changes_note: None,
        window: ChangeWindow::FifteenMinutes,
        now: now(),
    }
}

#[test]
fn report_lists_every_issue_not_only_six() {
    let issues: Vec<Issue> = (0..9).map(|n| issue(&format!("api-{n}"), "boom")).collect();
    let report = overview_report(&ReportInputs {
        issues: &issues,
        ..inputs()
    });
    assert!(report.contains("## Needs attention (9)"));
    assert_eq!(report.matches("CrashLoopBackOff").count(), 9);
    assert!(report.contains("payments / api-8 · container api"));
    assert!(report.contains("2024-05-01T11:00:00Z"));
}

#[test]
fn report_has_coverage_note_when_partial() {
    let issues = [issue("api-0", "boom")];
    let report = overview_report(&ReportInputs {
        issues: &issues,
        coverage_note: Some("Not checked: HPAs (denied).".to_owned()),
        ..inputs()
    });
    assert!(report.contains("Partial coverage: Not checked: HPAs (denied)."));
    assert!(!overview_report(&inputs()).contains("Partial coverage"));
}

#[test]
fn report_says_no_issues_when_empty() {
    let report = overview_report(&inputs());
    assert!(report.contains("## Needs attention (0)"));
    assert!(report.contains("No issues found."));
}

#[test]
fn report_capacity_and_nodes_tables() {
    let capacity = [CapacityRow::Cpu(Layers {
        used: Some(104.),
        requested: FromPods::Known(131.),
        allocatable: 168.,
        nodes: 4,
        unsampled_nodes: 0,
    })];
    let cells = [
        HeatCell {
            node: "wk-1".to_owned(),
            usage: NodeUsage {
                cpu: Some(0.62),
                memory: Some(0.48),
            },
            readiness: NodeReadiness::Ready,
            is_cordoned: false,
        },
        HeatCell {
            node: "wk-2".to_owned(),
            usage: NodeUsage::default(),
            readiness: NodeReadiness::NotReady,
            is_cordoned: true,
        },
    ];
    let report = overview_report(&ReportInputs {
        capacity: &capacity,
        cells: &cells,
        ..inputs()
    });
    assert!(report.contains("| CPU | 104 cores used · 131 cores req · 168 cores |"));
    assert!(report.contains("| wk-1 | Ready | 62% | 48% |"));
    assert!(report.contains("| wk-2 | NotReady, SchedulingDisabled | — | — |"));
}

#[test]
fn report_changes_use_window_label() {
    let changes = [ChangeEntry {
        at: "2024-05-01T11:55:00Z".parse().unwrap(),
        kind: ChangeKind::Deployment,
        object: "payments/api".to_owned(),
        text: "Scaled up replica set api-7d9f8c to 3".to_owned(),
        count: 2,
        actor: Some("deployment-controller".to_owned()),
        actor_source: ActorSource::EventSource,
        replica_set: None,
        target: None,
    }];
    let report = overview_report(&ReportInputs {
        changes: &changes,
        window: ChangeWindow::OneHour,
        ..inputs()
    });
    assert!(report.contains("## Recent changes (Last 1 h)"));
    assert!(report.contains(
        "| 2024-05-01T11:55:00Z | Deployment payments/api Scaled up replica set api-7d9f8c to 3 ×2 | deployment-controller |"
    ));
    assert!(report.contains("events kept ~1 h by the API server"));
}

#[test]
fn cell_escapes_pipes_and_newlines() {
    assert_eq!(cell("a|b"), "a\\|b");
    assert_eq!(cell("a\nb\r\nc"), "a b c");
    let issues = [issue("api-0", "x | y\nz")];
    let report = overview_report(&ReportInputs {
        issues: &issues,
        ..inputs()
    });
    assert!(report.contains("x \\| y z"));
}

#[test]
fn report_has_no_server_or_user() {
    let issues = [issue("api-0", "boom")];
    let report = overview_report(&ReportInputs {
        issues: &issues,
        ..inputs()
    });
    assert!(report.starts_with("# Overview — readonly@Monitor · Kubernetes v1.29.5"));
    assert!(report.contains("Generated 2024-05-01T12:00:00Z"));
    for forbidden in ["https://", "server:", "token", "user", "certificate"] {
        assert!(!report.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn report_says_not_checked_before_the_board_has_run() {
    let report = overview_report(&ReportInputs {
        checked: false,
        ..inputs()
    });
    assert!(report.contains("Not checked yet."));
    assert!(!report.contains("No issues found."));
}

#[test]
fn report_names_why_changes_are_missing() {
    let report = overview_report(&ReportInputs {
        changes_note: Some("Not permitted: list events".to_owned()),
        ..inputs()
    });
    assert!(report.contains("## Recent changes (Last 15 min)\n\nNot permitted: list events\n"));
    assert!(!report.contains("| Time |"));
}
