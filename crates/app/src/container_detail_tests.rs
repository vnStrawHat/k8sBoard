use cluster::{ContainerKind, ContainerProbes, EnvEntry, EnvFromEntry, MountEntry, StatusReason};

use super::*;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn container() -> ContainerSummary {
    ContainerSummary {
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
    assert_eq!(resource_label("example.com/gpu"), "example.com/gpu");
}

#[test]
fn probe_text_forms() {
    let http = probe(
        ProbeAction::HttpGet {
            scheme: "HTTP".to_owned(),
            port: "8080".to_owned(),
            path: "/ready".to_owned(),
        },
        5,
    );
    assert_eq!(
        probe_text(ProbeKind::Readiness, Some(&http)),
        "Readiness · HTTP GET :8080/ready · every 5s"
    );
    let https = probe(
        ProbeAction::HttpGet {
            scheme: "HTTPS".to_owned(),
            port: "web".to_owned(),
            path: "/".to_owned(),
        },
        10,
    );
    assert_eq!(
        probe_text(ProbeKind::Liveness, Some(&https)),
        "Liveness · HTTPS GET :web/ · every 10s"
    );
    let tcp = probe(
        ProbeAction::TcpSocket {
            port: "5432".to_owned(),
        },
        10,
    );
    assert_eq!(
        probe_text(ProbeKind::Liveness, Some(&tcp)),
        "Liveness · TCP :5432 · every 10s"
    );
    let grpc = probe(ProbeAction::Grpc { port: 9090 }, 10);
    assert_eq!(
        probe_text(ProbeKind::Startup, Some(&grpc)),
        "Startup · gRPC :9090 · every 10s"
    );
    let exec = probe(ProbeAction::Exec, 10);
    assert_eq!(
        probe_text(ProbeKind::Liveness, Some(&exec)),
        "Liveness · exec command · every 10s"
    );
    assert_eq!(probe_text(ProbeKind::Startup, None), "Startup");
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
            ("*", "all keys of secret/creds", None),
            ("MODE", "literal · value in the YAML tab", None),
            (
                "FROM_MAP",
                "configmap/api-config · mode",
                config_map("api-config")
            ),
            ("PASSWORD", "secret/api-db · password", None),
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
        mount("/token", "token", VolumeSource::Projected),
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
    let rows = mount_rows(&summary, "shop");
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
    // A Secret or claim has no screen yet, and an empty name never becomes a link.
    assert!(rows[1..].iter().all(|row| row.target.is_none()));
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
