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
    assert_eq!(container_suffix(&issue()).as_deref(), Some(" · api"));
    let whole_pod = Issue {
        container: None,
        ..issue()
    };
    assert_eq!(container_suffix(&whole_pod), None);
}

#[test]
fn the_container_shows_only_when_the_name_keeps_its_room() {
    let suffix = || container_suffix(&issue());
    // A short name keeps all of itself, so a narrow column still names the container.
    assert_eq!(
        suffix_that_fits("api", suffix(), 9).as_deref(),
        Some(" · api")
    );
    assert_eq!(suffix_that_fits("api", suffix(), 8), None);
    // A long name keeps 15 characters first: 15 + the 6 of ` · api`.
    let long = "crashloop-85cc769bcd-q4nqt";
    assert_eq!(
        suffix_that_fits(long, suffix(), 21).as_deref(),
        Some(" · api")
    );
    assert_eq!(suffix_that_fits(long, suffix(), 20), None);
    assert_eq!(suffix_that_fits(long, None, 40), None);
}

#[test]
fn the_cause_tooltip_names_the_container() {
    assert_eq!(
        cause_tooltip(&issue()),
        "Exits with Error (exit 1) on each start.\nContainer: api"
    );
    let whole_pod = Issue {
        container: None,
        ..issue()
    };
    assert_eq!(
        cause_tooltip(&whole_pod),
        "Exits with Error (exit 1) on each start."
    );
}

/// The widths of the columns of a window `window` px wide: the table is the window less the 250 px
/// sidebar.
fn widths_at(window: f32) -> Vec<f32> {
    let plan = ColumnPlan {
        specs: ISSUE_COLUMNS.to_vec(),
        flexible: CAUSE,
    };
    let layout = crate::table_layout::layout_columns(
        &plan.specs,
        plan.flexible,
        gpui_kit::px(window - 250.),
        &std::collections::BTreeSet::new(),
    );
    layout
        .columns
        .iter()
        .skip(1)
        .map(|column| f32::from(column.width))
        .collect()
}

#[test]
fn count_and_age_stay_inside_the_window_at_1100_and_1320_px() {
    for window in [1100., 1320.] {
        let widths = widths_at(window);
        // The window less the sidebar, the table gutter, and the checkbox column.
        let room = window - 250. - 28. - 32.;
        let total: f32 = widths.iter().sum();
        assert!(
            total <= room + 0.5,
            "{window} px: {total} px of columns for {room} px"
        );
    }
}

#[test]
fn object_keeps_fifteen_characters_and_cause_grows_most() {
    // A mono glyph is about 9.6 px, and a cell keeps 24 px of padding.
    let fifteen = 15. * 9.6 + 24.;
    for window in [1100., 1320.] {
        let widths = widths_at(window);
        assert!(widths[OBJECT] >= fifteen, "{window} px: {widths:?}");
    }
    let (narrow, wide) = (widths_at(1100.), widths_at(1320.));
    let growth = |index: usize| wide[index] - narrow[index];
    assert!(
        (0..8).all(|index| growth(CAUSE) >= growth(index)),
        "{narrow:?} {wide:?}"
    );
    assert!(wide[KIND] <= 130. && wide[NAMESPACE] <= 150.);
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
        annotations: cluster::AnnotationTerms::default(),
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
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
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
fn view_logs_of_a_job_reads_its_newest_pod() {
    let pod = |name: &str, job: &str, created: i64| cluster::PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        status: cluster::PodStatus::Reason(cluster::StatusReason::Error),
        ready: cluster::ReadyCount { ready: 0, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: Some(at(created)),
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: Some(cluster::ControllerRef {
            kind: "Job".to_owned(),
            name: job.to_owned(),
        }),
        conditions: Vec::new(),
        containers: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
    };
    let pods = [
        pod("report-a", "report", 100),
        pod("report-b", "report", 200),
        pod("other-a", "other", 300),
    ];
    let job = IssueObject::new("Job", Some("shop"), "report");
    let issue = Issue {
        shown: job.clone(),
        subject: job,
        container: None,
        action: IssueAction::ViewLogs { container: None },
        ..issue()
    };
    assert_eq!(logs_pod(&issue, &pods).expect("a pod").name, "report-b");
    assert!(logs_pod(&issue, &pods[2..]).is_none(), "no pod of the job");
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

#[test]
fn reason_holds_the_longest_built_in_reason_at_1100_and_1320_px() {
    // `Backoff limit reached` is the longest: measured at the UI font, `Backoff limit reac…` is cut at 152 px.
    for window in [1100., 1320.] {
        assert!(widths_at(window)[REASON] >= 175., "{window} px");
    }
}
