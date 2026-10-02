//! The log filter: `RUST_LOG` decides what is logged, except for one target it cannot raise.

use tracing_subscriber::EnvFilter;

/// kube-client logs the whole response body at `warn` when it cannot decode it, and a pod list
/// holds env literals. Pinned at `error` below whatever `RUST_LOG` says.
const SECRET_BEARING_TARGET: &str = "kube_client::client=error";

/// `RUST_LOG`, then the pinned directive.
pub(crate) fn log_filter() -> EnvFilter {
    pinned(EnvFilter::from_default_env())
}

/// A directive added last replaces an equal one from the environment, and a more specific one
/// beats a broader one, so `kube_client=debug` and `kube_client::client=debug` both stay capped.
fn pinned(filter: EnvFilter) -> EnvFilter {
    match SECRET_BEARING_TARGET.parse() {
        Ok(directive) => filter.add_directive(directive),
        // The text is a constant that parses (a test checks it), so this never happens.
        Err(_) => filter,
    }
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
    fn pinned_directive_parses() {
        assert!(
            SECRET_BEARING_TARGET
                .parse::<tracing_subscriber::filter::Directive>()
                .is_ok()
        );
    }

    #[test]
    fn rust_log_cannot_raise_the_kube_client_target() {
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
    fn other_targets_still_follow_rust_log() {
        let text = logged_with(pinned(EnvFilter::new("kube_client=warn")));
        assert!(text.contains("sibling-distinctive"), "{text}");
        let quiet = logged_with(pinned(EnvFilter::new("error")));
        assert!(!quiet.contains("sibling-distinctive"), "{quiet}");
    }
}
