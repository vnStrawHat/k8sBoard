use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

use cluster::{ClusterError, ForwardError};
use serde_json::json;

use super::*;

fn cluster(context: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("a.yaml"),
        context: context.to_owned(),
    }
}

fn spec(kind: TargetKind, name: &str, remote: u16, local: LocalPortSpec) -> ForwardSpec {
    ForwardSpec {
        namespace: "payments".to_owned(),
        target: TargetSpec {
            kind,
            name: name.to_owned(),
        },
        remote_port: remote,
        local_port: local,
    }
}

fn fixture(cluster_name: &str, spec: ForwardSpec, state: ForwardState) -> ForwardFixture {
    ForwardFixture {
        cluster: cluster(cluster_name),
        cluster_label: cluster_name.to_owned().into(),
        environment: Environment::Production,
        spec,
        state,
        local: None,
        pod: None,
        traffic: ForwardTraffic::default(),
        events: Vec::new(),
        started_at: None,
        is_preset: false,
    }
}

fn now() -> jiff::Timestamp {
    jiff::Timestamp::from_second(1_800_000_000).expect("a valid timestamp")
}

fn local(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

fn origin(name: &str) -> ForwardOrigin {
    ForwardOrigin {
        cluster: cluster(name),
        cluster_label: name.to_owned().into(),
        environment: Environment::Staging,
    }
}

fn starting(forwards: &mut PortForwards) -> ForwardId {
    forwards.begin(
        origin("prod-a"),
        spec(TargetKind::Pod, "postgres-0", 5432, LocalPortSpec::Auto),
        None,
        now(),
    )
}

fn state_of(forwards: &PortForwards, id: ForwardId) -> ForwardState {
    forwards.get(id).expect("the row exists").state.clone()
}

fn resolved() -> ForwardUpdate {
    ForwardUpdate::Resolved {
        pod: "postgres-0".to_owned(),
        pod_port: 5432,
    }
}

#[test]
fn apply_transition_table() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    assert_eq!(state_of(&forwards, id), ForwardState::Starting);

    // Listening only records the address; the start has not reported yet.
    let listening = ForwardUpdate::Listening {
        local: local(15432),
        has_ipv6: true,
    };
    assert_eq!(forwards.apply(id, listening, now()), None);
    assert_eq!(forwards.get(id).and_then(|f| f.local), Some(local(15432)));
    assert_eq!(state_of(&forwards, id), ForwardState::Starting);

    // Resolved: Active, the pod, and the first (and only) start report.
    assert_eq!(
        forwards.apply(id, resolved(), now()),
        Some(StartOutcome::Up)
    );
    let row = forwards.get(id).expect("the row exists");
    assert_eq!(row.state, ForwardState::Active);
    assert_eq!(row.pod.as_deref(), Some("postgres-0"));
    assert_eq!(row.started_at, Some(now()));

    let traffic = ForwardTraffic {
        open_connections: 3,
        received: 182,
        sent: 9,
    };
    assert_eq!(
        forwards.apply(id, ForwardUpdate::Traffic(traffic), now()),
        None
    );
    assert_eq!(forwards.get(id).map(|f| f.traffic), Some(traffic));

    let event = |event| ForwardUpdate::Event(event);
    forwards.apply(id, event(ForwardEvent::Reconnecting { attempt: 2 }), now());
    assert_eq!(
        state_of(&forwards, id),
        ForwardState::Reconnecting { attempt: 2 }
    );
    let reconnected = ForwardEvent::Reconnected {
        pod: "postgres-1".to_owned(),
    };
    forwards.apply(id, event(reconnected), now());
    assert_eq!(state_of(&forwards, id), ForwardState::Active);
    assert_eq!(
        forwards.get(id).and_then(|f| f.pod.clone()).as_deref(),
        Some("postgres-1")
    );
    forwards.apply(id, event(ForwardEvent::Paused), now());
    assert_eq!(state_of(&forwards, id), ForwardState::Paused);
    forwards.apply(id, event(ForwardEvent::Resumed), now());
    assert_eq!(state_of(&forwards, id), ForwardState::Active);

    let ended = ForwardUpdate::Ended(ForwardError::TargetLost);
    // The start reported long ago, so the end is not a start report.
    assert_eq!(forwards.apply(id, ended, now()), None);
    let row = forwards.get(id).expect("the row exists");
    assert_eq!(row.state, ForwardState::Failed(ForwardFailure::TargetLost));
    assert_eq!(row.local, None);
}

#[test]
fn ending_before_the_target_resolves_reports_a_failed_start() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    let ended = ForwardUpdate::Ended(ForwardError::NoReadyPod);
    assert_eq!(
        forwards.apply(id, ended, now()),
        Some(StartOutcome::Failed(
            "no ready pod backs the forward target".to_owned()
        ))
    );
}

#[test]
fn ending_maps_each_error_to_its_failure() {
    let failure = |error: ForwardError| {
        let mut forwards = PortForwards::new();
        let id = starting(&mut forwards);
        forwards.apply(id, ForwardUpdate::Ended(error), now());
        match state_of(&forwards, id) {
            ForwardState::Failed(failure) => failure,
            other => panic!("expected a failure, got {other:?}"),
        }
    };
    assert_eq!(
        failure(ForwardError::PortInUse(3000)),
        ForwardFailure::PortInUse(3000)
    );
    assert_eq!(
        failure(ForwardError::PortReserved(3000)),
        ForwardFailure::PortReserved(3000)
    );
    assert_eq!(
        failure(ForwardError::NotPermitted),
        ForwardFailure::NotPermitted
    );
    assert_eq!(
        failure(ForwardError::Unauthorized),
        ForwardFailure::Other("the server refused the port-forward connection (HTTP 401)".into())
    );
    let cluster_error = ForwardError::Cluster(ClusterError::Rendered {
        message: "boom".to_owned(),
    });
    assert!(matches!(failure(cluster_error), ForwardFailure::Other(_)));
}

#[test]
fn a_resolve_while_the_cluster_is_locked_stays_paused() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    forwards.apply(id, ForwardUpdate::Event(ForwardEvent::Paused), now());
    forwards.apply(id, resolved(), now());
    assert_eq!(state_of(&forwards, id), ForwardState::Paused);
    let reconnected = ForwardEvent::Reconnected {
        pod: "postgres-1".to_owned(),
    };
    forwards.apply(id, ForwardUpdate::Event(reconnected), now());
    assert_eq!(state_of(&forwards, id), ForwardState::Paused);
}

#[test]
fn status_text_matches_the_wireframe() {
    let mut forwards = PortForwards::new();
    let mut text = |state: ForwardState| {
        let id = forwards.insert_fixture(fixture(
            "prod-a",
            spec(TargetKind::Pod, "p", 1, LocalPortSpec::Auto),
            state,
        ));
        forwards.get(id).expect("the row exists").status()
    };
    let mut label = |state, expected: &str, tone| {
        let status = text(state);
        assert_eq!(status.text.as_ref(), expected);
        assert_eq!(status.tone, tone);
    };
    label(ForwardState::Starting, "Starting…", StatusTone::Info);
    label(ForwardState::Active, "Active", StatusTone::Ok);
    label(ForwardState::Paused, "Paused · read-only", StatusTone::Warn);
    label(
        ForwardState::Reconnecting { attempt: 2 },
        "Reconnecting 2/5",
        StatusTone::Warn,
    );
    label(
        ForwardState::Failed(ForwardFailure::PortInUse(3000)),
        "Port 3000 in use",
        StatusTone::Bad,
    );
    label(
        ForwardState::Failed(ForwardFailure::PortReserved(3000)),
        "Port 3000 is reserved by the system",
        StatusTone::Bad,
    );
    label(
        ForwardState::Failed(ForwardFailure::NotPermitted),
        "Not permitted",
        StatusTone::Bad,
    );
    label(
        ForwardState::Failed(ForwardFailure::TargetLost),
        "Target lost",
        StatusTone::Bad,
    );
    label(
        ForwardState::Failed(ForwardFailure::Other("it broke".into())),
        "it broke",
        StatusTone::Bad,
    );
    label(ForwardState::Stopped, "Stopped · preset", StatusTone::Done);
}

#[test]
fn ports_text_shows_the_bound_port_a_dash_while_starting_and_the_requested_port_after() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    assert_eq!(
        forwards.get(id).map(Forward::ports_text).as_deref(),
        Some("5432 → localhost:—")
    );
    forwards.apply(
        id,
        ForwardUpdate::Listening {
            local: local(15433),
            has_ipv6: false,
        },
        now(),
    );
    assert_eq!(
        forwards.get(id).map(Forward::ports_text).as_deref(),
        Some("5432 → localhost:15433")
    );
    forwards.apply(id, ForwardUpdate::Ended(ForwardError::TargetLost), now());
    // The bind is gone; the row names the port it asked for (10000 + 5432).
    assert_eq!(
        forwards.get(id).map(Forward::ports_text).as_deref(),
        Some("5432 → localhost:15432")
    );
}

#[test]
fn uptime_reads_hours_minutes_and_days() {
    assert_eq!(uptime_text(-5), "<1m");
    assert_eq!(uptime_text(59), "<1m");
    assert_eq!(uptime_text(38 * 60), "38m");
    assert_eq!(uptime_text((2 * 60 + 14) * 60), "2h 14m");
    assert_eq!(uptime_text((3 * 24 + 4) * 3600 + 59), "3d 4h");
}

#[test]
fn uptime_shows_a_dash_unless_the_forward_carries_traffic() {
    let mut forwards = PortForwards::new();
    let mut active = fixture(
        "prod-a",
        spec(TargetKind::Pod, "p", 1, LocalPortSpec::Auto),
        ForwardState::Active,
    );
    active.started_at =
        Some(jiff::Timestamp::from_second(now().as_second() - 134 * 60).expect("valid"));
    let id = forwards.insert_fixture(active);
    assert_eq!(
        forwards.get(id).map(|f| f.uptime_text(now())).as_deref(),
        Some("2h 14m")
    );
    let id = forwards.insert_fixture(fixture(
        "prod-a",
        spec(TargetKind::Pod, "q", 1, LocalPortSpec::Auto),
        ForwardState::Reconnecting { attempt: 1 },
    ));
    assert_eq!(
        forwards.get(id).map(|f| f.uptime_text(now())).as_deref(),
        Some("—")
    );
}

#[test]
fn byte_counts_use_decimal_units() {
    assert_eq!(byte_count_text(0), "0 B");
    assert_eq!(byte_count_text(999), "999 B");
    assert_eq!(byte_count_text(9_400_000), "9.4 MB");
    assert_eq!(byte_count_text(182_000_000), "182 MB");
    assert_eq!(byte_count_text(3_500_000_000), "3.5 GB");
}

#[test]
fn events_keep_the_last_20() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    for attempt in 0..30_u8 {
        forwards.apply(
            id,
            ForwardUpdate::Event(ForwardEvent::ConnectionRefused {
                reason: format!("refused {attempt}"),
            }),
            now(),
        );
    }
    let events = &forwards.get(id).expect("the row exists").events;
    assert_eq!(events.len(), 20);
    assert_eq!(
        events.back().map(|line| line.text.as_ref()),
        Some("connection refused: refused 29")
    );
    assert_eq!(
        events.front().map(|line| line.text.as_ref()),
        Some("connection refused: refused 10")
    );
}

#[test]
fn stop_removes_non_preset_rows() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    forwards.select(Some(id));
    assert!(forwards.stop(id));
    assert!(forwards.get(id).is_none());
    assert!(forwards.selected().is_none());
    assert!(!forwards.stop(id));
}

#[test]
fn stop_keeps_preset_rows_stopped() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    forwards.apply(id, resolved(), now());
    let kept = forwards.save_preset(id, &[]);
    assert!(kept.is_some());
    assert!(forwards.stop(id));
    let row = forwards.get(id).expect("a preset row stays");
    assert_eq!(row.state, ForwardState::Stopped);
    assert_eq!(row.local, None);
    assert!(row.is_preset);
    assert_eq!(forwards.running_count(), 0);
}

#[test]
fn presets_load_as_stopped_rows() {
    let mut forwards = PortForwards::new();
    let presets = vec![ForwardPreset {
        cluster: cluster("stg-b"),
        spec: spec(
            TargetKind::Service,
            "kafka",
            9092,
            LocalPortSpec::Exact(9092),
        ),
    }];
    forwards.load_presets(&presets, |cluster| {
        (cluster.context.clone().into(), Environment::Staging)
    });
    let rows = forwards.forwards();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, ForwardState::Stopped);
    assert!(rows[0].is_preset);
    assert_eq!(rows[0].environment, Environment::Staging);
    assert_eq!(forwards.running_count(), 0);

    // The settings changed elsewhere: the gone preset's row goes, a running row stays.
    let running = starting(&mut forwards);
    forwards.load_presets(&[], |cluster| {
        (cluster.context.clone().into(), Environment::Staging)
    });
    assert_eq!(forwards.forwards().len(), 1);
    assert!(forwards.get(running).is_some());
}

#[test]
fn loading_presets_twice_keeps_the_rows_and_the_open_drawer() {
    let mut forwards = PortForwards::new();
    let presets = vec![ForwardPreset {
        cluster: cluster("stg-b"),
        spec: spec(
            TargetKind::Service,
            "kafka",
            9092,
            LocalPortSpec::Exact(9092),
        ),
    }];
    let describe = |cluster: &ClusterRef| (cluster.context.clone().into(), Environment::Staging);
    forwards.load_presets(&presets, describe);
    let id = forwards.forwards()[0].id;
    forwards.select(Some(id));
    forwards.load_presets(&presets, describe);
    assert_eq!(forwards.forwards().len(), 1);
    assert_eq!(forwards.selected().map(|f| f.id), Some(id));
}

#[test]
fn a_running_forward_that_matches_a_preset_is_not_listed_twice() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    let presets = vec![ForwardPreset {
        cluster: cluster("prod-a"),
        spec: spec(
            TargetKind::Pod,
            "postgres-0",
            5432,
            LocalPortSpec::Exact(15432),
        ),
    }];
    forwards.load_presets(&presets, |cluster| {
        (cluster.context.clone().into(), Environment::Production)
    });
    assert_eq!(forwards.forwards().len(), 1);
    assert!(forwards.get(id).is_some_and(|f| f.is_preset));
}

#[test]
fn save_as_preset_dedups() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    forwards.apply(
        id,
        ForwardUpdate::Listening {
            local: local(15440),
            has_ipv6: false,
        },
        now(),
    );
    let first = forwards.save_preset(id, &[]).expect("a new preset");
    // A preset keeps the port the forward really listens on, as a typed one.
    assert_eq!(first.spec.local_port, LocalPortSpec::Exact(15440));
    assert_eq!(first.cluster, cluster("prod-a"));
    assert!(forwards.get(id).is_some_and(|f| f.is_preset));
    // The same target with another local port is the same preset.
    let mut moved = first.clone();
    moved.spec.local_port = LocalPortSpec::Exact(1);
    assert_eq!(forwards.save_preset(id, &[moved]), None);
    // Another cluster is another preset.
    let mut other = first;
    other.cluster = cluster("prod-b");
    assert!(forwards.save_preset(id, &[other]).is_some());
}

#[test]
fn clearing_a_preset_keeps_a_running_row_and_drops_a_stopped_one() {
    let mut forwards = PortForwards::new();
    let running = starting(&mut forwards);
    forwards.apply(running, resolved(), now());
    forwards.save_preset(running, &[]);
    forwards.clear_preset(running);
    assert!(forwards.get(running).is_some_and(|f| !f.is_preset));

    let stopped = forwards.insert_fixture(ForwardFixture {
        is_preset: true,
        ..fixture(
            "stg-b",
            spec(
                TargetKind::Service,
                "kafka",
                9092,
                LocalPortSpec::Exact(9092),
            ),
            ForwardState::Stopped,
        )
    });
    forwards.clear_preset(stopped);
    assert!(forwards.get(stopped).is_none());
}

#[test]
fn preset_round_trips_and_has_no_secret_keys() {
    let preset = ForwardPreset {
        cluster: cluster("stg-b"),
        spec: spec(
            TargetKind::StatefulSet,
            "db",
            5432,
            LocalPortSpec::Exact(15432),
        ),
    };
    let value = serde_json::to_value(&preset).expect("serializes");
    assert_eq!(
        value,
        json!({
            "cluster": { "kubeconfig": "a.yaml", "context": "stg-b" },
            "spec": {
                "namespace": "payments",
                "target": { "kind": "statefulset", "name": "db" },
                "remote_port": 5432,
                "local_port": { "exact": 15432 },
            },
        })
    );
    let back: ForwardPreset = serde_json::from_value(value).expect("parses");
    assert_eq!(back, preset);
    let auto = serde_json::to_value(LocalPortSpec::Auto).expect("serializes");
    assert_eq!(auto, json!("auto"));
}

#[test]
fn running_for_matches_cluster_namespace_target_port() {
    let mut forwards = PortForwards::new();
    let id = forwards.insert_fixture(fixture(
        "prod-a",
        spec(TargetKind::Service, "api", 80, LocalPortSpec::Auto),
        ForwardState::Active,
    ));
    forwards.insert_fixture(fixture(
        "prod-a",
        spec(TargetKind::Service, "api", 443, LocalPortSpec::Auto),
        ForwardState::Failed(ForwardFailure::TargetLost),
    ));
    let target = TargetSpec {
        kind: TargetKind::Service,
        name: "api".to_owned(),
    };
    let found = |cluster_name: &str, namespace: &str, target: &TargetSpec, port: u16| {
        forwards
            .running_for(&cluster(cluster_name), namespace, target, port)
            .map(|forward| forward.id)
    };
    assert_eq!(found("prod-a", "payments", &target, 80), Some(id));
    // Another cluster, namespace, target kind, or port is another forward.
    assert_eq!(found("prod-b", "payments", &target, 80), None);
    assert_eq!(found("prod-a", "other", &target, 80), None);
    assert_eq!(
        found("prod-a", "payments", &TargetSpec::pod("api"), 80),
        None
    );
    assert_eq!(found("prod-a", "payments", &target, 8080), None);
    // A forward that is not running does not make a button live.
    assert_eq!(found("prod-a", "payments", &target, 443), None);
}

#[test]
fn own_port_conflict_is_refused() {
    let mut forwards = PortForwards::new();
    let mut bound = fixture(
        "prod-a",
        spec(TargetKind::Pod, "p", 80, LocalPortSpec::Auto),
        ForwardState::Active,
    );
    bound.local = Some(local(18080));
    let bound = forwards.insert_fixture(bound);
    let typed = forwards.insert_fixture(fixture(
        "prod-a",
        spec(TargetKind::Pod, "q", 80, LocalPortSpec::Exact(3000)),
        ForwardState::Starting,
    ));
    forwards.insert_fixture(fixture(
        "prod-a",
        spec(TargetKind::Pod, "r", 80, LocalPortSpec::Exact(4000)),
        ForwardState::Failed(ForwardFailure::PortInUse(4000)),
    ));
    assert_eq!(forwards.port_holder(18080, None).map(|f| f.id), Some(bound));
    // A forward still starting holds the port it was asked for.
    assert_eq!(forwards.port_holder(3000, None).map(|f| f.id), Some(typed));
    // A restart does not conflict with itself.
    assert!(forwards.port_holder(18080, Some(bound)).is_none());
    // A failed forward holds nothing.
    assert!(forwards.port_holder(4000, None).is_none());
    assert!(forwards.port_holder(5000, None).is_none());
}

#[test]
fn rows_sort_running_first_then_by_target() {
    let mut forwards = PortForwards::new();
    let stopped = forwards.insert_fixture(fixture(
        "stg-b",
        spec(TargetKind::Service, "a-first", 1, LocalPortSpec::Auto),
        ForwardState::Stopped,
    ));
    let b = forwards.insert_fixture(fixture(
        "prod-a",
        spec(TargetKind::Service, "b", 1, LocalPortSpec::Auto),
        ForwardState::Active,
    ));
    let a = forwards.insert_fixture(fixture(
        "prod-a",
        spec(TargetKind::Pod, "a", 1, LocalPortSpec::Auto),
        ForwardState::Reconnecting { attempt: 1 },
    ));
    let ids: Vec<ForwardId> = forwards.sorted("").iter().map(|f| f.id).collect();
    assert_eq!(ids, [a, b, stopped]);
}

#[test]
fn the_filter_matches_target_and_cluster_text() {
    let mut forwards = PortForwards::new();
    forwards.insert_fixture(fixture(
        "prod-eu-1",
        spec(TargetKind::Service, "payments-api", 80, LocalPortSpec::Auto),
        ForwardState::Active,
    ));
    forwards.insert_fixture(fixture(
        "stg-eu-1",
        spec(TargetKind::Service, "grafana", 3000, LocalPortSpec::Auto),
        ForwardState::Active,
    ));
    assert_eq!(forwards.sorted("GRAF").len(), 1);
    assert_eq!(forwards.sorted("stg").len(), 1);
    assert_eq!(forwards.sorted("svc/").len(), 2);
    assert_eq!(forwards.sorted("   ").len(), 2);
    assert!(forwards.sorted("nothing").is_empty());
}

#[test]
fn restarting_a_row_resets_it_and_keeps_its_identity() {
    let mut forwards = PortForwards::new();
    let id = starting(&mut forwards);
    forwards.apply(id, resolved(), now());
    forwards.apply(id, ForwardUpdate::Event(ForwardEvent::Paused), now());
    let again = forwards.begin(
        origin("prod-a"),
        spec(
            TargetKind::Pod,
            "postgres-0",
            5432,
            LocalPortSpec::Exact(25432),
        ),
        Some(id),
        now(),
    );
    assert_eq!(again, id);
    assert_eq!(forwards.forwards().len(), 1);
    let row = forwards.get(id).expect("the row exists");
    assert_eq!(row.state, ForwardState::Starting);
    assert_eq!(row.spec.local_port, LocalPortSpec::Exact(25432));
    assert_eq!(row.pod, None);
}

#[test]
fn parse_target_accepts_pod_svc_deploy_sts() {
    let ok = |text: &str| parse_target(text).expect("a valid target");
    assert_eq!(ok("pod/api-0"), TargetSpec::pod("api-0"));
    assert_eq!(ok("svc/payments-api").kind, TargetKind::Service);
    assert_eq!(ok("deploy/web").kind, TargetKind::Deployment);
    assert_eq!(ok("sts/db").kind, TargetKind::StatefulSet);
    assert_eq!(ok("  Service/web.internal ").name, "web.internal");
    assert_eq!(ok("statefulset/db").kind, TargetKind::StatefulSet);
}

#[test]
fn parse_target_refuses_other_shapes_and_unsafe_names() {
    assert_eq!(parse_target("api-0"), Err(TargetParseError::Shape));
    assert_eq!(parse_target("node/wk-1"), Err(TargetParseError::Shape));
    assert_eq!(parse_target("pod/"), Err(TargetParseError::Name));
    // The transport puts the name in a request path unescaped.
    for name in ["a/b", "a/../b", "A", "a b", "a?x=1", "a#b", "-a", "a_b"] {
        assert_eq!(
            parse_target(&format!("pod/{name}")),
            Err(TargetParseError::Name),
            "{name}"
        );
    }
}

#[test]
fn ports_parse_from_1_to_65535() {
    assert_eq!(parse_port("8080"), Some(8080));
    assert_eq!(parse_port(" 1 "), Some(1));
    assert_eq!(parse_port("65535"), Some(65535));
    for text in ["", "0", "65536", "-1", "80a", "8 0"] {
        assert_eq!(parse_port(text), None, "{text}");
    }
}

#[test]
fn local_port_for_typed_is_exact() {
    assert_eq!(local_port_for(Some(8081), 80), LocalPort::Exact(8081));
    assert_eq!(local_port_for(None, 5432), LocalPort::Auto(15432));
    assert_eq!(local_port_for(None, 60000), LocalPort::Auto(60000));
}

#[test]
fn a_spec_builds_the_transport_request() {
    let request = spec(TargetKind::Service, "api", 80, LocalPortSpec::Auto).request();
    assert_eq!(request.namespace, "payments");
    assert_eq!(request.remote_port, 80);
    assert_eq!(request.local_port, LocalPort::Auto(10080));
    assert_eq!(
        request.target,
        ForwardTarget::Service {
            name: "api".to_owned()
        }
    );
    let typed = spec(TargetKind::Pod, "p", 80, LocalPortSpec::Exact(3000)).request();
    assert_eq!(typed.local_port, LocalPort::Exact(3000));
}

#[test]
fn target_text_names_namespace_short_kind_and_name() {
    let spec = spec(TargetKind::Deployment, "web", 80, LocalPortSpec::Auto);
    assert_eq!(spec.target_text(), "payments/deploy/web");
    assert_eq!(spec.short_target_text(), "deploy/web");
}
