//! The last line a crash-looping container logged before it died, for the WHY box of its pod: the
//! exit code says it failed, the line says why. One request (`previous=true`, one line) per
//! container and restart count, made only while the pod's drawer is open. Log lines are arbitrary
//! application output, so nothing here logs them.

use std::collections::BTreeMap;

use cluster::{ContainerState, LogLine, PodSummary, StatusReason};

use crate::cluster_runtime::WatchSubscription;

/// The most entries kept: a long session that opens many crash-looping pods forgets the oldest.
const MAX_ENTRIES: usize = 16;
/// The longest line the box quotes.
const LINE_CHARS: usize = 200;

/// What one request asks about.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LastLogKey {
    pub(crate) namespace: String,
    pub(crate) pod: String,
    pub(crate) container: String,
    /// A restart makes a new previous container, so it makes a new key.
    pub(crate) restart_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LastLog {
    Loading,
    /// `None` when the previous container logged nothing.
    Line(Option<String>),
    /// The kubelet has no previous log (rotated away, or the container never ran).
    Unavailable,
}

/// The key of the first container of `pod` that is in CrashLoopBackOff after a run to ask about.
pub(crate) fn last_log_key(pod: &PodSummary) -> Option<LastLogKey> {
    pod.containers
        .iter()
        .find(|container| {
            container.last_termination.is_some()
                && matches!(
                    &container.state,
                    ContainerState::Waiting {
                        reason: Some(StatusReason::CrashLoopBackOff),
                        ..
                    }
                )
        })
        .map(|container| LastLogKey {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            container: container.name.clone(),
            restart_count: container.restart_count,
        })
}

/// The newest non-empty line of a batch, without control characters and cut for the box.
pub(crate) fn last_line(lines: &[LogLine]) -> Option<String> {
    let text = lines
        .iter()
        .rev()
        .map(|line| line.text.trim())
        .find(|text| !text.is_empty())?;
    let printable: String = text
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let mut cut: String = printable.chars().take(LINE_CHARS).collect();
    if cut.len() < printable.len() {
        cut.push('…');
    }
    (!cut.trim().is_empty()).then_some(cut)
}

/// The answers so far and the request in flight.
#[derive(Default)]
pub(crate) struct LastLogs {
    entries: BTreeMap<LastLogKey, LastLog>,
    watch: Option<(LastLogKey, WatchSubscription)>,
}

impl LastLogs {
    pub(crate) fn get(&self, key: &LastLogKey) -> Option<&LastLog> {
        self.entries.get(key)
    }

    /// Whether a request for `key` is running or has answered.
    pub(crate) fn has(&self, key: &LastLogKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Drops the request in flight; an unanswered entry goes with it, so the next open asks again.
    pub(crate) fn stop(&mut self) {
        if let Some((key, _)) = self.watch.take()
            && self.entries.get(&key) == Some(&LastLog::Loading)
        {
            self.entries.remove(&key);
        }
    }

    pub(crate) fn start(&mut self, key: LastLogKey, subscription: WatchSubscription) {
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.clear();
        }
        self.entries.insert(key.clone(), LastLog::Loading);
        self.watch = Some((key, subscription));
    }

    pub(crate) fn set(&mut self, key: &LastLogKey, state: LastLog) {
        if let Some(entry) = self.entries.get_mut(key) {
            *entry = state;
        }
    }

    pub(crate) fn is_running(&self, key: &LastLogKey) -> bool {
        self.watch
            .as_ref()
            .is_some_and(|(running, _)| running == key)
    }
}

#[cfg(test)]
#[path = "last_log_tests.rs"]
mod last_log_tests;
