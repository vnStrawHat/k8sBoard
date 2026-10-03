//! A fake API server for tests: a `tower` service behind `kube::Client`, so no test reaches a
//! cluster. It records every request and answers from a closure.

use std::future::pending;
use std::sync::{Arc, Mutex};

use http::{Request, Response};
use kube::client::Body;

use crate::connection::ClusterConnection;
use crate::object_write::WritePolicy;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// One request the fake received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    /// The raw query string, empty when there is none.
    pub query: String,
    pub content_type: Option<String>,
    pub accept: Option<String>,
    pub body: String,
}

impl RecordedRequest {
    /// Whether the query has the pair `key=value`.
    pub fn has_query(&self, key: &str, value: &str) -> bool {
        self.query
            .split('&')
            .any(|pair| pair.split_once('=') == Some((key, value)))
    }

    /// Whether the query names `key` at all.
    pub fn has_query_key(&self, key: &str) -> bool {
        self.query
            .split('&')
            .any(|pair| pair.split_once('=').map_or(pair, |(name, _)| name) == key)
    }
}

/// How a transport that cannot answer fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The request is accepted and never answered.
    Hang,
    /// The service returns an error.
    Error,
}

/// The recorder of one fake connection.
pub struct FakeApi {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeApi {
    /// A connection whose every request is answered by `respond` with an HTTP status and a body.
    pub fn connection(
        policy: WritePolicy,
        respond: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static,
    ) -> (ClusterConnection, Self) {
        let respond = Arc::new(respond);
        Self::with_answer(policy, move |recorded| {
            let respond = Arc::clone(&respond);
            async move {
                let (code, body) = respond(&recorded);
                reply(code, body.into_bytes())
            }
        })
    }

    /// A connection whose every request is answered with `code` and the raw `body`, which need
    /// not be UTF-8.
    pub fn answering_bytes(
        policy: WritePolicy,
        code: u16,
        body: Vec<u8>,
    ) -> (ClusterConnection, Self) {
        Self::with_answer(policy, move |_| {
            let body = body.clone();
            async move { reply(code, body) }
        })
    }

    /// A connection whose requests are recorded and then fail as `failure` says.
    pub fn failing(policy: WritePolicy, failure: Failure) -> (ClusterConnection, Self) {
        Self::with_answer(policy, move |_| async move {
            match failure {
                Failure::Hang => pending().await,
                Failure::Error => Err("connection reset".into()),
            }
        })
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<RecordedRequest>> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A connection whose requests are answered by an async closure, for answers that must wait.
    pub(crate) fn with_answer<F, Fut>(policy: WritePolicy, answer: F) -> (ClusterConnection, Self)
    where
        F: Fn(RecordedRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response<Body>, BoxError>> + Send + 'static,
    {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        let answer = Arc::new(answer);
        let service = tower::service_fn(move |request: Request<Body>| {
            let log = Arc::clone(&log);
            let answer = Arc::clone(&answer);
            async move {
                let recorded = record(request).await?;
                log.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(recorded.clone());
                answer(recorded).await
            }
        });
        let client = kube::Client::new(service, "default");
        (
            ClusterConnection::from_client(client, "fake", policy),
            Self { requests },
        )
    }
}

async fn record(request: Request<Body>) -> Result<RecordedRequest, BoxError> {
    let (parts, body) = request.into_parts();
    let bytes = body.collect_bytes().await?;
    Ok(RecordedRequest {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().unwrap_or_default().to_owned(),
        content_type: parts
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        accept: parts
            .headers
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        body: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

pub(crate) fn reply(code: u16, body: Vec<u8>) -> Result<Response<Body>, BoxError> {
    Ok(Response::builder()
        .status(code)
        .header("content-type", "application/json")
        .body(Body::from(body))?)
}
