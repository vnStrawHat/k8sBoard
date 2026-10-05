//! Bytes the Kubernetes client has sent and received (spec 0054).
//!
//! Counted at the one place the client is built: a tower layer around the whole HTTP stack,
//! after decompression. It sees request and response heads (estimated from the method, URI and
//! header lengths) and every data frame of a response body, so long watch streams count as they
//! arrive. Connections upgraded to a websocket (exec, port-forward) leave the stack after the
//! handshake, so their payload is not counted.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use http::{HeaderMap, Request, Response};
use http_body::Body as _;
use http_body_util::BodyExt as _;
use kube::client::{Body, ClientBuilder, DynBody};
use tower::{BoxError, Service, ServiceBuilder};

/// Running totals of one client; clones share the same counters.
#[derive(Clone, Debug, Default)]
pub struct TrafficCounter(Arc<Totals>);

#[derive(Debug, Default)]
struct Totals {
    received: AtomicU64,
    sent: AtomicU64,
}

impl TrafficCounter {
    /// Bytes read from the API server since the client was built.
    pub fn received(&self) -> u64 {
        self.0.received.load(Ordering::Relaxed)
    }

    /// Bytes written to the API server since the client was built.
    pub fn sent(&self) -> u64 {
        self.0.sent.load(Ordering::Relaxed)
    }

    fn add_received(&self, bytes: u64) {
        self.0.received.fetch_add(bytes, Ordering::Relaxed);
    }

    fn add_sent(&self, bytes: u64) {
        self.0.sent.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Builds the client of `builder` with a layer around its whole stack that counts every
    /// request and response.
    pub(crate) fn client<Inner>(&self, builder: ClientBuilder<Inner>) -> kube::Client
    where
        Inner: Service<Request<Body>, Response = Response<Box<DynBody>>> + Send + 'static,
        Inner::Future: Send + 'static,
        Inner::Error: Into<BoxError>,
    {
        let sent = self.clone();
        let received = self.clone();
        let layer = ServiceBuilder::new()
            .map_request(move |request: Request<Body>| {
                sent.add_sent(request_bytes(&request));
                request
            })
            .map_response(move |response: Response<Box<DynBody>>| {
                received.add_received(head_bytes(response.headers()));
                let counter = received.clone();
                let (parts, body) = response.into_parts();
                let body = body.map_frame(move |frame| {
                    if let Some(data) = frame.data_ref() {
                        counter.add_received(data.len() as u64);
                    }
                    frame
                });
                Response::from_parts(parts, body)
            });
        builder.with_layer(&layer).build()
    }
}

/// Request line, headers and the body when its size is known up front.
fn request_bytes(request: &Request<Body>) -> u64 {
    let line = request.method().as_str().len() + request.uri().to_string().len();
    let body = request.body().size_hint().exact().unwrap_or(0);
    line as u64 + head_bytes(request.headers()) + body
}

/// `name: value\r\n` for every header, plus the blank line that ends the head.
fn head_bytes(headers: &HeaderMap) -> u64 {
    let fields: usize = headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.len() + 4)
        .sum();
    (fields + 2) as u64
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;

    #[test]
    fn counter_totals_are_shared_between_clones() {
        let counter = TrafficCounter::default();
        let clone = counter.clone();
        clone.add_received(10);
        clone.add_sent(3);
        counter.add_received(5);
        assert_eq!((counter.received(), counter.sent()), (15, 3));
    }

    #[test]
    fn head_bytes_counts_every_header_and_the_closing_blank_line() {
        let mut headers = HeaderMap::new();
        assert_eq!(head_bytes(&headers), 2);
        headers.insert("accept", HeaderValue::from_static("json"));
        // "accept: json\r\n" is 14 bytes.
        assert_eq!(head_bytes(&headers), 14 + 2);
    }

    #[test]
    fn request_bytes_adds_the_line_the_head_and_a_sized_body() {
        let request = Request::builder()
            .method("GET")
            .uri("/api")
            .body(Body::empty())
            .expect("request builds");
        assert_eq!(request_bytes(&request), 3 + 4 + 2);
        let request = Request::builder()
            .method("PUT")
            .uri("/api")
            .body(Body::from(vec![0; 10]))
            .expect("request builds");
        assert_eq!(request_bytes(&request), 3 + 4 + 2 + 10);
    }

    #[tokio::test]
    async fn the_client_counts_what_it_sends_and_reads() {
        const VERSION: &str = r#"{"major":"1","minor":"29","gitVersion":"v1.29.5","gitCommit":"c","gitTreeState":"clean","buildDate":"d","goVersion":"g","compiler":"gc","platform":"linux/amd64"}"#;
        let service = tower::service_fn(|_: Request<Body>| async {
            let body: Box<DynBody> = Box::new(
                Body::from(VERSION.as_bytes().to_vec())
                    .map_err(|error| -> BoxError { error.into() }),
            );
            Ok::<_, BoxError>(Response::new(body))
        });
        let counter = TrafficCounter::default();
        let client = counter.client(ClientBuilder::new(service, "default"));
        let info = client.apiserver_version().await.expect("answer");
        assert_eq!(info.git_version, "v1.29.5");
        assert!(counter.sent() >= "GET/version".len() as u64);
        // The body, then the closing blank line of the head at least.
        assert!(counter.received() >= VERSION.len() as u64 + 2);
    }
}
