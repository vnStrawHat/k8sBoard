//! The app runs of this settings folder that created node shell pods. The leftover review ticks a
//! pod of one of them by default (it is the user's own, left by a quit or a crash), and says when
//! that run quit; a pod of an unknown run stays unticked because it may be another user's.
//!
//! One line per event in `<config>/node-shell-runs.log`: `<run id> <started|quit> <RFC 3339>`.
//! A run id is `[a-z0-9]`, so lines split on whitespace. No secret and no cluster name is stored.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::Path;

use cluster::NodeShellLeftover;

const RUNS_FILE: &str = "node-shell-runs.log";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunEvent {
    /// The run is about to create its first node shell pod.
    Started,
    /// The app is quitting with node shell work known to the run.
    Quit,
}

impl RunEvent {
    fn word(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Quit => "quit",
        }
    }
}

/// Appends one event. Blocking and tiny: the quit path calls it before the process ends.
// ponytail: the file only grows, one short line per run; trim it if a user ever runs thousands.
pub(crate) fn record_run_event(dir: &Path, run_id: &str, event: RunEvent) -> io::Result<()> {
    let at = jiff::Timestamp::now();
    let line = format!("{run_id} {} {at}\n", event.word());
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(RUNS_FILE))?;
    file.write_all(line.as_bytes())
}

/// The runs this settings folder has recorded, each with the time it quit (if it did).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PastRuns(HashMap<String, Option<jiff::Timestamp>>);

impl PastRuns {
    /// Reads the file; a missing or unreadable file is no history, and a line that does not parse
    /// is skipped (a crash can truncate the last one).
    pub(crate) fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(RUNS_FILE))
            .map_or_else(|_| Self::default(), |text| Self::parse(&text))
    }

    pub(crate) fn parse(text: &str) -> Self {
        let mut runs = HashMap::new();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let (Some(run), Some(event), Some(at), None) =
                (words.next(), words.next(), words.next(), words.next())
            else {
                continue;
            };
            let Ok(at) = at.parse::<jiff::Timestamp>() else {
                continue;
            };
            match event {
                "started" => {
                    runs.entry(run.to_owned()).or_insert(None);
                }
                "quit" => {
                    runs.insert(run.to_owned(), Some(at));
                }
                _ => {}
            }
        }
        Self(runs)
    }

    /// Whether `leftover` was created by a run of this settings folder.
    pub(crate) fn owns(&self, leftover: &NodeShellLeftover) -> bool {
        leftover
            .instance
            .as_deref()
            .is_some_and(|run| self.0.contains_key(run))
    }

    /// `left by your session, quit at 14:41` for a pod of a recorded run; the quit time is left out
    /// when the run never recorded one (a crash or a kill).
    pub(crate) fn note(
        &self,
        leftover: &NodeShellLeftover,
        zone: &jiff::tz::TimeZone,
    ) -> Option<String> {
        if !self.owns(leftover) {
            return None;
        }
        let quit = leftover
            .instance
            .as_deref()
            .and_then(|run| self.0.get(run).copied().flatten());
        Some(match quit {
            Some(at) => format!(
                "left by your session, quit at {}",
                at.to_zoned(zone.clone()).strftime("%H:%M")
            ),
            None => "left by your session".to_owned(),
        })
    }
}

#[cfg(test)]
#[path = "node_shell_run_history_tests.rs"]
mod node_shell_run_history_tests;
