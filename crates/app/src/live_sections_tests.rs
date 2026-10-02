use cluster::{
    ControllerRef, CronSchedule, DeploymentSummary, EndpointPort, EndpointSummary, JobStatus,
    ServicePortSummary, TemplateContainer,
};

use super::*;

fn at(text: &str) -> jiff::Timestamp {
    text.parse().expect("valid timestamp")
}

fn deployment(revision: Option<&str>) -> DeploymentSummary {
    DeploymentSummary {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 3,
        ready: 3,
        up_to_date: 3,
        available: 3,
        strategy: String::new(),
        max_surge: None,
        max_unavailable: None,
        progress_deadline_seconds: 600,
        is_paused: false,
        revision: revision.map(str::to_owned),
        selector: vec!["app=api".to_owned()],
        containers: Vec::new(),
        conditions: Vec::new(),
    }
}

fn owner(kind: &str, name: &str) -> Option<ControllerRef> {
    Some(ControllerRef {
        kind: kind.to_owned(),
        name: name.to_owned(),
    })
}

fn replica_set(
    name: &str,
    revision: Option<&str>,
    owner: Option<ControllerRef>,
) -> ReplicaSetSummary {
    ReplicaSetSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 3,
        current: 3,
        ready: 3,
        owner,
        revision: revision.map(str::to_owned),
        selector: Vec::new(),
        containers: vec![TemplateContainer {
            name: "web".to_owned(),
            image: "registry.example.com:5000/api:1.4.2".to_owned(),
            ports: Vec::new(),
        }],
    }
}

fn job(name: &str, created: Option<&str>, owner: Option<ControllerRef>) -> JobSummary {
    JobSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: created.map(at),
        labels: Vec::new(),
        status: JobStatus::Complete,
        completions: Some(1),
        parallelism: Some(1),
        succeeded: 1,
        failed: 0,
        active: 0,
        backoff_limit: Some(6),
        active_deadline_seconds: None,
        ttl_seconds_after_finished: None,
        started_at: None,
        finished_at: None,
        owner,
        conditions: Vec::new(),
        containers: Vec::new(),
    }
}

fn cron_job(schedule: &str, is_suspended: bool) -> CronJobSummary {
    CronJobSummary {
        namespace: "team-a".to_owned(),
        name: "nightly".to_owned(),
        created_at: None,
        labels: Vec::new(),
        schedule: schedule.to_owned(),
        time_zone: None,
        timetable: CronSchedule::parse(schedule, None),
        is_suspended,
        concurrency_policy: String::new(),
        starting_deadline_seconds: None,
        successful_history_limit: None,
        failed_history_limit: None,
        active_jobs: Vec::new(),
        last_schedule_at: None,
        last_success_at: None,
        containers: Vec::new(),
    }
}

fn names<'a>(revisions: &[Revision<'a>]) -> Vec<&'a str> {
    revisions
        .iter()
        .map(|revision| revision.replica_set.name.as_str())
        .collect()
}

#[test]
fn revisions_newest_first_and_current_marked() {
    let owner = || owner("Deployment", "api");
    let sets = [
        replica_set("api-old", Some("9"), owner()),
        replica_set("api-new", Some("38"), owner()),
        replica_set("api-unnumbered", None, owner()),
        replica_set("api-mid", Some("10"), owner()),
    ];
    let revisions = revision_rows(&deployment(Some("38")), &sets);
    // Numeric order: 38 > 10 > 9, not the text order "9" > "38" > "10".
    assert_eq!(
        names(&revisions),
        ["api-new", "api-mid", "api-old", "api-unnumbered"]
    );
    let current: Vec<bool> = revisions
        .iter()
        .map(|revision| revision.is_current)
        .collect();
    assert_eq!(current, [true, false, false, false]);
    // Without a revision on the Deployment nothing is current.
    let revisions = revision_rows(&deployment(None), &sets);
    assert!(revisions.iter().all(|revision| !revision.is_current));
}

#[test]
fn revisions_keep_only_owned_replica_sets() {
    let sets = [
        replica_set("api-7d9f8c", Some("2"), owner("Deployment", "api")),
        replica_set(
            "api-worker-7d9f8c",
            Some("3"),
            owner("Deployment", "api-worker"),
        ),
        replica_set("api-orphan", Some("4"), None),
        replica_set("api-statefulish", Some("5"), owner("StatefulSet", "api")),
    ];
    let revisions = revision_rows(&deployment(Some("2")), &sets);
    assert_eq!(names(&revisions), ["api-7d9f8c"]);
    // A ReplicaSet of the same name in another namespace is not ours.
    let mut other = replica_set("api-other", Some("1"), owner("Deployment", "api"));
    other.namespace = "team-b".to_owned();
    assert!(revision_rows(&deployment(Some("1")), &[other]).is_empty());
}

#[test]
fn image_tag_of_registry_with_port() {
    assert_eq!(image_tag("nginx:1.27"), "1.27");
    assert_eq!(
        image_tag("registry.example.com:5000/team/api:1.4.2"),
        "1.4.2"
    );
    // The port after the host is not a tag; with no tag the whole reference reads.
    assert_eq!(
        image_tag("registry.example.com:5000/team/api"),
        "registry.example.com:5000/team/api"
    );
    assert_eq!(image_tag("nginx"), "nginx");
}

#[test]
fn recent_jobs_newest_first_and_owned_only() {
    let cron = cron_job("0 3 * * *", false);
    let owned = || owner("CronJob", "nightly");
    let jobs = [
        job("nightly-100", Some("2024-05-01T03:00:00Z"), owned()),
        job("nightly-300", Some("2024-05-03T03:00:00Z"), owned()),
        job("nightly-unstamped", None, owned()),
        job("nightly-200", Some("2024-05-02T03:00:00Z"), owned()),
        job(
            "other-400",
            Some("2024-05-04T03:00:00Z"),
            owner("CronJob", "other"),
        ),
        job("manual", Some("2024-05-05T03:00:00Z"), None),
    ];
    let shown: Vec<&str> = recent_jobs(&cron, &jobs)
        .iter()
        .map(|job| job.name.as_str())
        .collect();
    assert_eq!(
        shown,
        [
            "nightly-300",
            "nightly-200",
            "nightly-100",
            "nightly-unstamped"
        ]
    );
}

#[test]
fn run_label_today_and_later_day() {
    let schedule = CronSchedule::parse("45 10 * * *", None).expect("valid schedule");
    let now = at("2024-10-04T09:00:00Z");
    let runs = schedule.next_runs(now, 2);
    assert_eq!(run_label(&runs[0], now), "10:45 UTC");
    assert_eq!(run_label(&runs[1], now), "Oct 5 10:45 UTC");
}

const NOW: &str = "2024-10-04T10:44:00Z";

fn note_of(cron_job: &CronJobSummary) -> String {
    match next_runs_content(cron_job, at(NOW)) {
        NextRunsContent::Note(text) => text,
        NextRunsContent::Runs(runs) => panic!("expected a note, got runs {runs:?}"),
    }
}

#[test]
fn next_runs_content_lists_three_runs() {
    let NextRunsContent::Runs(runs) = next_runs_content(&cron_job("45 10 * * *", false), at(NOW))
    else {
        panic!("a daily schedule has runs");
    };
    assert_eq!(
        runs,
        [
            ("10:45 UTC".to_owned(), "in 1m".to_owned()),
            ("Oct 5 10:45 UTC".to_owned(), "in 1d".to_owned()),
            ("Oct 6 10:45 UTC".to_owned(), "in 2d".to_owned()),
        ]
    );
}

#[test]
fn suspended_cron_job_has_a_note_instead_of_runs() {
    assert_eq!(
        note_of(&cron_job("45 10 * * *", true)),
        "Suspended: no runs are scheduled"
    );
}

#[test]
fn invalid_schedule_has_an_error_note() {
    assert!(note_of(&cron_job("61 * * * *", false)).starts_with("Cannot compute next runs: "));
}

#[test]
fn impossible_schedule_has_a_five_year_note() {
    assert_eq!(
        note_of(&cron_job("0 0 30 2 *", false)),
        "No run within the next 5 years"
    );
}

#[test]
fn unanchored_every_schedule_waits_for_the_first_run() {
    assert_eq!(
        note_of(&cron_job("@every 1h", false)),
        "Next run is known after the first run"
    );
}

#[test]
fn next_run_text_counts_down_to_the_next_run() {
    let schedule = CronSchedule::parse("*/15 * * * *", None).expect("valid schedule");
    assert_eq!(
        next_run_text(&schedule, at("2024-10-04T10:00:30Z")).as_deref(),
        Some("in 14m")
    );
    let impossible = CronSchedule::parse("0 0 30 2 *", None).expect("valid schedule");
    assert_eq!(next_run_text(&impossible, at("2024-10-04T10:00:30Z")), None);
}

fn service() -> ServiceSummary {
    ServiceSummary {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        service_type: "ClusterIP".to_owned(),
        cluster_ips: Vec::new(),
        is_headless: false,
        external_addresses: Vec::new(),
        ports: Vec::new(),
        selector: vec!["app=api".to_owned()],
    }
}

fn slice(ports: &[(u16, &str)], endpoints: Vec<EndpointSummary>) -> EndpointSliceSummary {
    EndpointSliceSummary {
        namespace: "team-a".to_owned(),
        name: "api-abc".to_owned(),
        service: Some("api".to_owned()),
        address_type: "IPv4".to_owned(),
        ports: ports
            .iter()
            .map(|(port, protocol)| EndpointPort {
                name: None,
                port: Some(*port),
                protocol: (*protocol).to_owned(),
            })
            .collect(),
        endpoints,
    }
}

fn endpoint(address: &str, pod: Option<&str>, is_ready: bool) -> EndpointSummary {
    EndpointSummary {
        address: address.to_owned(),
        is_ready,
        is_terminating: false,
        pod: pod.map(str::to_owned),
        node: None,
    }
}

fn endpoint_pod(name: &str) -> Option<ResourceKey> {
    ResourceKey::of_object("Pod", Some("team-a"), name)
}

#[test]
fn endpoint_rows_single_and_multi_port() {
    let endpoints = vec![endpoint("10.0.0.1", Some("api-1"), true)];
    // One port joins the address, and there is no Ports field.
    let single = endpoints_content(&service(), &[slice(&[(8080, "TCP")], endpoints.clone())]);
    assert_eq!(single.ports, None);
    assert_eq!(single.rows[0].text, "10.0.0.1:8080 · api-1");
    // Several ports move to a Ports field, and the rows drop the port.
    let several = endpoints_content(
        &service(),
        &[slice(&[(8080, "TCP"), (9090, "UDP")], endpoints)],
    );
    assert_eq!(several.ports.as_deref(), Some("8080/TCP, 9090/UDP"));
    assert_eq!(several.rows[0].text, "10.0.0.1 · api-1");
}

#[test]
fn endpoint_rows_carry_their_state() {
    let mut terminating = endpoint("10.0.0.3", Some("api-3"), false);
    terminating.is_terminating = true;
    let endpoints = vec![
        endpoint("10.0.0.1", Some("api-1"), true),
        endpoint("10.0.0.2", None, false),
        terminating,
    ];
    let content = endpoints_content(&service(), &[slice(&[(8080, "TCP")], endpoints)]);
    let states: Vec<EndpointState> = content.rows.iter().map(|row| row.state).collect();
    assert_eq!(
        states,
        [
            EndpointState::Ready,
            EndpointState::NotReady,
            EndpointState::Terminating
        ]
    );
}

#[test]
fn endpoint_rows_link_only_endpoints_with_a_pod() {
    let endpoints = vec![
        endpoint("10.0.0.1", Some("api-1"), true),
        endpoint("10.0.0.2", None, true),
    ];
    let content = endpoints_content(&service(), &[slice(&[(8080, "TCP")], endpoints)]);
    assert_eq!(content.rows[0].pod, endpoint_pod("api-1"));
    assert_eq!(content.rows[1].pod, None);
}

#[test]
fn endpoint_ports_follow_the_service_port_order() {
    let mut own = service();
    own.ports = ["webhook", "metrics"]
        .map(|name| ServicePortSummary {
            name: Some(name.to_owned()),
            port: 80,
            target_port: None,
            node_port: None,
            protocol: "TCP".to_owned(),
        })
        .to_vec();
    let mut api = slice(&[(8080, "TCP"), (7000, "TCP")], Vec::new());
    // The API lists the ports in another order than the Service does.
    api.ports[0].name = Some("metrics".to_owned());
    api.ports[1].name = Some("webhook".to_owned());
    let content = endpoints_content(&own, &[api]);
    assert_eq!(content.ports.as_deref(), Some("7000/TCP, 8080/TCP"));
}

#[test]
fn endpoint_rows_bracket_ipv6_addresses() {
    let content = endpoints_content(
        &service(),
        &[slice(&[(80, "TCP")], vec![endpoint("fd00::1", None, true)])],
    );
    assert_eq!(content.rows[0].text, "[fd00::1]:80");
}

#[test]
fn endpoint_rows_skip_slices_of_other_services() {
    let mut other = slice(&[(80, "TCP")], vec![endpoint("10.0.0.9", None, true)]);
    other.service = Some("web".to_owned());
    assert!(endpoints_content(&service(), &[other]).rows.is_empty());
}
