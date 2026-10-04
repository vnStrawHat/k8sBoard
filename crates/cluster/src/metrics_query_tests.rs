use std::collections::HashMap;

use serde_json::json;

use super::*;
use crate::fake_api::{Failure, FakeApi, RecordedRequest};
use crate::metrics_source::{MetricsScheme, MetricsSourceFields};
use crate::object_write::WritePolicy;

const START: i64 = 1_700_000_000;
const STEP: u64 = 60;

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

fn target() -> UsageTarget {
    UsageTarget::Pod {
        namespace: "shop".to_owned(),
        pod: "api-1".to_owned(),
        container: None,
    }
}

/// Ten minutes at one-minute steps: 11 points.
fn range() -> RangeSpec {
    let at = |second| jiff::Timestamp::from_second(second).expect("in range");
    RangeSpec::new(at(START), at(START + 600), Duration::from_secs(STEP)).expect("valid range")
}

fn matrix(series: &[Vec<(f64, &str)>]) -> String {
    let result: Vec<_> = series
        .iter()
        .map(|values| json!({"metric": {}, "values": values}))
        .collect();
    json!({"status": "success", "data": {"resultType": "matrix", "result": result}}).to_string()
}

fn status_body(code: u16, reason: &str, message: &str) -> String {
    json!({
        "kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure",
        "message": message, "reason": reason, "code": code,
    })
    .to_string()
}

async fn usage_answering(code: u16, body: String) -> Result<UsageSeries, MetricsError> {
    let (connection, _api) =
        FakeApi::connection(WritePolicy::Blocked, move |_| (code, body.clone()));
    connection
        .usage_range(&source(), &target(), UsageMetric::Cpu, &range())
        .await
}

fn decoded_query(request: &RecordedRequest) -> HashMap<String, String> {
    form_urlencoded::parse(request.query.as_bytes())
        .into_owned()
        .collect()
}

#[tokio::test]
async fn query_range_path_and_query() {
    let (connection, api) =
        FakeApi::connection(WritePolicy::Blocked, |_| (200, matrix(&[vec![(1.0, "1")]])));
    connection
        .usage_range(&source(), &target(), UsageMetric::Cpu, &range())
        .await
        .expect("answered");
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.path,
        "/api/v1/namespaces/monitoring/services/http:vmselect-x:8481/proxy/select/0/prometheus/api/v1/query_range"
    );
    assert_eq!(request.accept.as_deref(), Some("application/json"));
    assert!(request.body.is_empty());
    let pairs = decoded_query(request);
    assert_eq!(
        pairs["query"],
        r#"sum(rate(container_cpu_usage_seconds_total{namespace="shop",pod="api-1",container!="",container!="POD"}[120s]))"#
    );
    assert_eq!(pairs["start"], START.to_string());
    assert_eq!(pairs["end"], (START + 600).to_string());
    assert_eq!(pairs["step"], "60");
    assert_eq!(pairs["timeout"], "15s");
    assert!(
        !request.query.contains('{') && !request.query.contains('"'),
        "the PromQL is percent-encoded: {}",
        request.query
    );
}

#[test]
fn endpoints_are_fixed() {
    assert_eq!(MetricsEndpoint::Query.path(), "query");
    assert_eq!(MetricsEndpoint::QueryRange.path(), "query_range");
}

#[tokio::test]
async fn check_counts_cpu_series() {
    let vector = |value: serde_json::Value| {
        json!({"status": "success", "data": {"resultType": "vector", "result": value}}).to_string()
    };
    let counted = vector(json!([{"metric": {}, "value": [1_700_000_000.5, "1234"]}]));
    let (connection, api) =
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, counted.clone()));
    let check = connection
        .check_metrics_source(&source())
        .await
        .expect("answered");
    assert_eq!(check.cpu_series, 1234);
    let requests = api.requests();
    assert_eq!(
        requests[0].path,
        "/api/v1/namespaces/monitoring/services/http:vmselect-x:8481/proxy/select/0/prometheus/api/v1/query"
    );
    let pairs = decoded_query(&requests[0]);
    assert_eq!(pairs["query"], "count(container_cpu_usage_seconds_total)");
    assert!(pairs.contains_key("time"));
    assert_eq!(pairs["timeout"], "15s");

    let empty = vector(json!([]));
    let (connection, _api) =
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, empty.clone()));
    let check = connection
        .check_metrics_source(&source())
        .await
        .expect("answered");
    assert_eq!(check.cpu_series, 0);
}

#[tokio::test]
async fn matrix_decodes_with_gaps() {
    let t = |step: i64| (START + step * 60) as f64;
    let body = matrix(&[vec![
        (t(0), "1.5"),
        (t(1), "NaN"),
        (t(3), "2"),
        (t(4), "+Inf"),
        (t(5), "-Inf"),
        (t(6), "garbage"),
    ]]);
    let series = usage_answering(200, body).await.expect("decoded");
    assert!(!series.was_cut);
    assert_eq!(series.points.len(), 11);
    let values: Vec<Option<f64>> = series.points.iter().map(|(_, value)| *value).collect();
    assert_eq!(
        &values[..7],
        &[Some(1.5), None, None, Some(2.0), None, None, None]
    );
    assert!(values[7..].iter().all(Option::is_none));
    for (index, (at, _)) in series.points.iter().enumerate() {
        assert_eq!(at.as_second(), START + 60 * index as i64);
    }
}

#[tokio::test]
async fn samples_snap_to_the_nearest_step() {
    let body = matrix(&[vec![
        (START as f64 + 2.0 * 60.0 + 0.4, "7"),
        (START as f64 - 300.0, "9"),
        (START as f64 + 601.0 + 3600.0, "9"),
        (START as f64 + 10.0 * 60.0 + 20.0, "4"),
        (START as f64 - 20.0, "5"),
    ]]);
    let series = usage_answering(200, body).await.expect("decoded");
    let values: Vec<Option<f64>> = series.points.iter().map(|(_, value)| *value).collect();
    assert_eq!(
        values[0],
        Some(5.0),
        "20 s before the start rounds to step 0"
    );
    assert_eq!(values[2], Some(7.0));
    assert_eq!(
        values[10],
        Some(4.0),
        "20 s after the end rounds to the last step"
    );
    assert_eq!(
        values.iter().flatten().sum::<f64>(),
        16.0,
        "outside samples are dropped"
    );
}

#[tokio::test]
async fn series_are_summed_per_step() {
    let t = START as f64;
    let body = matrix(&[vec![(t, "1"), (t + 60.0, "2")], vec![(t, "10")]]);
    let series = usage_answering(200, body).await.expect("decoded");
    assert_eq!(series.points[0].1, Some(11.0));
    assert_eq!(series.points[1].1, Some(2.0));
    assert_eq!(series.points[2].1, None);
}

#[tokio::test]
async fn matrix_over_series_limit_is_cut() {
    let one = vec![(START as f64, "1")];
    let many = vec![one; 65];
    let series = usage_answering(200, matrix(&many)).await.expect("decoded");
    assert!(series.was_cut);
    assert_eq!(
        series.points[0].1,
        Some(64.0),
        "only the first 64 are summed"
    );

    let exact = vec![vec![(START as f64, "1")]; 64];
    let series = usage_answering(200, matrix(&exact)).await.expect("decoded");
    assert!(!series.was_cut);
}

#[tokio::test]
async fn body_over_limit_is_too_large() {
    for code in [200, 500] {
        let (connection, _api) =
            FakeApi::answering_bytes(WritePolicy::Blocked, code, vec![b' '; BODY_LIMIT + 1]);
        let error = connection
            .usage_range(&source(), &target(), UsageMetric::Cpu, &range())
            .await
            .expect_err("too large");
        assert_eq!(error, MetricsError::TooLarge, "HTTP {code}");
    }
}

#[tokio::test]
async fn body_at_the_limit_is_read() {
    let (connection, _api) =
        FakeApi::answering_bytes(WritePolicy::Blocked, 200, vec![b' '; BODY_LIMIT]);
    let error = connection
        .usage_range(&source(), &target(), UsageMetric::Cpu, &range())
        .await
        .expect_err("not JSON");
    assert_eq!(error, undecodable());
}

// The fake's head never arrives: kube's `Body` has no public constructor for a stalled stream, so
// this exercises the one deadline that covers `send` and the body read together.
#[tokio::test(start_paused = true)]
async fn slow_body_times_out() {
    let (connection, _api) = FakeApi::failing(WritePolicy::Blocked, Failure::Hang);
    let error = connection
        .usage_range(&source(), &target(), UsageMetric::Cpu, &range())
        .await
        .expect_err("times out");
    assert_eq!(error, MetricsError::TimedOut);
}

#[tokio::test]
async fn transport_failure_has_fixed_text() {
    let (connection, _api) = FakeApi::failing(WritePolicy::Blocked, Failure::Error);
    let error = connection
        .usage_range(&source(), &target(), UsageMetric::Cpu, &range())
        .await
        .expect_err("fails");
    assert!(matches!(error, MetricsError::Unexpected(_)), "{error}");
    assert!(!error.to_string().contains("connection reset"), "{error}");
}

#[tokio::test]
async fn forbidden_status_is_denied() {
    let body = status_body(403, "Forbidden", "services/proxy is forbidden");
    let error = usage_answering(403, body).await.expect_err("denied");
    assert_eq!(
        error,
        MetricsError::Denied("services/proxy is forbidden".to_owned())
    );
    assert_eq!(
        error.to_string(),
        "not permitted: services/proxy is forbidden"
    );
}

#[tokio::test]
async fn missing_service_and_no_endpoints_are_unreachable() {
    let missing = status_body(404, "NotFound", r#"services "x" not found"#);
    assert_eq!(
        usage_answering(404, missing).await.expect_err("missing"),
        MetricsError::Unreachable(r#"services "x" not found"#.to_owned())
    );
    let none = status_body(
        503,
        "ServiceUnavailable",
        "no endpoints available for service \"x\"",
    );
    assert_eq!(
        usage_answering(503, none).await.expect_err("no endpoints"),
        MetricsError::Unreachable("no endpoints available for service \"x\"".to_owned())
    );
}

#[tokio::test]
async fn backend_404_without_status_is_no_api_at_prefix() {
    let error = usage_answering(404, "404 page not found".to_owned())
        .await
        .expect_err("wrong prefix");
    assert_eq!(error, MetricsError::NoApiAtPrefix);
}

#[tokio::test]
async fn other_code_is_unexpected_with_the_code() {
    let secret = "<html>Bearer abc.def.ghi</html>";
    let error = usage_answering(502, secret.to_owned())
        .await
        .expect_err("bad gateway");
    assert_eq!(error, MetricsError::Unexpected("HTTP 502".to_owned()));
    assert!(!error.to_string().contains("Bearer"));
}

#[tokio::test]
async fn backend_error_json_is_rejected() {
    let body = json!({"status": "error", "errorType": "bad_data", "error": "parse error"});
    let error = usage_answering(400, body.to_string())
        .await
        .expect_err("rejected");
    assert_eq!(error, MetricsError::Rejected("parse error".to_owned()));
}

#[tokio::test]
async fn success_with_error_status_is_rejected() {
    let body = json!({"status": "error", "errorType": "timeout", "error": "query timed out"});
    let error = usage_answering(200, body.to_string())
        .await
        .expect_err("rejected");
    assert_eq!(error, MetricsError::Rejected("query timed out".to_owned()));
}

#[tokio::test]
async fn error_text_is_cut_to_one_line() {
    let message = "line one\nline two\t".repeat(100);
    assert!(message.chars().count() > 1_000);
    let body = json!({"status": "error", "error": message});
    let error = usage_answering(400, body.to_string())
        .await
        .expect_err("rejected");
    let MetricsError::Rejected(text) = error else {
        panic!("expected Rejected");
    };
    assert_eq!(text.chars().count(), 200);
    assert!(!text.chars().any(char::is_control));
}

#[tokio::test]
async fn wrong_result_type_is_undecodable() {
    let body = json!({"status": "success", "data": {"resultType": "vector", "result": []}});
    let error = usage_answering(200, body.to_string())
        .await
        .expect_err("not a matrix");
    assert_eq!(error, undecodable());
}

#[tokio::test]
async fn invalid_names_send_no_request() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, String::new()));
    let bad = UsageTarget::Pod {
        namespace: "shop".to_owned(),
        pod: "a\"b".to_owned(),
        container: None,
    };
    let error = connection
        .usage_range(&source(), &bad, UsageMetric::Cpu, &range())
        .await
        .expect_err("invalid");
    assert_eq!(error, MetricsError::InvalidName);
    assert!(api.requests().is_empty());
}

#[test]
fn error_texts_name_what_failed() {
    assert_eq!(
        MetricsError::NoApiAtPrefix.to_string(),
        "nothing answers at this path prefix; check the prefix and port"
    );
    assert_eq!(
        MetricsError::TooLarge.to_string(),
        "the metrics answer is larger than 4 MiB"
    );
    assert_eq!(
        MetricsError::TimedOut.to_string(),
        "the metrics backend did not answer within 20 s"
    );
    assert_eq!(
        MetricsError::Unreachable("gone".to_owned()).to_string(),
        "the metrics service cannot be reached: gone"
    );
    assert_eq!(
        MetricsError::Rejected("bad".to_owned()).to_string(),
        "the metrics backend refused the query: bad"
    );
    assert_eq!(
        MetricsError::Unexpected("HTTP 502".to_owned()).to_string(),
        "the metrics request failed: HTTP 502"
    );
}
