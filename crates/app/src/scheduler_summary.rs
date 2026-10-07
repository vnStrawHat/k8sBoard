//! The scheduler's `FailedScheduling` message in one line (spec 0034, drain tab): `0/3 nodes are
//! available: 1 node(s) had untolerated taint {workload: data}, 2 node(s) didn't match Pod's node
//! affinity/selector. preemption: …` reads `0/3 nodes: 1 taint, 2 selector`.

/// What a cause of the message is called in the summary, matched by a phrase of the scheduler's
/// own words; the first phrase found wins, so a specific one comes before a general one.
const CAUSES: [(&str, &str); 11] = [
    ("untolerated taint", "taint"),
    ("volume node affinity conflict", "volume"),
    ("persistent volumes to bind", "volume"),
    ("node affinity/selector", "selector"),
    ("node(s) were unschedulable", "cordoned"),
    ("anti-affinity", "anti-affinity"),
    ("free ports", "ports"),
    ("Insufficient cpu", "cpu"),
    ("Insufficient memory", "memory"),
    ("Insufficient", "resources"),
    ("topology spread", "spread"),
];

/// The one-line form of `message`, or `message` itself when it is not the usual node count and
/// causes (an unfamiliar cause keeps its own words, cut at the first comma).
pub(crate) fn scheduler_summary(message: &str) -> String {
    let Some((counts, causes)) = message.split_once(" nodes are available: ") else {
        return message.to_owned();
    };
    // The sentence ends where the scheduler adds its preemption verdict.
    let causes = causes.split_once(". ").map_or(causes, |(causes, _)| causes);
    let causes = causes.trim_end_matches('.');
    let parts: Vec<String> = split_causes(causes)
        .iter()
        .filter_map(|cause| {
            let (count, words) = cause.split_once(' ')?;
            count.parse::<u32>().ok()?;
            let name = CAUSES
                .iter()
                .find(|(phrase, _)| words.contains(phrase))
                .map_or(words, |(_, name)| name);
            Some(format!("{count} {name}"))
        })
        .collect();
    if parts.is_empty() {
        return message.to_owned();
    }
    format!("{counts} nodes: {}", parts.join(", "))
}

/// The causes of the sentence: split at a comma that starts a count, since a taint's braces hold
/// commas of their own (`{a: b}, {c: d}`).
fn split_causes(causes: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, _) in causes.match_indices(", ") {
        let starts_count = causes[index + 2..].starts_with(|c: char| c.is_ascii_digit());
        if starts_count {
            parts.push(&causes[start..index]);
            start = index + 2;
        }
    }
    parts.push(&causes[start..]);
    parts
}

#[cfg(test)]
#[path = "scheduler_summary_tests.rs"]
mod scheduler_summary_tests;
