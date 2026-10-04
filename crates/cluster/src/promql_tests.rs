use super::*;

fn seconds(value: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(value).expect("in range")
}

fn range(step_seconds: u64) -> RangeSpec {
    RangeSpec::new(
        seconds(1_000_000),
        seconds(1_000_000 + 3_600),
        Duration::from_secs(step_seconds),
    )
    .expect("valid range")
}

fn pod(container: Option<&str>) -> UsageTarget {
    UsageTarget::Pod {
        namespace: "shop".to_owned(),
        pod: "api-7d9f8c-x2k4q".to_owned(),
        container: container.map(str::to_owned),
    }
}

fn node() -> UsageTarget {
    UsageTarget::Node {
        name: "worker-1".to_owned(),
    }
}

fn workload(kind: WorkloadKind, name: &str) -> UsageTarget {
    UsageTarget::Workload {
        namespace: "shop".to_owned(),
        kind,
        name: name.to_owned(),
    }
}

fn query(target: &UsageTarget, metric: UsageMetric, step_seconds: u64) -> String {
    usage_query(target, metric, &range(step_seconds)).expect("valid names")
}

#[test]
fn string_literal_escapes_quote_backslash_newline() {
    assert_eq!(string_literal("a\"b\\c\n"), r#""a\"b\\c\n""#);
    assert_eq!(string_literal("a\tb\rc\u{7}d"), r#""abcd""#);
    assert_eq!(string_literal("plain"), r#""plain""#);
}

#[test]
fn regex_literal_escapes_metacharacters() {
    assert_eq!(
        string_literal(&regex_escape("api.v1")),
        r#""api\\.v1""#,
        "a dot is escaped, then the backslash is doubled for the string"
    );
    assert_eq!(
        regex_escape(r"\.+*?()|[]{}^$"),
        r"\\\.\+\*\?\(\)\|\[\]\{\}\^\$"
    );
    assert_eq!(regex_escape("a-b"), "a-b");
}

#[test]
fn pod_cpu_query_text() {
    assert_eq!(
        query(&pod(None), UsageMetric::Cpu, 15),
        r#"sum(rate(container_cpu_usage_seconds_total{namespace="shop",pod="api-7d9f8c-x2k4q",container!="",container!="POD"}[120s]))"#
    );
}

#[test]
fn container_memory_query_text() {
    assert_eq!(
        query(&pod(Some("api")), UsageMetric::Memory, 15),
        r#"sum(container_memory_working_set_bytes{namespace="shop",pod="api-7d9f8c-x2k4q",container="api"})"#
    );
}

#[test]
fn node_query_text() {
    assert_eq!(
        query(&node(), UsageMetric::Cpu, 60),
        r#"sum(rate(container_cpu_usage_seconds_total{id=~"/|",pod="",node="worker-1"}[120s]))"#
    );
    assert_eq!(
        query(&node(), UsageMetric::Memory, 60),
        r#"sum(container_memory_working_set_bytes{id=~"/|",pod="",node="worker-1"})"#
    );
}

#[test]
fn disk_queries_use_the_filesystem_counters() {
    assert_eq!(
        query(&pod(Some("api")), UsageMetric::DiskRead, 15),
        r#"sum(rate(container_fs_reads_bytes_total{namespace="shop",pod="api-7d9f8c-x2k4q",container="api"}[120s]))"#
    );
    assert_eq!(
        query(&node(), UsageMetric::DiskWrite, 15),
        r#"sum(rate(container_fs_writes_bytes_total{id=~"/|",pod="",node="worker-1"}[120s]))"#
    );
}

#[test]
fn workload_patterns_per_kind() {
    let class = "[bcdfghjklmnpqrstvwxz2456789]";
    let cases = [
        (
            WorkloadKind::Deployment,
            format!(r#"pod=~"api\\.v1-{class}{{1,10}}-{class}{{5}}""#),
        ),
        (
            WorkloadKind::StatefulSet,
            r#"pod=~"api\\.v1-[0-9]+""#.to_owned(),
        ),
        (
            WorkloadKind::DaemonSet,
            format!(r#"pod=~"api\\.v1-{class}{{5}}""#),
        ),
        (
            WorkloadKind::ReplicaSet,
            format!(r#"pod=~"api\\.v1-{class}{{5}}""#),
        ),
        (
            WorkloadKind::Job,
            format!(r#"pod=~"api\\.v1-{class}{{5}}""#),
        ),
    ];
    for (kind, matcher) in cases {
        let text = query(&workload(kind, "api.v1"), UsageMetric::Memory, 15);
        assert_eq!(
            text,
            format!(
                r#"sum(container_memory_working_set_bytes{{namespace="shop",{matcher},container!="",container!="POD"}})"#
            ),
            "{kind:?}"
        );
    }
}

/// A hand-written matcher for `name-S{1,10}-S{5}`, so the test needs no regex dependency.
fn fits_deployment_pattern(pod_name: &str, name: &str) -> bool {
    let in_class = |text: &str| {
        text.bytes()
            .all(|byte| b"bcdfghjklmnpqrstvwxz2456789".contains(&byte))
    };
    let Some(rest) = pod_name
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('-'))
    else {
        return false;
    };
    let Some((replica_set, pod_suffix)) = rest.split_once('-') else {
        return false;
    };
    (1..=10).contains(&replica_set.len())
        && in_class(replica_set)
        && pod_suffix.len() == 5
        && in_class(pod_suffix)
}

#[test]
fn deployment_pattern_skips_cronjob_pods() {
    assert!(fits_deployment_pattern("api-7d9f8c-x2k4q", "api"));
    assert!(!fits_deployment_pattern("api-28312790-xb7zq", "api"));
}

#[test]
fn node_network_uses_the_virtual_interface_filter() {
    assert_eq!(
        query(&node(), UsageMetric::NetworkReceive, 15),
        r#"sum(rate(container_network_receive_bytes_total{id=~"/|",pod="",node="worker-1",interface!~"lo|(veth|cali|cni|flannel|cilium|lxc|docker|tunl|vxlan|kube-|weave|br-).*"}[120s]))"#
    );
    assert_eq!(
        query(&pod(None), UsageMetric::NetworkTransmit, 15),
        r#"sum(rate(container_network_transmit_bytes_total{namespace="shop",pod="api-7d9f8c-x2k4q",interface!="lo"}[120s]))"#
    );
}

#[test]
fn network_ignores_the_container_filter() {
    assert_eq!(
        query(&pod(Some("api")), UsageMetric::NetworkReceive, 15),
        query(&pod(None), UsageMetric::NetworkReceive, 15)
    );
}

#[test]
fn rate_window_is_at_least_two_minutes() {
    assert_eq!(range(15).rate_window(), "120s");
    assert_eq!(range(60).rate_window(), "120s");
    assert_eq!(range(7_200).rate_window(), "7200s");
    let text = usage_query(
        &pod(None),
        UsageMetric::Cpu,
        &RangeSpec::new(seconds(0), seconds(7_200 * 100), Duration::from_secs(7_200)).expect("ok"),
    )
    .expect("valid names");
    assert!(text.ends_with("[7200s]))"), "{text}");
}

#[test]
fn invalid_names_build_no_query() {
    let bad_pod = UsageTarget::Pod {
        namespace: "shop".to_owned(),
        pod: "a\"b".to_owned(),
        container: None,
    };
    let bad_namespace = UsageTarget::Pod {
        namespace: "Shop".to_owned(),
        pod: "api".to_owned(),
        container: None,
    };
    let bad_container = pod(Some("a.b"));
    let bad_node = UsageTarget::Node {
        name: "n\nx".to_owned(),
    };
    let bad_workload = workload(WorkloadKind::Deployment, "a\\b");
    for target in [
        bad_pod,
        bad_namespace,
        bad_container,
        bad_node,
        bad_workload,
    ] {
        assert_eq!(
            usage_query(&target, UsageMetric::Cpu, &range(15)),
            Err(InvalidName),
            "{target:?}"
        );
    }
}

#[test]
fn range_spec_limits() {
    let start = seconds(1_000_000);
    let step = Duration::from_secs(10);
    let end_for = |points: i64| seconds(1_000_000 + (points - 1) * 10);
    assert_eq!(
        RangeSpec::new(start, end_for(10), Duration::from_millis(999)),
        Err(RangeError::StepTooShort)
    );
    assert_eq!(
        RangeSpec::new(start, start, step),
        Err(RangeError::EmptyRange)
    );
    assert_eq!(
        RangeSpec::new(start, seconds(999_000), step),
        Err(RangeError::EmptyRange)
    );
    assert_eq!(
        RangeSpec::new(start, end_for(401), step),
        Err(RangeError::TooManyPoints)
    );
    assert!(RangeSpec::new(start, end_for(400), step).is_ok());
}

#[test]
fn step_table_stays_under_the_point_cap() {
    let now = seconds(1_790_000_123);
    for (span, step) in RANGE_STEPS {
        let spec = RangeSpec::ending_at(now, span).expect("in the table");
        assert_eq!(spec.step(), step);
        assert!(spec.points() <= 400, "{span:?}: {}", spec.points());
        assert_eq!(
            spec.end().as_second() % i64::try_from(step.as_secs()).expect("small"),
            0,
            "end is rounded down to the step"
        );
        assert_eq!(
            spec.end().as_second() - spec.start().as_second(),
            i64::try_from(span.as_secs()).expect("small")
        );
    }
    assert_eq!(
        RangeSpec::ending_at(now, Duration::from_secs(5)),
        Err(RangeError::UnknownSpan)
    );
}

#[test]
fn step_table_point_counts_match_the_spec() {
    let now = seconds(1_790_000_000);
    let counts: Vec<usize> = RANGE_STEPS
        .iter()
        .map(|(span, _)| {
            RangeSpec::ending_at(now, *span)
                .expect("in the table")
                .points()
        })
        .collect();
    assert_eq!(counts, [61, 241, 361, 289, 337, 361]);
}
