use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};
use crate::metrics_source::{MetricsScheme, MetricsSourceFields};
use crate::object_write::WritePolicy;

fn source() -> MetricsSource {
    MetricsSource::new(&MetricsSourceFields {
        namespace: "monitoring".to_owned(),
        service: "vmselect-x".to_owned(),
        port: "8481".to_owned(),
        scheme: MetricsScheme::Http,
        prefix: "/select/0/prometheus".to_owned(),
    })
    .expect("valid source")
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn vector(series: &[(Value, &str)]) -> String {
    let result: Vec<Value> = series
        .iter()
        .map(|(labels, value)| json!({"metric": labels, "value": [1_700_000_000.0, value]}))
        .collect();
    json!({"status": "success", "data": {"resultType": "vector", "result": result}}).to_string()
}

fn query_text(request: &RecordedRequest) -> String {
    form_urlencoded::parse(request.query.as_bytes())
        .find(|(key, _)| key == "query")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

fn istio_flow(workload: &str, namespace: &str, service: &str) -> Value {
    json!({
        "source_workload": workload,
        "source_workload_namespace": namespace,
        "destination_service_name": service,
    })
}

/// Answers the errors query (`response_code`) with `errors` and the totals query with `totals`.
fn istio_answers(totals: String, errors: (u16, String)) -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Blocked, move |request| {
        if query_text(request).contains("response_code") {
            errors.clone()
        } else {
            (200, totals.clone())
        }
    })
}

fn istio() -> TrafficMetricSource {
    TrafficMetricSource::detect(&names(&["istio_requests_total"]))[0]
}

fn pod_network() -> TrafficMetricSource {
    TrafficMetricSource::detect(&names(&["container_network_receive_bytes_total"]))[0]
}

#[test]
fn detect_orders_by_priority() {
    let found = TrafficMetricSource::detect(&names(&[
        "container_network_receive_bytes_total",
        "istio_requests_total",
    ]));
    let kinds: Vec<_> = found.iter().map(|found| found.kind()).collect();
    assert_eq!(
        kinds,
        [TrafficSourceKind::Istio, TrafficSourceKind::PodNetwork]
    );
}

#[test]
fn detect_on_uat_names_finds_only_pod_network() {
    let uat = names(&[
        "container_cpu_usage_seconds_total",
        "container_memory_working_set_bytes",
        "container_network_receive_bytes_total",
        "container_network_transmit_bytes_total",
        "kube_pod_info",
        "kube_pod_owner",
    ]);
    let found = TrafficMetricSource::detect(&uat);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind(), TrafficSourceKind::PodNetwork);
    assert_eq!(found[0].kind().label(), "pod network bytes");
    assert!(TrafficMetricSource::detect(&names(&["up"])).is_empty());
}

#[tokio::test]
async fn istio_rows_map_workload_to_service() {
    let totals = vector(&[
        (istio_flow("ledger", "payments", "payments-api"), "35"),
        (istio_flow("frontend", "web", "payments-api"), "80"),
        (istio_flow("unknown", "unknown", "payments-api"), "4"),
    ]);
    let errors = vector(&[(istio_flow("ledger", "payments", "payments-api"), "2.1")]);
    let (connection, api) = istio_answers(totals, (200, errors));
    let reading = connection
        .traffic_rates(&source(), istio(), "payments")
        .await
        .expect("answered");
    assert!(!reading.was_cut);
    let flow = |from: TrafficEnd, requests: f64, errors: f64| TrafficRate {
        from: Some(from),
        to: TrafficEnd::Service("payments-api".to_owned()),
        requests: Some(requests),
        errors: Some(errors),
        receive: None,
        transmit: None,
    };
    // Rows come in label order: `frontend`, `ledger`, `unknown`.
    assert_eq!(
        reading.rates,
        [
            flow(TrafficEnd::Outside("web/frontend".to_owned()), 80.0, 0.0),
            flow(TrafficEnd::Workload("ledger".to_owned()), 35.0, 2.1),
            flow(TrafficEnd::Outside("unknown".to_owned()), 4.0, 0.0),
        ]
    );
    let requests = api.requests();
    assert_eq!(requests.len(), 2, "two queries per source");
    assert!(requests.iter().all(|request| request.method == "GET"));
    assert!(
        requests
            .iter()
            .all(|request| request.path.ends_with("/api/v1/query"))
    );
}

#[tokio::test]
async fn pod_network_joins_receive_and_transmit() {
    let receive = vector(&[
        (json!({"pod": "api-1"}), "1200"),
        (json!({"pod": "api-2"}), "0"),
    ]);
    let transmit = vector(&[
        (json!({"pod": "api-1"}), "300"),
        (json!({"pod": "api-2"}), "5"),
        (json!({"pod": "tx-only"}), "7"),
        (json!({"pod": ""}), "9"),
    ]);
    let (connection, _api) = FakeApi::connection(WritePolicy::Blocked, move |request| {
        let body = if query_text(request).contains("transmit") {
            &transmit
        } else {
            &receive
        };
        (200, body.clone())
    });
    let reading = connection
        .traffic_rates(&source(), pod_network(), "payments")
        .await
        .expect("answered");
    let pod = |name: &str, receive: Option<f64>, transmit: Option<f64>| TrafficRate {
        from: None,
        to: TrafficEnd::Pod(name.to_owned()),
        requests: None,
        errors: None,
        receive,
        transmit,
    };
    assert_eq!(
        reading.rates,
        [
            pod("api-1", Some(1200.0), Some(300.0)),
            pod("api-2", Some(0.0), Some(5.0)),
            pod("tx-only", None, Some(7.0)),
        ]
    );
}

#[tokio::test]
async fn label_values_are_cleaned() {
    let dirty = format!("sv\n{}\u{1b}\u{202e}c", "x".repeat(200));
    let totals = vector(&[(istio_flow("ledger", "payments", &dirty), "1")]);
    let (connection, _api) = istio_answers(totals, (200, vector(&[])));
    let reading = connection
        .traffic_rates(&source(), istio(), "payments")
        .await
        .expect("answered");
    let TrafficEnd::Service(service) = &reading.rates[0].to else {
        panic!("expected a Service end");
    };
    assert_eq!(service.chars().count(), 80);
    assert!(service.starts_with("svxxx"));
    assert!(
        !service
            .chars()
            .any(|ch| ch.is_control() || ch == '\u{202e}')
    );
}

#[tokio::test]
async fn failed_errors_query_keeps_totals() {
    let totals = vector(&[(istio_flow("ledger", "payments", "payments-api"), "35")]);
    let (connection, _api) = istio_answers(totals, (500, "oops".to_owned()));
    let reading = connection
        .traffic_rates(&source(), istio(), "payments")
        .await
        .expect("the totals are enough");
    assert_eq!(reading.rates.len(), 1);
    assert_eq!(reading.rates[0].requests, Some(35.0));
    assert_eq!(reading.rates[0].errors, None);
}

#[tokio::test]
async fn failed_total_query_fails_the_reading() {
    let (connection, _api) = FakeApi::connection(WritePolicy::Blocked, |request| {
        if query_text(request).contains("response_code") {
            (200, vector(&[]))
        } else {
            (500, "oops".to_owned())
        }
    });
    let error = connection
        .traffic_rates(&source(), istio(), "payments")
        .await
        .expect_err("the first query failed");
    assert_eq!(error, MetricsError::Unexpected("HTTP 500".to_owned()));
}

#[tokio::test]
async fn traffic_reading_is_cut_at_2000_series() {
    let pods: Vec<(Value, &str)> = (0..2_001)
        .map(|index| (json!({"pod": format!("pod-{index:05}")}), "1"))
        .collect();
    let receive = vector(&pods);
    let (connection, _api) = FakeApi::connection(WritePolicy::Blocked, move |request| {
        if query_text(request).contains("transmit") {
            (200, vector(&[]))
        } else {
            (200, receive.clone())
        }
    });
    let reading = connection
        .traffic_rates(&source(), pod_network(), "payments")
        .await
        .expect("answered");
    assert!(reading.was_cut);
    assert_eq!(reading.rates.len(), TRAFFIC_SERIES_LIMIT);
    assert_eq!(reading.rates[0].to, TrafficEnd::Pod("pod-00000".to_owned()));
}

#[tokio::test]
async fn invalid_namespace_sends_nothing() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, String::new()));
    for kind in [istio(), pod_network()] {
        let error = connection
            .traffic_rates(&source(), kind, "pay\"ments")
            .await
            .expect_err("invalid");
        assert_eq!(error, MetricsError::InvalidName);
    }
    assert!(api.requests().is_empty());
}
