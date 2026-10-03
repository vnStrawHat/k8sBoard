//! The log filter: `RUST_LOG` decides what is logged, except for the targets it cannot raise.

use tracing_subscriber::EnvFilter;

/// Targets that log secrets, each pinned below whatever `RUST_LOG` says.
///
/// - kube-client logs the whole response body at `warn` when it cannot decode it, and a pod list
///   holds env literals.
/// - tungstenite traces every WebSocket frame, payload included (`trace!("received frame ...")`),
///   which for a shell or a port-forward is keystrokes, passwords, and traffic. Its `log` records
///   reach `tracing` through the bridge `tracing_subscriber` installs. The submodules that print
///   frames are listed too, because a more specific directive from the environment would beat
///   the broad one.
const PINNED_TARGETS: [&str; 5] = [
    "kube_client::client=error",
    "tungstenite=info",
    "tungstenite::protocol=info",
    "tungstenite::protocol::frame=info",
    "tokio_tungstenite=info",
];

/// `RUST_LOG`, then the pinned directive.
pub(crate) fn log_filter() -> EnvFilter {
    pinned(EnvFilter::from_default_env())
}

/// A directive added last replaces an equal one from the environment, and a more specific one
/// beats a broader one, so `kube_client=debug` and `kube_client::client=debug` both stay capped.
fn pinned(filter: EnvFilter) -> EnvFilter {
    PINNED_TARGETS
        .into_iter()
        .fold(filter, |filter, target| match target.parse() {
            Ok(directive) => filter.add_directive(directive),
            // The texts are constants that parse (a test checks it), so this never happens.
            Err(_) => filter,
        })
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if let Ok(mut bytes) = self.0.lock() {
                bytes.extend_from_slice(buf);
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// What `filter` lets through when `kube_client::client` and a sibling both log a warning.
    fn logged_with(filter: EnvFilter) -> String {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: "kube_client::client", "body-distinctive");
            tracing::warn!(target: "kube_client::watcher", "sibling-distinctive");
        });
        let bytes = captured
            .0
            .lock()
            .map(|bytes| bytes.clone())
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn pinned_directives_parse() {
        for target in PINNED_TARGETS {
            assert!(
                target
                    .parse::<tracing_subscriber::filter::Directive>()
                    .is_ok(),
                "{target}"
            );
        }
    }

    /// What `filter` lets through of a trace event on each of the WebSocket targets.
    fn websocket_logged_with(filter: EnvFilter) -> String {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target: "tungstenite::protocol", "frame-distinctive");
            tracing::trace!(target: "tungstenite::protocol::frame", "wire-distinctive");
            tracing::trace!(target: "tokio_tungstenite::compat", "compat-distinctive");
            tracing::info!(target: "tungstenite::protocol", "info-distinctive");
        });
        let bytes = captured
            .0
            .lock()
            .map(|bytes| bytes.clone())
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn websocket_frame_traces_are_suppressed() {
        for env in [
            "trace",
            "tungstenite=trace",
            "tungstenite::protocol=trace",
            "tungstenite::protocol::frame=trace",
            "tokio_tungstenite=trace",
        ] {
            let text = websocket_logged_with(pinned(EnvFilter::new(env)));
            for secret in [
                "frame-distinctive",
                "wire-distinctive",
                "compat-distinctive",
            ] {
                assert!(!text.contains(secret), "{env}: {text}");
            }
        }
    }

    #[test]
    fn websocket_info_events_still_pass_the_filter() {
        let text = websocket_logged_with(pinned(EnvFilter::new("trace")));
        assert!(text.contains("info-distinctive"), "{text}");
    }

    #[test]
    fn kube_client_body_warning_is_suppressed() {
        for env in [
            "kube_client=debug",
            "kube_client::client=trace",
            "debug",
            "warn",
        ] {
            let text = logged_with(pinned(EnvFilter::new(env)));
            assert!(!text.contains("body-distinctive"), "{env}: {text}");
        }
    }

    #[test]
    fn other_warnings_still_pass_the_filter() {
        let text = logged_with(pinned(EnvFilter::new("kube_client=warn")));
        assert!(text.contains("sibling-distinctive"), "{text}");
        let quiet = logged_with(pinned(EnvFilter::new("error")));
        assert!(!quiet.contains("sibling-distinctive"), "{quiet}");
    }
}
