use cluster::{ContainerKind, ContainerProbes, EnvEntry, EnvFromEntry, MountEntry, StatusReason};

use super::*;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn container() -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: "api".to_owned(),
        image: "img".to_owned(),
        kind: ContainerKind::Main,
        state: ContainerState::NotReported,
        is_ready: false,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn termination() -> Termination {
    Termination {
        reason: Some(StatusReason::Error),
        exit_code: 1,
        signal: None,
        started_at: None,
        finished_at: None,
    }
}

fn resource(name: &str, request: Option<&str>, limit: Option<&str>) -> ContainerResource {
    ContainerResource {
        name: name.to_owned(),
        request: request.map(str::to_owned),
        limit: limit.map(str::to_owned),
    }
}

fn probe(action: ProbeAction, period_seconds: u32) -> ProbeSummary {
    ProbeSummary {
        action,
        period_seconds,
        failure_threshold: 3,
        initial_delay_seconds: 0,
        timeout_seconds: 1,
    }
}

fn env(name: &str, source: EnvSource) -> EnvEntry {
    EnvEntry {
        name: name.to_owned(),
        source,
    }
}

fn config_map_key(name: &str, key: &str) -> EnvSource {
    EnvSource::ConfigMapKey {
        name: name.to_owned(),
        key: key.to_owned(),
    }
}

fn secret_key(name: &str, key: &str) -> EnvSource {
    EnvSource::SecretKey {
        name: name.to_owned(),
        key: key.to_owned(),
    }
}

fn mount(path: &str, volume: &str, source: VolumeSource) -> MountEntry {
    MountEntry {
        path: path.to_owned(),
        volume: volume.to_owned(),
        source,
        is_read_only: false,
        sub_path: None,
    }
}

#[test]
fn resource_text_forms() {
    assert_eq!(
        resource_text(&resource("cpu", Some("250m"), Some("1"))),
        "request 250m · limit 1"
    );
    assert_eq!(
        resource_text(&resource("cpu", Some("250m"), None)),
        "request 250m · no limit"
    );
    assert_eq!(
        resource_text(&resource("memory", None, Some("512Mi"))),
        "no request · limit 512Mi"
    );
}

#[test]
fn resource_label_names() {
    assert_eq!(resource_label("cpu"), "CPU");
    assert_eq!(resource_label("memory"), "Memory");
    assert_eq!(resource_label("ephemeral-storage"), "Ephemeral storage");
    assert_eq!(resource_label("pods"), "Pods");
    assert_eq!(resource_label("hugepages-2Mi"), "Hugepages 2Mi");
    assert_eq!(resource_label("example.com/gpu"), "example.com/gpu");
}

#[test]
fn probe_summary_text_forms() {
    let exec = probe(
        ProbeAction::Exec {
            command: vec!["test".to_owned(), "-f".to_owned(), "/tmp/ready".to_owned()],
        },
        5,
    );
    assert_eq!(
        probe_summary_text(&exec),
        "exec `test -f /tmp/ready` · every 5s"
    );
    let long = probe(
        ProbeAction::Exec {
            command: vec!["x".repeat(150)],
        },
        10,
    );
    assert_eq!(
        probe_summary_text(&long),
        format!("exec `{}…` · every 10s", "x".repeat(100))
    );
}

fn chip_texts(probe: &ProbeSummary) -> Vec<String> {
    probe_chips(probe).iter().map(ToString::to_string).collect()
}

#[test]
fn http_probe_chips_name_scheme_method_port_path_and_timings() {
    let mut http = probe(
        ProbeAction::HttpGet {
            scheme: "HTTP".to_owned(),
            port: "8080".to_owned(),
            path: "/ready".to_owned(),
        },
        10,
    );
    http.initial_delay_seconds = 5;
    assert_eq!(
        chip_texts(&http),
        [
            "HTTP",
            "GET",
            "port 8080",
            "path /ready",
            "delay 5s",
            "timeout 1s",
            "period 10s",
            "failures 3"
        ]
    );
}

#[test]
fn https_probe_chips_keep_the_scheme_and_a_named_port() {
    let https = probe(
        ProbeAction::HttpGet {
            scheme: "HTTPS".to_owned(),
            port: "web".to_owned(),
            path: "/".to_owned(),
        },
        10,
    );
    assert_eq!(
        chip_texts(&https)[..4],
        ["HTTPS", "GET", "port web", "path /"]
    );
}

#[test]
fn tcp_and_grpc_probe_chips_have_protocol_and_port_only() {
    let tcp = probe(
        ProbeAction::TcpSocket {
            port: "5432".to_owned(),
        },
        10,
    );
    assert_eq!(chip_texts(&tcp)[..3], ["TCP", "port 5432", "delay 0s"]);
    let grpc = probe(ProbeAction::Grpc { port: 9090 }, 10);
    assert_eq!(chip_texts(&grpc)[..3], ["gRPC", "port 9090", "delay 0s"]);
}

#[test]
fn exec_probe_chips_carry_the_command_cut_at_the_limit() {
    let exec = probe(
        ProbeAction::Exec {
            command: vec!["test".to_owned(), "-f".to_owned(), "/tmp/ready".to_owned()],
        },
        5,
    );
    assert_eq!(chip_texts(&exec)[..2], ["exec", "test -f /tmp/ready"]);
    let long = probe(
        ProbeAction::Exec {
            command: vec!["x".repeat(150)],
        },
        10,
    );
    assert_eq!(chip_texts(&long)[1], format!("{}…", "x".repeat(100)));
}

#[test]
fn unknown_probe_action_is_one_chip_before_the_timings() {
    let unknown = probe(ProbeAction::Unknown, 7);
    assert_eq!(
        chip_texts(&unknown),
        [
            "unknown action",
            "delay 0s",
            "timeout 1s",
            "period 7s",
            "failures 3"
        ]
    );
}

#[test]
fn probe_result_labels_use_muted_ok_warn_and_bad_tones() {
    let label = |result| probe_result_label(result);
    assert_eq!(label(ProbeResult::Passing).tone, StatusTone::Ok);
    assert_eq!(label(ProbeResult::Pending).tone, StatusTone::Warn);
    assert_eq!(label(ProbeResult::NotSet).tone, StatusTone::Done);
    assert_eq!(label(ProbeResult::Failing { failures: 0 }).text, "Failing");
    assert_eq!(
        label(ProbeResult::Failing { failures: 3 }).text,
        "Failing · ×3"
    );
    assert_eq!(
        label(ProbeResult::Failing { failures: 3 }).tone,
        StatusTone::Bad
    );
    assert_eq!(
        label(ProbeResult::WaitingForStartup).text,
        "Waiting for startup"
    );
}

#[test]
fn env_summary_groups_sources_and_caps_at_three() {
    let mut summary = container();
    assert_eq!(env_summary(&summary), None);

    summary.env = vec![env("MODE", EnvSource::Literal)];
    assert_eq!(env_summary(&summary).as_deref(), Some("1 env var"));

    summary.env = vec![
        env("A", EnvSource::Literal),
        env("B", config_map_key("api-config", "b")),
        env("C", config_map_key("api-config", "c")),
        env("D", secret_key("api-db", "d")),
        env(
            "POD",
            EnvSource::Field {
                path: "metadata.name".to_owned(),
            },
        ),
    ];
    summary.env_from = vec![EnvFromEntry {
        source: EnvFromSource::Secret {
            name: "extra".to_owned(),
        },
        prefix: None,
    }];
    assert_eq!(
        env_summary(&summary).as_deref(),
        Some(
            "5 env vars · 2 from configmap/api-config · 1 from secret/api-db · all of secret/extra"
        )
    );

    summary.env_from.push(EnvFromEntry {
        source: EnvFromSource::ConfigMap {
            name: "more".to_owned(),
        },
        prefix: None,
    });
    assert_eq!(
        env_summary(&summary).as_deref(),
        Some(
            "5 env vars · 2 from configmap/api-config · 1 from secret/api-db · all of secret/extra · +1 more"
        )
    );
}

#[test]
fn mount_summary_first_and_more() {
    let mut summary = container();
    assert_eq!(mount_summary(&summary), None);

    let mut first = mount(
        "/etc/api",
        "config",
        VolumeSource::ConfigMap {
            name: "api-config".to_owned(),
        },
    );
    first.is_read_only = true;
    summary.mounts = vec![first];
    assert_eq!(
        mount_summary(&summary).as_deref(),
        Some("/etc/api ← configmap/api-config (read-only)")
    );

    summary.mounts.extend([
        mount("/tmp", "scratch", VolumeSource::EmptyDir),
        mount("/data", "data", VolumeSource::Other),
    ]);
    assert_eq!(
        mount_summary(&summary).as_deref(),
        Some("/etc/api ← configmap/api-config (read-only) · +2 more")
    );
}

#[test]
fn env_rows_list_env_from_first_with_targets() {
    let mut summary = container();
    summary.env = vec![
        env("MODE", EnvSource::Literal),
        env("FROM_MAP", config_map_key("api-config", "mode")),
        env("PASSWORD", secret_key("api-db", "password")),
        env(
            "POD",
            EnvSource::Field {
                path: "metadata.name".to_owned(),
            },
        ),
        env(
            "CPU",
            EnvSource::ResourceField {
                resource: "limits.cpu".to_owned(),
            },
        ),
        env("ODD", EnvSource::Unknown),
    ];
    summary.env_from = vec![
        EnvFromEntry {
            source: EnvFromSource::ConfigMap {
                name: "shared".to_owned(),
            },
            prefix: Some("APP_".to_owned()),
        },
        EnvFromEntry {
            source: EnvFromSource::Secret {
                name: "creds".to_owned(),
            },
            prefix: None,
        },
    ];
    let rows = env_rows(&summary, "shop");
    let config_map = |name: &str| ResourceKey::of_object("ConfigMap", Some("shop"), name);
    let secret = |name: &str| ResourceKey::of_object("Secret", Some("shop"), name);
    let listed: Vec<_> = rows
        .iter()
        .map(|row| (row.name.as_str(), row.source.as_str(), row.target.clone()))
        .collect();
    assert_eq!(
        listed,
        [
            (
                "APP_*",
                "all keys of configmap/shared",
                config_map("shared")
            ),
            ("*", "all keys of secret/creds", secret("creds")),
            ("MODE", "literal · value in the YAML tab", None),
            (
                "FROM_MAP",
                "configmap/api-config · mode",
                config_map("api-config")
            ),
            ("PASSWORD", "secret/api-db · password", secret("api-db")),
            ("POD", "field metadata.name", None),
            ("CPU", "resource limits.cpu", None),
            ("ODD", "unknown source", None),
        ]
    );
    assert!(config_map("shared").is_some());
}

#[test]
fn mount_rows_text_and_targets() {
    let mut config = mount(
        "/etc/api",
        "config",
        VolumeSource::ConfigMap {
            name: "api-config".to_owned(),
        },
    );
    config.is_read_only = true;
    config.sub_path = Some("app.yaml".to_owned());
    let mut summary = container();
    summary.mounts = vec![
        config,
        mount(
            "/etc/key",
            "key",
            VolumeSource::Secret {
                name: "api-key".to_owned(),
            },
        ),
        mount(
            "/data",
            "data",
            VolumeSource::PersistentVolumeClaim {
                claim: "data-claim".to_owned(),
            },
        ),
        mount("/tmp", "scratch", VolumeSource::EmptyDir),
        mount(
            "/logs",
            "logs",
            VolumeSource::HostPath {
                path: "/var/log".to_owned(),
            },
        ),
        mount(
            "/token",
            "token",
            VolumeSource::Projected {
                config_maps: Vec::new(),
                secrets: Vec::new(),
            },
        ),
        mount("/labels", "labels", VolumeSource::DownwardApi),
        mount("/share", "share", VolumeSource::Other),
        mount(
            "/blank",
            "blank",
            VolumeSource::ConfigMap {
                name: String::new(),
            },
        ),
    ];
    let rows = mount_rows(&summary, "shop", None);
    let listed: Vec<_> = rows
        .iter()
        .map(|row| (row.name.as_str(), row.source.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            (
                "/etc/api",
                "configmap/api-config · read-only · subPath app.yaml"
            ),
            ("/etc/key", "secret/api-key"),
            ("/data", "pvc/data-claim"),
            ("/tmp", "emptyDir scratch"),
            ("/logs", "hostPath /var/log"),
            ("/token", "projected token"),
            ("/labels", "downwardAPI labels"),
            ("/share", "volume share"),
            ("/blank", "configmap/"),
        ]
    );
    assert_eq!(
        rows[0].target,
        ResourceKey::of_object("ConfigMap", Some("shop"), "api-config")
    );
    assert_eq!(
        rows[1].target,
        ResourceKey::of_object("Secret", Some("shop"), "api-key")
    );
    assert_eq!(
        rows[2].target,
        ResourceKey::of_object("PersistentVolumeClaim", Some("shop"), "data-claim")
    );
    // The other volume kinds have no screen, and an empty name never becomes a link.
    assert!(rows[3..].iter().all(|row| row.target.is_none()));
}

#[test]
fn source_targets_need_a_name() {
    for kind in ["ConfigMap", "Secret", "PersistentVolumeClaim"] {
        assert!(source_target(kind, "shop", "x").is_some(), "{kind}");
        assert_eq!(source_target(kind, "shop", ""), None, "{kind}");
    }
}

#[test]
fn empty_secret_and_claim_names_are_not_links() {
    let secret = VolumeSource::Secret {
        name: String::new(),
    };
    let claim = VolumeSource::PersistentVolumeClaim {
        claim: String::new(),
    };
    assert_eq!(volume_target(&secret, "shop"), None);
    assert_eq!(volume_target(&claim, "shop"), None);
}

fn history_with_claim(used: Option<u64>, capacity: Option<u64>) -> KubeletHistory {
    const GI: u64 = 1 << 30;
    let usage = cluster::PvcUsage {
        namespace: "shop".to_owned(),
        claim: "data-claim".to_owned(),
        sampled_at: Some(at(15)),
        used: used.map(|gi| cluster::ByteAmount::from_bytes(gi * GI)),
        capacity: capacity.map(|gi| cluster::ByteAmount::from_bytes(gi * GI)),
        available: None,
        inodes_used: None,
        inodes: None,
    };
    let pod = cluster::PodKubeletStats {
        namespace: "shop".to_owned(),
        name: "db-0".to_owned(),
        uid: "u".to_owned(),
        network: None,
        volumes: vec![usage],
    };
    let node = cluster::NodeKubeletStats {
        node: "node-a".to_owned(),
        summary: Ok(cluster::KubeletSummary {
            network: None,
            pods: vec![pod],
        }),
        disk_io: None,
    };
    let mut history = KubeletHistory::default();
    history.record(at(15), &[node], &[], &cluster::NamespaceScope::All);
    history
}

fn claim_mount_row(kubelet: Option<&KubeletHistory>) -> SourceRow {
    let mut summary = container();
    summary.mounts = vec![mount(
        "/data",
        "data",
        VolumeSource::PersistentVolumeClaim {
            claim: "data-claim".to_owned(),
        },
    )];
    mount_rows(&summary, "shop", kubelet).remove(0)
}

fn usage_of(used: u64, capacity: u64) -> Option<UsageNote> {
    claim_mount_row(Some(&history_with_claim(Some(used), Some(capacity)))).usage
}

#[test]
fn pvc_mount_shows_usage() {
    let history = history_with_claim(Some(83), Some(100));
    let row = claim_mount_row(Some(&history));
    // The source keeps the claim name alone; the usage is its own line.
    assert_eq!(row.source, "pvc/data-claim");
    assert_eq!(
        row.usage,
        Some(UsageNote {
            text: "83 of 100Gi used (83%)".to_owned(),
            tone: Some(StatusTone::Warn),
        })
    );
}

#[test]
fn pvc_usage_turns_warn_at_80_and_bad_at_90_percent() {
    assert_eq!(
        usage_of(50, 100).map(|usage| (usage.text, usage.tone)),
        Some(("50 of 100Gi used (50%)".to_owned(), None))
    );
    assert_eq!(usage_of(79, 100).and_then(|usage| usage.tone), None);
    assert_eq!(
        usage_of(80, 100).and_then(|usage| usage.tone),
        Some(StatusTone::Warn)
    );
    assert_eq!(
        usage_of(89, 100).and_then(|usage| usage.tone),
        Some(StatusTone::Warn)
    );
    assert_eq!(
        usage_of(90, 100).and_then(|usage| usage.tone),
        Some(StatusTone::Bad)
    );
}

#[test]
fn pvc_mount_without_stats_is_unchanged() {
    let row = claim_mount_row(None);
    assert_eq!(row.source, "pvc/data-claim");
    assert_eq!(row.usage, None);
}

#[test]
fn pvc_mount_of_an_unseen_claim_is_unchanged() {
    let other_claim = history_with_claim(Some(1), Some(2));
    let mut summary = container();
    summary.mounts = vec![mount(
        "/data",
        "data",
        VolumeSource::PersistentVolumeClaim {
            claim: "unknown".to_owned(),
        },
    )];
    let rows = mount_rows(&summary, "shop", Some(&other_claim));
    assert_eq!(rows[0].source, "pvc/unknown");
    assert_eq!(rows[0].usage, None);
}

#[test]
fn pvc_mount_without_a_used_amount_has_no_usage_line() {
    let no_used = history_with_claim(None, Some(100));
    assert_eq!(claim_mount_row(Some(&no_used)).usage, None);
}

#[test]
fn pvc_mount_with_zero_capacity_has_no_usage_line() {
    let zero = history_with_claim(Some(0), Some(0));
    assert_eq!(claim_mount_row(Some(&zero)).usage, None);
}

#[test]
fn other_mounts_have_no_usage_line() {
    let mut summary = container();
    summary.mounts = vec![mount("/tmp", "scratch", VolumeSource::EmptyDir)];
    let history = history_with_claim(Some(83), Some(100));
    assert_eq!(mount_rows(&summary, "shop", Some(&history))[0].usage, None);
}

#[test]
fn last_run_text_needs_both_times() {
    let mut last = termination();
    assert_eq!(last_run_text(&last, at(1_000)), None);
    last.started_at = Some(at(0));
    assert_eq!(last_run_text(&last, at(1_000)), None);
    last.started_at = Some(at(100));
    last.finished_at = Some(at(100 + 4 * 60));
    assert_eq!(
        last_run_text(&last, at(100 + 6 * 60)).as_deref(),
        Some("ran 4m, ended 2m ago")
    );
}

#[test]
fn last_state_text_lists_reason_exit_signal_and_age() {
    let mut last = termination();
    last.exit_code = 137;
    last.reason = Some(StatusReason::OomKilled);
    last.signal = Some(9);
    last.finished_at = Some(at(1_000));
    assert_eq!(
        last_state_text(&last, at(1_000 + 3 * 3_600)),
        "OOMKilled · exit 137 · signal 9 · ended 3h ago"
    );
    let bare = Termination {
        reason: None,
        ..termination()
    };
    assert_eq!(last_state_text(&bare, at(0)), "Terminated · exit 1");
}

#[test]
fn state_text_running_includes_started_age() {
    let mut up = container();
    up.state = ContainerState::Running {
        started_at: Some(at(0)),
    };
    up.is_ready = true;
    let label = container_state_label(&up);
    assert_eq!(state_text(&up, &label, at(120)), "Running · started 2m ago");

    let mut waiting = container();
    waiting.state = ContainerState::Waiting {
        reason: None,
        message: None,
    };
    let label = container_state_label(&waiting);
    assert_eq!(state_text(&waiting, &label, at(120)), "Waiting");
}

#[test]
fn resource_rows_add_usage_rows_for_missing_cpu_and_memory() {
    let mut container = ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        resources: vec![resource("ephemeral-storage", Some("1Gi"), None)],
        ..container()
    };
    let usage = ResourceUsage::default();
    let names = |rows: &[ContainerResource]| -> Vec<String> {
        rows.iter().map(|row| row.name.clone()).collect()
    };
    // Without usage the rows are exactly the container's, not a copy.
    let plain = resource_rows(&container, None);
    assert!(matches!(plain, Cow::Borrowed(_)));
    assert_eq!(names(&plain), ["ephemeral-storage"]);
    // A BestEffort-style container gets empty cpu and memory rows, ordered first.
    let added = resource_rows(&container, Some(usage));
    assert_eq!(names(&added), ["cpu", "memory", "ephemeral-storage"]);
    assert_eq!(added[0].request, None);
    container
        .resources
        .push(resource("memory", Some("64Mi"), None));
    let partial = resource_rows(&container, Some(usage));
    assert_eq!(names(&partial), ["cpu", "memory", "ephemeral-storage"]);
    assert_eq!(partial[1].request.as_deref(), Some("64Mi"));
}

#[test]
fn resource_rows_keep_the_order_when_nothing_is_added() {
    let container = ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        resources: vec![
            resource("memory", None, Some("1Gi")),
            resource("cpu", Some("1"), None),
        ],
        ..container()
    };
    let rows = resource_rows(&container, Some(ResourceUsage::default()));
    assert!(matches!(rows, Cow::Borrowed(_)));
    let names: Vec<_> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["memory", "cpu"]);
}

#[test]
fn container_tabs_put_logs_before_monitor() {
    assert_eq!(
        CONTAINER_TABS,
        [
            ContainerTab::Info,
            ContainerTab::Env,
            ContainerTab::Mounts,
            ContainerTab::Logs,
            ContainerTab::Monitor,
        ]
    );
}
