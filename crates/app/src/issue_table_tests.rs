use super::*;
use crate::issue::{IssueKey, IssueObject, IssueRule, IssueSeverity};
use crate::table_filter::{TableFilter, matches};

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn issue() -> Issue {
    let shown = IssueObject::new("Deployment", Some("shop"), "api");
    Issue {
        key: IssueKey {
            rule: IssueRule::PodCrash,
            object: shown.clone(),
        },
        severity: IssueSeverity::Critical,
        reason: "CrashLoopBackOff".into(),
        cause: "Exits with Error (exit 1) on each start.".to_owned(),
        target: shown.target(),
        subject: IssueObject::pod("shop", "api-7d9f8c-a"),
        shown,
        container: Some("api".to_owned()),
        count: 3,
        since: at(1_000),
        onset: Some(at(1_000)),
        action: IssueAction::ViewLogs {
            container: Some("api".to_owned()),
        },
    }
}

fn text(value: CellValue<'_>) -> String {
    match value {
        CellValue::Text(text) => text.into_owned(),
        CellValue::Status { text, .. } => text.to_string(),
        // The short label; the full kind is the prefix the filter also reads.
        CellValue::Qualified { text, .. } => text.to_owned(),
        _ => panic!("expected a text cell"),
    }
}

#[test]
fn issue_row_values_match_columns() {
    let issue = issue();
    assert!(matches!(
        issue.value(SEVERITY),
        CellValue::Status {
            tone: StatusTone::Bad,
            ..
        }
    ));
    assert_eq!(text(issue.value(SEVERITY)), "Critical");
    assert_eq!(text(issue.value(REASON)), "CrashLoopBackOff");
    assert_eq!(text(issue.value(KIND)), "Deployment");
    assert_eq!(text(issue.value(OBJECT)), "api");
    assert_eq!(text(issue.value(NAMESPACE)), "shop");
    assert!(text(issue.value(CAUSE)).starts_with("Exits with Error"));
    assert!(matches!(issue.value(COUNT), CellValue::Number(3)));
    assert!(matches!(issue.value(AGE), CellValue::Age(Some(since)) if since == at(1_000)));
    assert!(matches!(
        issue.value(ISSUE_COLUMNS.len()),
        CellValue::Absent
    ));
    assert_eq!((issue.namespace(), issue.name()), (Some("shop"), "api"));
    assert_eq!(issue.tone(), StatusTone::Bad);
    // Critical sorts before Warning, so the Severity column orders by tone through its text.
    let warning = Issue {
        severity: IssueSeverity::Warning,
        ..issue
    };
    assert_eq!(text(warning.value(SEVERITY)), "Warning");
}

#[test]
fn a_cluster_object_has_no_namespace_cell() {
    let node = IssueObject::node("node-a");
    let issue = Issue {
        shown: node.clone(),
        subject: node,
        ..issue()
    };
    assert!(matches!(issue.value(NAMESPACE), CellValue::Absent));
    assert_eq!(issue.namespace(), None);
}

#[test]
fn object_cell_appends_container() {
    assert_eq!(
        container_suffix(&issue()).as_deref(),
        Some(" · container api")
    );
    let whole_pod = Issue {
        container: None,
        ..issue()
    };
    assert_eq!(container_suffix(&whole_pod), None);
}

#[test]
fn count_cell_hides_one() {
    assert_eq!(count_text(1), None);
    assert_eq!(count_text(0), None);
    assert_eq!(count_text(12).as_deref(), Some("12"));
}

#[test]
fn quick_filter_matches_kind_and_cause() {
    let issue = issue();
    let filter = |text: &str| TableFilter {
        text: text.to_owned(),
        ..TableFilter::default()
    };
    let columns = ISSUE_COLUMNS.len();
    assert!(matches(&issue, &filter("deployment"), columns), "kind");
    assert!(matches(&issue, &filter("each start"), columns), "cause");
    assert!(matches(&issue, &filter("crashloop"), columns), "reason");
    assert!(matches(&issue, &filter("shop"), columns), "namespace");
    assert!(!matches(&issue, &filter("statefulset"), columns));
}

#[test]
fn view_logs_needs_the_subject_pod_in_the_list() {
    let pod = |name: &str| cluster::PodSummary {
        is_finished: false,
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        status: cluster::PodStatus::Reason(cluster::StatusReason::Running),
        ready: cluster::ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        containers: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
    };
    let pods = [pod("api-7d9f8c-a"), pod("api-7d9f8c-b")];
    // A group reads the logs of its representative pod, not of the Deployment it shows.
    let found = logs_pod(&issue(), &pods).expect("the subject pod");
    assert_eq!(found.name, "api-7d9f8c-a");
    assert!(logs_pod(&issue(), &pods[1..]).is_none(), "the pod is gone");
    let open_only = Issue {
        action: IssueAction::Open,
        ..issue()
    };
    assert!(logs_pod(&open_only, &pods).is_none());
}

#[test]
fn long_kinds_show_short_and_still_match_in_full() {
    assert_eq!(short_kind("HorizontalPodAutoscaler"), "HPA");
    assert_eq!(short_kind("PodDisruptionBudget"), "PDB");
    assert_eq!(short_kind("PersistentVolumeClaim"), "PVC");
    assert_eq!(short_kind("Deployment"), "Deployment");
    let hpa = Issue {
        shown: IssueObject::new("HorizontalPodAutoscaler", Some("shop"), "web"),
        ..issue()
    };
    let filter = |text: &str| TableFilter {
        text: text.to_owned(),
        ..TableFilter::default()
    };
    let columns = ISSUE_COLUMNS.len();
    assert!(matches(&hpa, &filter("hpa"), columns));
    assert!(matches(&hpa, &filter("autoscaler"), columns));
}

#[test]
fn an_unknown_onset_has_no_age_and_sorts_last() {
    let unknown = Issue {
        onset: None,
        ..issue()
    };
    assert!(matches!(unknown.value(AGE), CellValue::Age(None)));
}
