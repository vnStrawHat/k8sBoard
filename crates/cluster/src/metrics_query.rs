//! Read-only queries of a Prometheus-compatible source through the API server service proxy
//! (spec 0048). Only `query` and `query_range` are ever requested, as a `GET` whose path comes from
//! a validated `MetricsSource` and whose query text comes from `promql.rs`. A body or a header is
//! never traced or echoed.

use std::time::Duration;

use http_body_util::{BodyExt, LengthLimitError, Limited};
use kube::client::Body;
use serde::Deserialize;
use tokio::time::Instant;

use crate::connection::ClusterConnection;
use crate::metrics_source::MetricsSource;
use crate::promql::{QueryError, RangeSpec, UsageMetric, UsageTarget, usage_query};

/// Head and body together, for a success and an error answer alike.
const METRICS_DEADLINE: Duration = Duration::from_secs(20);
/// The `timeout=` query parameter, so the backend stops work it cannot finish.
const BACKEND_TIMEOUT: &str = "15s";
const BODY_LIMIT: usize = 4 * 1024 * 1024;
const SERIES_LIMIT: usize = 64;
const MESSAGE_LIMIT: usize = 200;
const CPU_SERIES_QUERY: &str = "count(container_cpu_usage_seconds_total)";

/// The backend endpoints that may be requested. A path is never built from anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MetricsEndpoint {
    Query,
    QueryRange,
}

impl MetricsEndpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::QueryRange => "query_range",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetricsError {
    #[error("not permitted: {0}")]
    Denied(String),
    #[error("the metrics service cannot be reached: {0}")]
    Unreachable(String),
    #[error("nothing answers at this path prefix; check the prefix and port")]
    NoApiAtPrefix,
    #[error("the metrics backend refused the query: {0}")]
    Rejected(String),
    #[error("the metrics answer is larger than 4 MiB")]
    TooLarge,
    #[error("the metrics backend did not answer within 20 s")]
    TimedOut,
    #[error("a name is not valid for a metrics query")]
    InvalidName,
    #[error("the metrics source does not provide this metric for this target")]
    Unsupported,
    #[error("the metrics request failed: {0}")]
    Unexpected(String),
}

/// The answer of the check query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceCheck {
    pub latency: Duration,
    /// 0: reachable, but no cAdvisor data.
    pub cpu_series: u64,
}

/// One chart series: a value per step from `start` to `end`, `None` where the source has a gap.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageSeries {
    pub points: Vec<(jiff::Timestamp, Option<f64>)>,
    /// The backend answered with more than 64 series; the first 64 were summed.
    pub was_cut: bool,
}

/// Status and body of one answer. Has no `Debug`: the body is never printed.
struct MetricsAnswer {
    status: http::StatusCode,
    body: Vec<u8>,
}

impl ClusterConnection {
    /// Counts the cAdvisor CPU series at now: the reachability check of a source.
    pub async fn check_metrics_source(
        &self,
        source: &MetricsSource,
    ) -> Result<SourceCheck, MetricsError> {
        let started = Instant::now();
        let now = jiff::Timestamp::now().as_second().to_string();
        let query = query_string(&[("query", CPU_SERIES_QUERY), ("time", &now)]);
        let data = self
            .metrics_data(source, MetricsEndpoint::Query, &query)
            .await?;
        let latency = started.elapsed();
        if data.result_type != "vector" {
            return Err(undecodable());
        }
        let cpu_series = data
            .result
            .first()
            .and_then(|series| series.value.as_ref())
            .and_then(Sample::finite)
            .map_or(0, |count| count.max(0.0).round() as u64);
        Ok(SourceCheck {
            latency,
            cpu_series,
        })
    }

    /// One metric of one target over `range`, summed per step.
    pub async fn usage_range(
        &self,
        source: &MetricsSource,
        target: &UsageTarget,
        metric: UsageMetric,
        range: &RangeSpec,
    ) -> Result<UsageSeries, MetricsError> {
        let promql = usage_query(target, metric, range).map_err(|error| match error {
            QueryError::InvalidName => MetricsError::InvalidName,
            QueryError::NodeDisk => MetricsError::Unsupported,
        })?;
        let query = query_string(&[
            ("query", &promql),
            ("start", &range.start().as_second().to_string()),
            ("end", &range.end().as_second().to_string()),
            ("step", &range.step().as_secs().to_string()),
        ]);
        let data = self
            .metrics_data(source, MetricsEndpoint::QueryRange, &query)
            .await?;
        if data.result_type != "matrix" {
            return Err(undecodable());
        }
        Ok(sum_matrix(&data.result, range))
    }

    /// One request, its answer decoded; traces endpoint, status, size, series, and time only.
    async fn metrics_data(
        &self,
        source: &MetricsSource,
        endpoint: MetricsEndpoint,
        query: &str,
    ) -> Result<Data, MetricsError> {
        let started = Instant::now();
        let answer = self.metrics_get(source, endpoint, query).await?;
        let decoded = decode(&answer);
        tracing::debug!(
            endpoint = endpoint.path(),
            status = answer.status.as_u16(),
            bytes = answer.body.len(),
            series = decoded.as_ref().map_or(0, |data| data.result.len()),
            ms = started.elapsed().as_millis() as u64,
            "read metrics"
        );
        decoded
    }

    /// `GET` of one endpoint of the allow-list. `ClusterConnection::run` is not used: its 30 s
    /// timeout and `classify_error` would drop the `Status` this module maps itself, and kube's
    /// own readers take a success or an error body whole.
    // Read-only GET of the 0048 metrics endpoint allow-list through the service proxy: the metrics_query.rs row of the 0030 table.
    #[allow(clippy::disallowed_methods)]
    async fn metrics_get(
        &self,
        source: &MetricsSource,
        endpoint: MetricsEndpoint,
        query: &str,
    ) -> Result<MetricsAnswer, MetricsError> {
        let uri = format!("{}/api/v1/{}?{query}", source.proxy_path(), endpoint.path());
        let request = http::Request::get(uri)
            .header(http::header::ACCEPT, "application/json")
            .body(Body::empty())
            .map_err(|_| MetricsError::Unexpected("the request could not be built".to_owned()))?;
        let deadline = Instant::now() + METRICS_DEADLINE;
        let read = async {
            let response = self
                .client()
                .send(request)
                .await
                .map_err(|error| MetricsError::Unexpected(transport_text(&error).to_owned()))?;
            let (parts, body) = response.into_parts();
            let collected = Limited::new(body, BODY_LIMIT)
                .collect()
                .await
                .map_err(|error| {
                    if error.downcast_ref::<LengthLimitError>().is_some() {
                        MetricsError::TooLarge
                    } else {
                        MetricsError::Unexpected("the metrics answer could not be read".to_owned())
                    }
                })?;
            Ok(MetricsAnswer {
                status: parts.status,
                body: collected.to_bytes().to_vec(),
            })
        };
        match tokio::time::timeout_at(deadline, read).await {
            Ok(answer) => answer,
            Err(_elapsed) => Err(MetricsError::TimedOut),
        }
    }
}

/// Fixed text per failure: the error text of an auth or TLS layer can hold secrets.
fn transport_text(error: &kube::Error) -> &'static str {
    match error {
        kube::Error::Auth(_) => "credentials are unavailable",
        kube::Error::HyperError(_)
        | kube::Error::Service(_)
        | kube::Error::RustlsTls(_)
        | kube::Error::HttpError(_) => "the API server could not be reached",
        _ => "the request could not be sent",
    }
}

fn undecodable() -> MetricsError {
    MetricsError::Unexpected("the answer could not be decoded".to_owned())
}

/// The query string: percent-encoded pairs plus the backend `timeout`.
fn query_string(pairs: &[(&str, &str)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.append_pair("timeout", BACKEND_TIMEOUT);
    serializer.finish()
}

/// What the API server and the backend both send; unknown fields are ignored.
#[derive(Deserialize)]
struct Envelope {
    /// `Status` for an API server error object.
    kind: Option<String>,
    /// The backend's `success` or `error`, or the API server's `Failure`.
    status: Option<String>,
    /// The API server's text.
    message: Option<String>,
    /// The backend's text.
    error: Option<String>,
    data: Option<Data>,
}

#[derive(Deserialize)]
struct Data {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Vec<Series>,
}

#[derive(Deserialize)]
struct Series {
    /// A vector sample.
    value: Option<Sample>,
    /// A matrix: samples in time order.
    values: Option<Vec<Sample>>,
}

/// `[unix seconds, "value"]`.
#[derive(Deserialize)]
struct Sample(f64, String);

impl Sample {
    /// `None` for `NaN`, an infinity, and text that is not a number.
    fn finite(&self) -> Option<f64> {
        self.1.parse::<f64>().ok().filter(|value| value.is_finite())
    }
}

/// Maps status and body to the data or to the error; the body is never echoed, only the
/// message fields of a `Status` and of a backend error, cut to one short line.
fn decode(answer: &MetricsAnswer) -> Result<Data, MetricsError> {
    let code = answer.status.as_u16();
    let envelope: Option<Envelope> = serde_json::from_slice(&answer.body).ok();
    if let Some(envelope) = &envelope {
        if envelope.kind.as_deref() == Some("Status") {
            let message = one_line(envelope.message.as_deref());
            return Err(match code {
                // The text of a 401 is not quoted: it can echo what the credential looked like.
                401 => MetricsError::Denied("credentials were rejected".to_owned()),
                403 => MetricsError::Denied(or_text(message, "the API server refused the request")),
                404 | 503 => {
                    MetricsError::Unreachable(or_text(message, "the service did not answer"))
                }
                _ => MetricsError::Unexpected(format!("HTTP {code}")),
            });
        }
        if envelope.status.as_deref() == Some("error") {
            return Err(MetricsError::Rejected(or_text(
                one_line(envelope.error.as_deref()),
                "no reason given",
            )));
        }
    }
    if code == 404 {
        return Err(MetricsError::NoApiAtPrefix);
    }
    if !answer.status.is_success() {
        return Err(MetricsError::Unexpected(format!("HTTP {code}")));
    }
    match envelope {
        Some(Envelope {
            status: Some(status),
            data: Some(data),
            ..
        }) if status == "success" => Ok(data),
        _ => Err(undecodable()),
    }
}

/// `fallback` when `text` is empty, so an error never ends in a bare colon.
fn or_text(text: String, fallback: &str) -> String {
    if text.is_empty() {
        fallback.to_owned()
    } else {
        text
    }
}

/// At most 200 characters; control characters and the invisible direction and format characters
/// are dropped, so a backend text cannot reorder or hide what the line shows.
fn one_line(text: Option<&str>) -> String {
    text.unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_control() && !is_invisible_format(*ch))
        .take(MESSAGE_LIMIT)
        .collect()
}

fn is_invisible_format(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// Adds the first 64 series per step. A sample goes to the nearest step (never a float equality)
/// and is dropped outside the range; a step without a sample stays `None`.
fn sum_matrix(series: &[Series], range: &RangeSpec) -> UsageSeries {
    let count = range.points();
    let mut sums: Vec<Option<f64>> = vec![None; count];
    let start = range.start().as_second() as f64;
    let step = range.step().as_secs() as f64;
    for samples in series
        .iter()
        .take(SERIES_LIMIT)
        .filter_map(|s| s.values.as_ref())
    {
        for sample in samples {
            let Some(value) = sample.finite() else {
                continue;
            };
            let index = ((sample.0 - start) / step).round();
            if index < 0.0 || index >= count as f64 {
                continue;
            }
            let slot = &mut sums[index as usize];
            *slot = Some(slot.unwrap_or(0.0) + value);
        }
    }
    let step_seconds = range.step().as_secs() as i64;
    let points = sums
        .into_iter()
        .enumerate()
        .map(|(index, sum)| {
            let offset = step_seconds.saturating_mul(index as i64);
            let at = jiff::Timestamp::from_second(range.start().as_second().saturating_add(offset))
                .unwrap_or(range.end());
            (at, sum)
        })
        .collect();
    UsageSeries {
        points,
        was_cut: series.len() > SERIES_LIMIT,
    }
}

#[cfg(test)]
#[path = "metrics_query_tests.rs"]
mod metrics_query_tests;
