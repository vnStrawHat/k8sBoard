use cluster::{
    ConfigMapKey, ConfigMapValue, ControllerRef, CronSchedule, DeploymentSummary, EndpointPort,
    EndpointSummary, JobStatus, ServicePortSummary, TemplateContainer,
};

use super::*;

// ---- Bindings ----

fn account_subject(namespace: &str, name: &str) -> cluster::Subject {
    cluster::Subject {
        kind: cluster::SubjectKind::ServiceAccount,
        name: name.to_owned(),
        namespace: Some(namespace.to_owned()),
    }
}

fn binding_named(
    namespace: Option<&str>,
    name: &str,
    role: (cluster::RoleKind, &str),
    subjects: Vec<cluster::Subject>,
) -> BindingSummary {
    BindingSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: cluster::RoleRef {
            kind: role.0,
            name: role.1.to_owned(),
        },
        subjects,
    }
}

fn plain_role(namespace: Option<&str>, name: &str, grants_everything: bool) -> RoleSummary {
    let star = || vec!["*".to_owned()];
    RoleSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        rules: if grants_everything {
            vec![cluster::RbacRule {
                api_groups: star(),
                resources: star(),
                resource_names: Vec::new(),
                verbs: star(),
                non_resource_urls: Vec::new(),
            }]
        } else {
            Vec::new()
        },
        aggregation: Vec::new(),
    }
}

#[test]
fn role_bindings_rows_link_back() {
    let bindings = [
        binding_named(
            Some("shop"),
            "zeta",
            (cluster::RoleKind::Role, "reader"),
            Vec::new(),
        ),
        binding_named(
            Some("shop"),
            "alpha",
            (cluster::RoleKind::Role, "reader"),
            Vec::new(),
        ),
        binding_named(
            Some("other"),
            "beta",
            (cluster::RoleKind::Role, "reader"),
            Vec::new(),
        ),
    ];
    let index = BindingIndex::build(&crate::access_bindings::BindingLists {
        role_bindings: &bindings,
        cluster_role_bindings: &[],
    });
    let listed = role_binding_list(&index, &plain_role(Some("shop"), "reader", false));
    let names: Vec<&str> = listed.iter().map(|binding| binding.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zeta"]);
    // Each row links to the binding row, whose drawer links back to the role.
    assert_eq!(
        binding_key(listed[0]),
        ResourceKey::Kind {
            kind: ResourceKind::RoleBindings,
            namespace: Some("shop".to_owned()),
            name: "alpha".to_owned(),
        }
    );
    assert_eq!(binding_text(listed[0]), "rolebinding/alpha");
}

#[test]
fn role_subjects_service_accounts_first_with_review() {
    let bindings = [binding_named(
        None,
        "root",
        (cluster::RoleKind::ClusterRole, "super"),
        vec![
            cluster::Subject {
                kind: cluster::SubjectKind::User,
                name: "ana".to_owned(),
                namespace: None,
            },
            account_subject("kube-system", "tiller"),
        ],
    )];
    let index = BindingIndex::build(&crate::access_bindings::BindingLists {
        role_bindings: &[],
        cluster_role_bindings: &bindings,
    });
    let broad = plain_role(None, "super", true);
    let subjects = role_subjects(&index.bindings_of_role(&broad));
    let texts: Vec<&str> = subjects.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(texts, ["sa kube-system/tiller", "user ana"]);
    assert!(needs_review(&broad, &subjects[0]));
    assert!(!needs_review(&broad, &subjects[1]));
    // A narrow role needs no review.
    assert!(!needs_review(
        &plain_role(None, "super", false),
        &subjects[0]
    ));
    assert_eq!(binding_key(subjects[0].binding), binding_key(&bindings[0]));
}

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

fn config_map(keys: &[(&str, usize, bool)]) -> ConfigMapSummary {
    ConfigMapSummary {
        namespace: "team-a".to_owned(),
        name: "settings".to_owned(),
        created_at: None,
        labels: Vec::new(),
        keys: keys
            .iter()
            .map(|(name, size_bytes, is_binary)| ConfigMapKey {
                name: (*name).to_owned(),
                size_bytes: *size_bytes,
                is_binary: *is_binary,
            })
            .collect(),
        is_immutable: false,
    }
}

fn values(entries: Vec<(&str, ValuePreview)>) -> ConfigMapValues {
    ConfigMapValues {
        namespace: "team-a".to_owned(),
        name: "settings".to_owned(),
        entries: entries
            .into_iter()
            .map(|(key, preview)| ConfigMapValue {
                key: key.to_owned(),
                preview,
            })
            .collect(),
    }
}

fn texts(lines: &[DataLine]) -> Vec<(&str, &str)> {
    lines
        .iter()
        .map(|line| (line.key.as_str(), line.text.as_str()))
        .collect()
}

#[test]
fn config_map_data_prefers_previews() {
    let summary = config_map(&[
        ("app.yaml", 40, false),
        ("config.json", 412, false),
        ("logo.png", 2048, true),
        ("motd", 3, false),
        ("script.sh", 1229, false),
    ]);
    let loaded = values(vec![
        ("app.yaml", ValuePreview::Line("debug=true".to_owned())),
        ("config.json", ValuePreview::Json { size_bytes: 412 }),
        ("logo.png", ValuePreview::Binary { size_bytes: 2048 }),
        (
            "script.sh",
            ValuePreview::Text {
                size_bytes: 1229,
                lines: 3,
            },
        ),
    ]);
    let lines = data_lines(&summary, Some(&loaded));
    assert_eq!(
        texts(&lines),
        [
            ("app.yaml", "debug=true"),
            ("config.json", "JSON · 412 B"),
            ("logo.png", "binary · 2.0 KiB"),
            // A key the watch has not delivered keeps its size.
            ("motd", "3 B"),
            ("script.sh", "text · 3 lines · 1.2 KiB"),
        ]
    );
}

#[test]
fn config_map_data_shows_sizes_before_the_values_load() {
    let summary = config_map(&[("app.yaml", 412, false), ("logo.png", 2048, true)]);
    let lines = data_lines(&summary, None);
    assert_eq!(
        texts(&lines),
        [("app.yaml", "412 B"), ("logo.png", "2.0 KiB · binary")]
    );
}

#[test]
fn text_preview_of_one_line_is_singular() {
    let preview = ValuePreview::Text {
        size_bytes: 300,
        lines: 1,
    };
    assert_eq!(preview_text(&preview), "text · 1 line · 300 B");
}

// ---- Selected pods ----

fn selected_budget(selector: Option<&[&str]>) -> PodDisruptionBudgetSummary {
    let selector = selector.map(|terms| {
        let terms: Vec<String> = terms.iter().map(|term| (*term).to_owned()).collect();
        // An empty term list is the selector that selects everything.
        cluster::Selector::of_labels(&terms).unwrap_or_else(cluster::Selector::everything)
    });
    PodDisruptionBudgetSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        min_available: Some("1".to_owned()),
        max_unavailable: None,
        selector,
        current_healthy: 0,
        desired_healthy: 1,
        expected_pods: 0,
        disruptions_allowed: 0,
        unhealthy_pod_eviction_policy: None,
        conditions: Vec::new(),
        is_status_stale: false,
    }
}

fn labelled_pod(namespace: &str, name: &str, label: &str, is_ready: bool) -> PodSummary {
    PodSummary {
        namespace: namespace.to_owned(),
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
        conditions: vec![cluster::PodCondition {
            name: "Ready".to_owned(),
            is_true: is_ready,
            reason: None,
            message: None,
            changed_at: None,
        }],
        status_message: None,
        labels: vec![label.to_owned()],
        host_network: false,
        image_pull_secrets: Vec::new(),
        containers: Vec::new(),
    }
}

#[test]
fn selected_pods_unhealthy_first() {
    let pods = [
        labelled_pod("team-a", "web-b", "app=web", true),
        labelled_pod("team-a", "web-c", "app=web", false),
        labelled_pod("team-a", "web-a", "app=web", true),
        labelled_pod("team-a", "db-a", "app=db", false),
        labelled_pod("team-b", "web-d", "app=web", false),
    ];
    let budget = selected_budget(Some(&["app=web"]));
    let selected = selected_pods(&budget, &pods);
    let order: Vec<(&str, bool)> = selected
        .iter()
        .map(|entry| (entry.pod.name.as_str(), entry.is_healthy))
        .collect();
    assert_eq!(order, [("web-c", false), ("web-a", true), ("web-b", true)]);
}

#[test]
fn null_selector_selects_no_pods() {
    let pods = [labelled_pod("team-a", "web-a", "app=web", true)];
    assert!(selected_pods(&selected_budget(None), &pods).is_empty());
    // An empty selector selects every pod of the namespace.
    assert_eq!(selected_pods(&selected_budget(Some(&[])), &pods).len(), 1);
}

// ---- Scaling events and blocked creations ----

fn event(reason: &str, message: &str, last_seen: &str) -> EventSummary {
    EventSummary {
        namespace: "team-a".to_owned(),
        name: format!("event-{last_seen}"),
        event_type: cluster::EventType::Warning,
        reason: reason.to_owned(),
        object: cluster::InvolvedObject {
            kind: "ReplicaSet".to_owned(),
            namespace: Some("team-a".to_owned()),
            name: "web-7d9f8c".to_owned(),
        },
        message: message.to_owned(),
        count: 1,
        first_seen: None,
        last_seen: Some(at(last_seen)),
        source: None,
        container: None,
    }
}

#[test]
fn scaling_events_newest_first_rescale_only() {
    let events = [
        event("SuccessfulRescale", "New size: 4", "2026-10-02T10:00:00Z"),
        event(
            "FailedGetResourceMetric",
            "no metrics",
            "2026-10-02T10:30:00Z",
        ),
        event("SuccessfulRescale", "New size: 6", "2026-10-02T11:00:00Z"),
    ];
    let order: Vec<&str> = scaling_events(&events)
        .iter()
        .map(|event| event.message.as_str())
        .collect();
    assert_eq!(order, ["New size: 6", "New size: 4"]);
}

fn rejection(quota: &str) -> String {
    format!("Error creating: pods \"x\" is forbidden: exceeded quota: {quota}, requested: pods=1")
}

#[test]
fn blocked_creations_match_quota_name_exactly() {
    let needles = QuotaNeedles::of("compute-quota");
    assert!(needles.matches(&rejection("compute-quota")));
    // A longer name that starts with the quota name is another quota.
    assert!(!needles.matches(&rejection("compute-quota-2")));
    assert!(needles.matches("exceeded quota: compute-quota"));
    assert!(!needles.matches("something else"));
    let events = [
        event(
            "FailedCreate",
            &rejection("compute-quota-2"),
            "2026-10-02T10:00:00Z",
        ),
        event(
            "FailedCreate",
            &rejection("compute-quota"),
            "2026-10-02T11:00:00Z",
        ),
    ];
    assert_eq!(blocked_creations(&events, "compute-quota").len(), 1);
}

#[test]
fn blocked_creations_newest_first() {
    let events = [
        event(
            "FailedCreate",
            &rejection("compute-quota"),
            "2026-10-02T11:00:00Z",
        ),
        event(
            "FailedCreate",
            &rejection("compute-quota"),
            "2026-10-02T12:00:00Z",
        ),
    ];
    let blocked = blocked_creations(&events, "compute-quota");
    assert_eq!(blocked[0].last_seen, Some(at("2026-10-02T12:00:00Z")));
    assert_eq!(blocked[1].last_seen, Some(at("2026-10-02T11:00:00Z")));
}

#[test]
fn blocked_creations_include_failed_quota() {
    let needles = QuotaNeedles::of("compute-quota");
    assert!(
        needles.matches(
            "pods \"x\" is forbidden: failed quota: compute-quota: must specify limits.cpu"
        )
    );
    assert!(!needles.matches("failed quota: compute-quota-2: must specify limits.cpu"));
}

#[test]
fn event_time_label_uses_the_zone_and_reads_a_missing_time() {
    let now = at("2026-10-02T12:00:00Z");
    assert_eq!(
        event_time_label(Some(at("2026-10-02T10:45:00Z")), now, &TimeZone::UTC),
        "10:45 UTC"
    );
    assert_eq!(
        event_time_label(Some(at("2026-10-01T02:30:00Z")), now, &TimeZone::UTC),
        "Oct 1 02:30 UTC"
    );
    assert_eq!(event_time_label(None, now, &TimeZone::UTC), "—");
}

#[test]
fn cut_text_adds_an_ellipsis_only_when_cut() {
    assert_eq!(cut_text("short", 10), "short");
    assert_eq!(cut_text("abcdef", 3), "abc…");
}

// ---- PersistentVolumeClaims ----

fn mounting_pod(namespace: &str, name: &str, claim: &str, paths: &[&str]) -> PodSummary {
    let mount = |path: &str| cluster::MountEntry {
        path: path.to_owned(),
        volume: "data".to_owned(),
        source: VolumeSource::PersistentVolumeClaim {
            claim: claim.to_owned(),
        },
        is_read_only: false,
        sub_path: None,
    };
    let container = |index: usize, path: &str| cluster::ContainerSummary {
        name: format!("c{index}"),
        image: "img".to_owned(),
        kind: cluster::ContainerKind::Main,
        state: cluster::ContainerState::Running { started_at: None },
        is_ready: true,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: cluster::ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: vec![mount(path)],
    };
    PodSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        status: cluster::PodStatus::Reason(cluster::StatusReason::Running),
        ready: cluster::ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: Some("wk-01".to_owned()),
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        containers: paths
            .iter()
            .enumerate()
            .map(|(index, path)| container(index, path))
            .collect(),
    }
}

fn bound_claim() -> PersistentVolumeClaimSummary {
    PersistentVolumeClaimSummary {
        namespace: "shop".to_owned(),
        name: "data".to_owned(),
        created_at: None,
        labels: Vec::new(),
        phase: "Bound".to_owned(),
        is_terminating: false,
        volume: Some("pv-1".to_owned()),
        capacity: Some("10Gi".to_owned()),
        requested: None,
        access_modes: Vec::new(),
        storage_class: None,
        volume_mode: None,
        conditions: Vec::new(),
    }
}

fn sample(used: u64, capacity: u64) -> PvcUsage {
    PvcUsage {
        namespace: "shop".to_owned(),
        claim: "data".to_owned(),
        sampled_at: Some(at("2026-01-01T00:00:00Z")),
        used: Some(cluster::ByteAmount::from_bytes(used)),
        capacity: Some(cluster::ByteAmount::from_bytes(capacity)),
        available: None,
        inodes_used: Some(50),
        inodes: Some(200),
    }
}

fn note_text(rows: &[DetailRow]) -> String {
    match rows {
        [DetailRow::Note(text)] => text.to_string(),
        other => panic!("expected one note, got {other:?}"),
    }
}

#[test]
fn claim_pods_dedupes_and_sorts() {
    let pods = [
        mounting_pod("shop", "web-2", "data", &["/data"]),
        // Two containers mount the claim: one entry, the first path.
        mounting_pod("shop", "web-1", "data", &["/var/lib/data", "/backup"]),
        mounting_pod("shop", "web-3", "other", &["/x"]),
    ];
    let mounting = claim_pods("shop", "data", &pods);
    let listed: Vec<(&str, &str)> = mounting
        .iter()
        .map(|(pod, path)| (pod.name.as_str(), *path))
        .collect();
    assert_eq!(listed, [("web-1", "/var/lib/data"), ("web-2", "/data")]);
}

#[test]
fn claim_pods_ignore_other_namespaces() {
    let pods = [mounting_pod("other", "web-1", "data", &["/data"])];
    assert!(claim_pods("shop", "data", &pods).is_empty());
}

#[test]
fn claim_usage_bars_and_inodes() {
    let now = at("2026-01-01T00:00:12Z");
    let rows = claim_usage_rows(
        &bound_claim(),
        Some(&sample(8 * 1024 * 1024 * 1024, 10 * 1024 * 1024 * 1024)),
        &FeedStatus::Live,
        now,
    );
    assert_eq!(
        rows,
        [
            DetailRow::Bar {
                label: "Used".into(),
                percent: 80,
                text: "8 of 10Gi".into(),
                tone: Some(StatusTone::Warn),
            },
            DetailRow::Bar {
                label: "Inodes".into(),
                percent: 25,
                text: "25%".into(),
                tone: None,
            },
            DetailRow::Note("Sampled 12s ago".into()),
        ]
    );
}

#[test]
fn claim_usage_note_when_not_bound() {
    let mut pending = bound_claim();
    pending.phase = "Pending".to_owned();
    let rows = claim_usage_rows(
        &pending,
        None,
        &FeedStatus::Live,
        at("2026-01-01T00:00:00Z"),
    );
    assert_eq!(note_text(&rows), "No usage until the claim is bound");
}

#[test]
fn claim_usage_note_without_samples() {
    let now = at("2026-01-01T00:00:00Z");
    let waiting = claim_usage_rows(&bound_claim(), None, &FeedStatus::Waiting, now);
    assert!(note_text(&waiting).starts_with("No usage data yet"));
    let off = claim_usage_rows(
        &bound_claim(),
        None,
        &FeedStatus::Unavailable("Not permitted: get nodes/proxy".to_owned()),
        now,
    );
    assert_eq!(
        note_text(&off),
        "Usage unavailable: Not permitted: get nodes/proxy"
    );
}

#[test]
fn block_claim_reads_no_usage() {
    let mut block = bound_claim();
    block.volume_mode = Some("Block".to_owned());
    let rows = claim_usage_rows(
        &block,
        Some(&sample(1, 2)),
        &FeedStatus::Live,
        at("2026-01-01T00:00:00Z"),
    );
    assert_eq!(note_text(&rows), "Block volumes report no usage");
}

#[test]
fn shared_filesystem_claim_is_labelled_node_filesystem() {
    // The claim asks for 10Gi, but the kubelet reports a 100Gi filesystem: the node's disk.
    let gi = 1024 * 1024 * 1024;
    let rows = claim_usage_rows(
        &bound_claim(),
        Some(&sample(40 * gi, 100 * gi)),
        &FeedStatus::Live,
        at("2026-01-01T00:00:12Z"),
    );
    assert_eq!(
        rows[0],
        DetailRow::Bar {
            label: "Node filesystem".into(),
            percent: 40,
            text: "40 of 100Gi".into(),
            tone: None,
        }
    );
    assert!(rows.contains(&DetailRow::Note(
        "Shared with the node: the claim has no quota of its own".into()
    )));
}

#[test]
fn own_filesystem_claim_has_no_node_note() {
    let rows = claim_usage_rows(
        &bound_claim(),
        Some(&sample(1, 10 * 1024 * 1024 * 1024)),
        &FeedStatus::Live,
        at("2026-01-01T00:00:12Z"),
    );
    assert!(matches!(&rows[0], DetailRow::Bar { label, .. } if label.as_ref() == "Used"));
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, DetailRow::Note(text) if text.contains("Shared")))
    );
}

// ---- StorageClasses ----

fn volume_in(name: &str, class: &str, phase: &str) -> PersistentVolumeSummary {
    PersistentVolumeSummary {
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        capacity: None,
        access_modes: Vec::new(),
        reclaim_policy: "Retain".to_owned(),
        phase: phase.to_owned(),
        is_terminating: false,
        claim: None,
        storage_class: Some(class.to_owned()),
        volume_mode: None,
        backend: cluster::VolumeBackend::Other { kind: "unknown" },
        node_affinity: Vec::new(),
        mount_options: Vec::new(),
        reason: None,
        message: None,
    }
}

#[test]
fn class_volumes_counts_bound_and_released() {
    let volumes = [
        volume_in("pv-1", "gp3", "Bound"),
        volume_in("pv-2", "gp3", "Bound"),
        volume_in("pv-3", "gp3", "Released"),
        volume_in("pv-4", "gp3", "Available"),
    ];
    assert_eq!(
        class_volumes_text(&class_volumes("gp3", &volumes)),
        "4 · 2 bound · 1 released"
    );
}

#[test]
fn class_volumes_filters_by_class() {
    let volumes = [
        volume_in("pv-1", "gp3", "Bound"),
        volume_in("pv-2", "other", "Bound"),
        volume_in("pv-3", "gp3", "Released"),
    ];
    let own = class_volumes("gp3", &volumes);
    let names: Vec<&str> = own.iter().map(|volume| volume.name.as_str()).collect();
    assert_eq!(names, ["pv-1", "pv-3"]);
}

#[test]
fn class_volumes_text_skips_zero_parts() {
    let volumes = [volume_in("pv-1", "gp3", "Available")];
    assert_eq!(class_volumes_text(&class_volumes("gp3", &volumes)), "1");
    assert_eq!(class_volumes_text(&class_volumes("none", &volumes)), "0");
    let released = [volume_in("pv-1", "gp3", "Released")];
    assert_eq!(
        class_volumes_text(&class_volumes("gp3", &released)),
        "1 · 1 released"
    );
}

// ---- Service accounts ----

fn bound(group: Option<&str>) -> BoundRole {
    BoundRole {
        role: cluster::RoleRef {
            kind: cluster::RoleKind::ClusterRole,
            name: "view".to_owned(),
        },
        role_key: None,
        binding: ResourceKey::Kind {
            kind: ResourceKind::ClusterRoleBindings,
            namespace: None,
            name: "view-all".to_owned(),
        },
        binding_text: "clusterrolebinding/view-all".to_owned(),
        group: group.map(str::to_owned),
    }
}

#[test]
fn bound_roles_rows_via_binding_and_group() {
    assert_eq!(group_suffix(&bound(None)), None);
    assert_eq!(
        group_suffix(&bound(Some("system:serviceaccounts:shop"))).as_deref(),
        Some(" · group system:serviceaccounts:shop")
    );
    // The binding is a link to its own row.
    assert_eq!(
        bound(None).binding,
        ResourceKey::Kind {
            kind: ResourceKind::ClusterRoleBindings,
            namespace: None,
            name: "view-all".to_owned(),
        }
    );
}

#[test]
fn bound_roles_note_names_scope() {
    assert_eq!(
        bindings_scope_note("all namespaces"),
        "Role bindings from all namespaces"
    );
    assert_eq!(bindings_scope_note("shop"), "Role bindings from shop");
}

fn pod_running_as(namespace: &str, name: &str, account: Option<&str>) -> PodSummary {
    use cluster::{PodStatus, ReadyCount, StatusReason};

    PodSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: account.map(str::to_owned),
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        containers: Vec::new(),
    }
}

#[test]
fn service_account_pods_by_name() {
    let account = ServiceAccountSummary {
        namespace: "shop".to_owned(),
        name: "default".to_owned(),
        created_at: None,
        labels: Vec::new(),
        secrets: Vec::new(),
        image_pull_secrets: Vec::new(),
        automount_token: None,
        cloud_identities: Vec::new(),
    };
    let pods = [
        pod_running_as("shop", "zeta", None),
        pod_running_as("shop", "alpha", Some("default")),
        pod_running_as("shop", "other", Some("api")),
        pod_running_as("elsewhere", "beta", Some("default")),
    ];
    let names: Vec<&str> = account_pods(&account, &pods)
        .iter()
        .map(|pod| pod.name.as_str())
        .collect();
    assert_eq!(names, ["alpha", "zeta"]);
}

// ---- Secrets ----

#[test]
fn secret_used_by_note_when_unused() {
    assert_eq!(unused_notes(IngressesState::Ready), [UNUSED_NOTE]);
    assert_eq!(unused_notes(IngressesState::Loading), ["Loading…"]);
    assert_eq!(
        unused_notes(IngressesState::Denied),
        [UNUSED_NOTE_PODS_ONLY, "Not permitted: list ingresses"]
    );
    assert_eq!(
        unused_notes(IngressesState::Unavailable),
        [UNUSED_NOTE_PODS_ONLY, "Ingresses are unavailable"]
    );
    assert!(UNUSED_NOTE.contains("Gateway API and Istio"));
    assert!(UNUSED_NOTE_PODS_ONLY.starts_with("No pod in this namespace uses it."));
}

#[test]
fn ingresses_state_follows_the_list_and_the_plan() {
    let ready: LiveList<IngressSummary> = LiveList::Ready {
        items: Vec::new(),
        interruption: None,
    };
    let failed: LiveList<IngressSummary> = LiveList::Failed {
        message: "boom".to_owned(),
    };
    let start = CompanionPlan::Start(crate::cluster_session::CompanionKind::Ingresses);
    let denied = CompanionPlan::Denied(cluster::AccessCheck::ListIngresses);
    assert_eq!(ingresses_state(Some(&ready), start), IngressesState::Ready);
    assert_eq!(
        ingresses_state(Some(&LiveList::Loading), start),
        IngressesState::Loading
    );
    assert_eq!(ingresses_state(None, denied), IngressesState::Denied);
    assert_eq!(
        ingresses_state(Some(&failed), start),
        IngressesState::Unavailable
    );
    assert_eq!(ingresses_state(None, start), IngressesState::Unavailable);
}

// ---- Can do ----

fn rbac_rule(resources: &[&str], verbs: &[&str]) -> cluster::RbacRule {
    let texts = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect();
    cluster::RbacRule {
        api_groups: vec![String::new()],
        resources: texts(resources),
        resource_names: Vec::new(),
        verbs: texts(verbs),
        non_resource_urls: Vec::new(),
    }
}

fn group_subject(name: &str) -> cluster::Subject {
    cluster::Subject {
        kind: cluster::SubjectKind::Group,
        name: name.to_owned(),
        namespace: None,
    }
}

fn snapshot_with(
    roles: Vec<(&str, Vec<cluster::RbacRule>)>,
    bindings: Vec<BindingSummary>,
) -> cluster::RbacSnapshot {
    cluster::RbacSnapshot {
        roles: Vec::new(),
        cluster_roles: roles
            .into_iter()
            .map(|(name, rules)| RoleSummary {
                namespace: None,
                name: name.to_owned(),
                created_at: None,
                labels: Vec::new(),
                rules,
                aggregation: Vec::new(),
            })
            .collect(),
        role_bindings: Vec::new(),
        cluster_role_bindings: bindings,
        coverage: cluster::RbacCoverage {
            cluster_roles: true,
            cluster_bindings: true,
            roles: cluster::NamespaceCoverage::AllNamespaces,
            role_bindings: cluster::NamespaceCoverage::AllNamespaces,
        },
    }
}

fn ready(snapshot: cluster::RbacSnapshot) -> RbacState {
    RbacState::Ready {
        snapshot: std::rc::Rc::new(snapshot),
        listed_at: jiff::Timestamp::UNIX_EPOCH,
    }
}

fn chip_texts(content: &CanDoContent) -> Vec<String> {
    let CanDoContent::Ready(summary) = content else {
        panic!("expected chips");
    };
    summary
        .chips
        .chips
        .iter()
        .map(|(text, _)| text.to_string())
        .collect()
}

#[test]
fn can_do_excludes_authenticated_only_grants() {
    let data = snapshot_with(
        vec![
            (
                "basic",
                vec![rbac_rule(&["selfsubjectreviews"], &["create"])],
            ),
            ("reader", vec![rbac_rule(&["pods"], &["get"])]),
        ],
        vec![
            binding_named(
                None,
                "everyone",
                (cluster::RoleKind::ClusterRole, "basic"),
                vec![group_subject("system:authenticated")],
            ),
            binding_named(
                None,
                "own",
                (cluster::RoleKind::ClusterRole, "reader"),
                vec![account_subject("shop", "robot")],
            ),
        ],
    );
    let content = can_do_content(&ready(data), "shop", "robot", &CanDoCell::default());
    assert_eq!(chip_texts(&content), ["get pods"]);
}

fn summary_of(content: CanDoContent) -> Rc<CanDoSummary> {
    match content {
        CanDoContent::Ready(summary) => summary,
        CanDoContent::Line(_) => panic!("expected chips"),
    }
}

#[test]
fn can_do_chips_are_reused_for_the_same_snapshot_and_account() {
    let state = ready(snapshot_with(Vec::new(), Vec::new()));
    let cache = CanDoCell::default();
    let first = summary_of(can_do_content(&state, "shop", "robot", &cache));
    let again = summary_of(can_do_content(&state, "shop", "robot", &cache));
    assert!(Rc::ptr_eq(&first, &again));
    // Another account is a different key.
    let other = summary_of(can_do_content(&state, "shop", "worker", &cache));
    assert!(!Rc::ptr_eq(&first, &other));
}

#[test]
fn can_do_chips_are_recomputed_for_a_refreshed_snapshot() {
    let cache = CanDoCell::default();
    let first = summary_of(can_do_content(
        &ready(snapshot_with(Vec::new(), Vec::new())),
        "shop",
        "robot",
        &cache,
    ));
    // An equal snapshot listed again is a new `Rc`.
    let refreshed = summary_of(can_do_content(
        &ready(snapshot_with(Vec::new(), Vec::new())),
        "shop",
        "robot",
        &cache,
    ));
    assert!(!Rc::ptr_eq(&first, &refreshed));
}

#[test]
fn can_do_loading_and_failed_lines() {
    let loading = RbacState::Loading {
        _task: gpui_kit::Task::ready(()),
    };
    for state in [RbacState::Idle, loading] {
        let CanDoContent::Line(text) =
            can_do_content(&state, "shop", "robot", &CanDoCell::default())
        else {
            panic!("a line");
        };
        assert_eq!(text, "Listing RBAC objects…");
    }
    let failed = RbacState::Failed("timed out".to_owned());
    let CanDoContent::Line(text) = can_do_content(&failed, "shop", "robot", &CanDoCell::default())
    else {
        panic!("a line");
    };
    assert_eq!(text, "RBAC objects are unavailable: timed out");
}

#[test]
fn can_do_coverage_warning() {
    let mut data = snapshot_with(Vec::new(), Vec::new());
    data.coverage.cluster_bindings = false;
    let state = ready(data);
    let summary = summary_of(can_do_content(
        &state,
        "shop",
        "robot",
        &CanDoCell::default(),
    ));
    assert!(summary.chips.chips.is_empty());
    assert_eq!(
        summary.warnings,
        ["ClusterRoleBindings were not listed; only namespace grants are shown."]
    );
}

// ---- Roll back buttons ----

fn gate(availability: ActionAvailability) -> RollBackGate {
    let cluster = crate::cluster_registry::ClusterRef {
        kubeconfig: std::path::PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    };
    RollBackGate {
        subject: ClusterObject::new(
            cluster,
            ResourceKey::Kind {
                kind: ResourceKind::Deployments,
                namespace: Some("team-a".to_owned()),
                name: "api".to_owned(),
            },
        ),
        availability,
    }
}

fn button_of(
    deployment: &DeploymentSummary,
    sets: &[ReplicaSetSummary],
    wanted: &str,
    gate: Option<&RollBackGate>,
) -> RollBackButton {
    let revisions = revision_rows(deployment, sets);
    let revision = revisions
        .iter()
        .find(|revision| revision.replica_set.name == wanted)
        .expect("the revision is listed");
    roll_back_button(deployment, revision, gate)
}

fn disabled_reason(button: RollBackButton) -> String {
    match button {
        RollBackButton::Disabled(reason) => reason.to_string(),
        RollBackButton::Enabled(..) => panic!("expected a disabled button"),
    }
}

#[test]
fn revision_roll_back_button_follows_the_gate() {
    let sets = [
        replica_set("api-new", Some("38"), owner("Deployment", "api")),
        replica_set("api-old", Some("37"), owner("Deployment", "api")),
        replica_set("api-unnumbered", None, owner("Deployment", "api")),
    ];
    let running = deployment(Some("38"));
    let open = gate(ActionAvailability::Enabled);
    match button_of(&running, &sets, "api-old", Some(&open)) {
        RollBackButton::Enabled(subject, target) => {
            assert_eq!(subject, open.subject);
            assert_eq!(target.replica_set, "api-old");
            assert_eq!(target.revision, 37);
            assert_eq!(target.tag.as_deref(), Some("1.4.2"));
        }
        RollBackButton::Disabled(reason) => panic!("disabled: {reason}"),
    }
    // The gate of the drawer's cluster speaks first: a denied right, a locked cluster.
    for reason in ["Not permitted: patch deployments", "stg-b is read-only"] {
        let closed = gate(ActionAvailability::Disabled {
            reason: reason.into(),
        });
        assert_eq!(
            disabled_reason(button_of(&running, &sets, "api-old", Some(&closed))),
            reason
        );
    }
    // Without a gate (the session has no guard yet) there is nothing to run on.
    assert_eq!(
        disabled_reason(button_of(&running, &sets, "api-old", None)),
        "Not connected"
    );
    // A ReplicaSet with no revision number cannot be named.
    assert_eq!(
        disabled_reason(button_of(&running, &sets, "api-unnumbered", Some(&open))),
        "This ReplicaSet has no revision number"
    );
    // A paused rollout refuses a roll back, as kubectl does.
    let mut paused = running.clone();
    paused.is_paused = true;
    assert_eq!(
        disabled_reason(button_of(&paused, &sets, "api-old", Some(&open))),
        "Resume the rollout first"
    );
}

#[test]
fn no_button_on_the_current_revision() {
    let sets = [
        replica_set("api-new", Some("38"), owner("Deployment", "api")),
        replica_set("api-old", Some("37"), owner("Deployment", "api")),
    ];
    let revisions = revision_rows(&deployment(Some("38")), &sets);
    // `revision_element` draws the button only on the rows that are not current.
    let with_button: Vec<&str> = revisions
        .iter()
        .filter(|revision| !revision.is_current)
        .map(|revision| revision.replica_set.name.as_str())
        .collect();
    assert_eq!(with_button, ["api-old"]);
}
