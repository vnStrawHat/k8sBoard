//! Why a pod is failing or not ready, and what its probes report. Pure: the cluster crate
//! exposes the data, this module explains it. Event messages are arbitrary text, so nothing
//! here logs them.

use cluster::{
    ContainerKind, ContainerState, ContainerSummary, EventSummary, EventType, PodStatus,
    PodSummary, ProbeSummary, StatusReason, Termination,
};
use jiff::{SignedDuration, Timestamp};

use crate::age::format_age;
use crate::container_detail::probe_summary_text;
use crate::event_rows::message_line;
use crate::status_tone::{StatusTone, is_bad_reason};

/// The text of the WHY box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PodDiagnosis {
    /// `Bad` or `Warn`.
    pub(crate) tone: StatusTone,
    /// The container the text is about; `None` for a pod-level cause.
    pub(crate) container: Option<String>,
    pub(crate) text: String,
    /// The rule that fired. The box ignores it; the issue rules read it.
    pub(crate) cause: DiagnosisCause,
}

/// Which diagnosis rule fired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiagnosisCause {
    /// P1; `since` is the `PodScheduled` condition's transition time.
    Unschedulable { since: Option<Timestamp> },
    /// P2.
    SchedulingGated,
    /// P3.
    PodFailed,
    /// C1.
    ImagePull(StatusReason),
    /// C2 to C4.
    CrashLoop,
    /// C5.
    Waiting(StatusReason),
    /// C6.
    Exited { reason: Option<StatusReason> },
    /// C7.
    StartupPending,
    /// C8.
    NotReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProbeKind {
    Liveness,
    Readiness,
    Startup,
}

impl ProbeKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Liveness => "Liveness",
            Self::Readiness => "Readiness",
            Self::Startup => "Startup",
        }
    }
}

/// What a probe currently reports. The API has no per-probe status, so this is read from the
/// container's ready and started flags plus the `Unhealthy` events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProbeResult {
    NotSet,
    /// The pod is terminating or the container is not running.
    Inactive,
    /// The kubelet runs liveness and readiness probes only after the startup probe passes.
    WaitingForStartup,
    /// A startup probe that has not passed yet.
    Pending,
    Passed,
    Passing,
    /// `failures` is the count of the newest failure event, or 0 when no event matches.
    Failing {
        failures: u32,
    },
    /// The events are not loaded yet.
    NoData,
}

struct Problem {
    tone: StatusTone,
    text: String,
    cause: DiagnosisCause,
}

pub(crate) fn pod_diagnosis(
    pod: &PodSummary,
    events: Option<&[EventSummary]>,
    now: Timestamp,
) -> Option<PodDiagnosis> {
    if is_diagnosis_skipped(pod) {
        return None;
    }
    if let Some(problem) = pod_problem(pod) {
        return Some(PodDiagnosis {
            tone: problem.tone,
            container: None,
            text: problem.text,
            cause: problem.cause,
        });
    }

    let problems: Vec<(&ContainerSummary, Problem)> = pod
        .containers
        .iter()
        .filter_map(|container| {
            container_problem(container, events, now).map(|problem| (container, problem))
        })
        .collect();
    let (container, problem) = problems
        .iter()
        .find(|(_, problem)| problem.tone == StatusTone::Bad)
        .or_else(|| problems.first())?;
    let suffix = match (problems.len(), pod.containers.len()) {
        (1, total) if total >= 2 => Some("Other containers are healthy.".to_owned()),
        (1, _) => None,
        (2, _) => Some("1 other container also has a problem.".to_owned()),
        (count, _) => Some(format!(
            "{} other containers also have problems.",
            count - 1
        )),
    };
    let text = match suffix {
        Some(suffix) => sentence_then(&problem.text, &suffix),
        None => problem.text.clone(),
    };
    Some(PodDiagnosis {
        tone: problem.tone,
        container: Some(container.name.clone()),
        text,
        cause: problem.cause.clone(),
    })
}

/// One name of a pod's `imagePullSecrets`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PullSecret {
    pub(crate) name: String,
    /// Not in the namespace's Secrets list. Always `false` while that list is not loaded.
    pub(crate) is_missing: bool,
}

/// The pod's pull secrets. `existing` holds the names of the Secrets in the pod's namespace when
/// that list is loaded; without it nothing is called missing.
pub(crate) fn pull_secrets(pod: &PodSummary, existing: Option<&[&str]>) -> Vec<PullSecret> {
    pod.image_pull_secrets
        .iter()
        .map(|name| PullSecret {
            is_missing: existing.is_some_and(|names| !names.contains(&name.as_str())),
            name: name.clone(),
        })
        .collect()
}

impl PodDiagnosis {
    /// The text for the WHY box: a scheduler message lists one reason per line. The issue rules
    /// read `text`, which stays one sentence.
    pub(crate) fn display_text(&self) -> String {
        match self.cause {
            DiagnosisCause::Unschedulable { .. } => scheduler_bullets(&self.text),
            _ => self.text.clone(),
        }
    }

    /// C1 for a pull that was refused or retried, not for a bad image name or a never-pull policy.
    pub(crate) fn is_pull_failure(&self) -> bool {
        matches!(
            self.cause,
            DiagnosisCause::ImagePull(StatusReason::ImagePullBackOff | StatusReason::ErrImagePull)
        )
    }

    /// Adds the secrets the kubelet tried to a failed pull, so a missing one is seen at once.
    pub(crate) fn with_pull_secrets(mut self, secrets: &[PullSecret]) -> Self {
        if self.is_pull_failure() && !secrets.is_empty() {
            let names: Vec<String> = secrets
                .iter()
                .map(|secret| {
                    if secret.is_missing {
                        format!("{} (missing)", secret.name)
                    } else {
                        secret.name.clone()
                    }
                })
                .collect();
            self.text = format!("{}\nPull secrets: {}", self.text, names.join(", "));
        }
        self
    }
}
/// `0/3 nodes are available: 1 Insufficient cpu, 2 node(s) had taint {a: b}. preemption: …` with a
/// line for each sentence and a bullet for each counted reason. Text that does not look like a
/// scheduler message stays as it is.
fn scheduler_bullets(text: &str) -> String {
    text.split(". ")
        .map(|sentence| {
            let Some((head, reasons)) = sentence.split_once(" are available: ") else {
                return sentence.to_owned();
            };
            let mut lines = vec![format!("{head} are available:")];
            let mut start = 0;
            // A reason starts with its node count, so a comma inside a taint is not a split.
            for (at, _) in reasons.match_indices(", ") {
                if reasons[at + 2..].starts_with(|c: char| c.is_ascii_digit()) {
                    lines.push(format!("• {}", &reasons[start..at]));
                    start = at + 2;
                }
            }
            lines.push(format!("• {}", &reasons[start..]));
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// P0: a pod that is going away or is done has nothing to explain.
pub(crate) fn is_diagnosis_skipped(pod: &PodSummary) -> bool {
    matches!(
        pod.status,
        PodStatus::Terminating
            | PodStatus::Reason(StatusReason::Succeeded | StatusReason::Completed)
    )
}

/// `text` followed by `next`, with a full stop added when `text` does not end a sentence.
fn sentence_then(text: &str, next: &str) -> String {
    if text.ends_with(['.', '!', '?']) {
        format!("{text} {next}")
    } else {
        format!("{text}. {next}")
    }
}

/// P1 to P3: a cause that belongs to the pod, which beats any container cause.
fn pod_problem(pod: &PodSummary) -> Option<Problem> {
    let unschedulable = pod
        .conditions
        .iter()
        .find(|condition| condition.name == "PodScheduled")
        .filter(|condition| {
            !condition.is_true && condition.reason.as_deref() == Some("Unschedulable")
        });
    if let Some(condition) = unschedulable {
        return Some(Problem {
            tone: StatusTone::Bad,
            text: match condition.message.as_deref() {
                Some(message) => format!("Cannot be scheduled: {message}"),
                None => "Cannot be scheduled.".to_owned(),
            },
            cause: DiagnosisCause::Unschedulable {
                since: condition.changed_at,
            },
        });
    }
    if pod.status == PodStatus::Reason(StatusReason::SchedulingGated) {
        return Some(Problem {
            tone: StatusTone::Warn,
            text: "Waiting for its scheduling gates to be removed.".to_owned(),
            cause: DiagnosisCause::SchedulingGated,
        });
    }
    pod.status_message.as_ref().map(|message| Problem {
        tone: StatusTone::Bad,
        text: format!("{}: {message}", pod.status),
        cause: DiagnosisCause::PodFailed,
    })
}

fn container_problem(
    container: &ContainerSummary,
    events: Option<&[EventSummary]>,
    now: Timestamp,
) -> Option<Problem> {
    match &container.state {
        ContainerState::Waiting { reason, message } => {
            waiting_problem(container, reason.as_ref()?, message.as_deref(), events)
        }
        ContainerState::Terminated(termination) if termination.exit_code != 0 => {
            Some(terminated_problem(container, termination))
        }
        ContainerState::Running { .. } => running_problem(container, events, now),
        ContainerState::Terminated(_) | ContainerState::NotReported => None,
    }
}

/// C1 to C5.
fn waiting_problem(
    container: &ContainerSummary,
    reason: &StatusReason,
    message: Option<&str>,
    events: Option<&[EventSummary]>,
) -> Option<Problem> {
    let (text, cause) = match reason {
        StatusReason::ImagePullBackOff
        | StatusReason::ErrImagePull
        | StatusReason::InvalidImageName
        | StatusReason::ErrImageNeverPull => {
            let mut text = format!(
                "Cannot pull image {}: {}",
                container.image,
                message.map_or_else(|| reason.to_string(), str::to_owned)
            );
            // The waiting message only says the kubelet backs off; the events say why.
            let is_retry = matches!(
                reason,
                StatusReason::ImagePullBackOff | StatusReason::ErrImagePull
            );
            for line in events
                .filter(|_| is_retry)
                .map_or_else(Vec::new, pull_event_lines)
            {
                text.push('\n');
                text.push_str(&line);
            }
            (text, DiagnosisCause::ImagePull(reason.clone()))
        }
        StatusReason::CrashLoopBackOff => (
            crash_loop_text(container, message),
            DiagnosisCause::CrashLoop,
        ),
        reason if is_bad_reason(reason) => (
            match message {
                Some(message) => format!("{reason}: {message}"),
                None => format!("{reason}."),
            },
            DiagnosisCause::Waiting(reason.clone()),
        ),
        _ => return None,
    };
    Some(Problem {
        tone: StatusTone::Bad,
        text,
        cause,
    })
}

/// The longest pull failure message the WHY box quotes.
const PULL_CAUSE_CHARS: usize = 200;

/// `Cause: …` from the newest Failed event that says more than `Error: ImagePullBackOff`, and
/// `Pull secret X not found in ns` from the `FailedToRetrieveImagePullSecret` event.
fn pull_event_lines(events: &[EventSummary]) -> Vec<String> {
    let newest = |accepts: &dyn Fn(&EventSummary) -> bool| {
        events
            .iter()
            .filter(|event| event.event_type == EventType::Warning && accepts(event))
            .max_by_key(|event| event.last_seen)
    };
    let mut lines = Vec::new();
    let failed = newest(&|event| {
        event.reason == "Failed" && event.message.starts_with("Failed to pull image")
    });
    if let Some(event) = failed {
        lines.push(format!(
            "Cause: {}",
            pull_cause(&message_line(&event.message))
        ));
    }
    let secrets = newest(&|event| event.reason == "FailedToRetrieveImagePullSecret");
    if let Some(event) = secrets
        && let Some(names) = pull_secret_names(&event.message)
    {
        let noun = if names.contains(", ") {
            "secrets"
        } else {
            "secret"
        };
        lines.push(format!(
            "Pull {noun} {names} not found in {}",
            event.namespace
        ));
    }
    lines
}

/// The tail of a kubelet pull failure that names the network or registry error, else the message
/// cut to `PULL_CAUSE_CHARS`. The head repeats the image name, which the box already shows.
fn pull_cause(message: &str) -> String {
    if let Some((_, tail)) = message.rsplit_once("dial tcp: ") {
        return tail.to_owned();
    }
    let mut cut: String = message.chars().take(PULL_CAUSE_CHARS).collect();
    if cut.len() < message.len() {
        cut.push('…');
    }
    cut
}

/// `a, b` from `Unable to retrieve some image pull secrets (a, b); attempting …`.
fn pull_secret_names(message: &str) -> Option<&str> {
    let (_, rest) = message.split_once('(')?;
    let (names, _) = rest.split_once(')')?;
    (!names.is_empty()).then_some(names)
}

/// C2 to C4.
fn crash_loop_text(container: &ContainerSummary, message: Option<&str>) -> String {
    let Some(termination) = &container.last_termination else {
        return match message {
            Some(message) => format!("CrashLoopBackOff: {message}"),
            None => "CrashLoopBackOff.".to_owned(),
        };
    };
    let when = match run_duration(termination) {
        Some(run) => format!("about {run} after each start"),
        None => "on each start".to_owned(),
    };
    if termination.reason == Some(StatusReason::OomKilled) {
        let head = format!("OOMKilled (exit {}) {when}", termination.exit_code);
        return match memory_limit(container) {
            Some(limit) => format!("{head}: memory hits the {limit} limit."),
            None => format!("{head}. No memory limit is set."),
        };
    }
    let exit = match &termination.reason {
        Some(reason) => format!("{reason} (exit {})", termination.exit_code),
        None => format!("code {}", termination.exit_code),
    };
    let restarts = container.restart_count;
    let unit = if restarts == 1 { "time" } else { "times" };
    format!("Exits with {exit} {when}. Restarted {restarts} {unit}.")
}

/// C6.
fn terminated_problem(container: &ContainerSummary, termination: &Termination) -> Problem {
    let reason = termination
        .reason
        .as_ref()
        .map_or_else(|| "Error".to_owned(), ToString::to_string);
    let mut text = format!("Exited with {reason} (exit {}).", termination.exit_code);
    if termination.reason == Some(StatusReason::OomKilled)
        && let Some(limit) = memory_limit(container)
    {
        text.push_str(&format!(" Memory limit {limit}."));
    }
    Problem {
        tone: StatusTone::Bad,
        text,
        cause: DiagnosisCause::Exited {
            reason: termination.reason.clone(),
        },
    }
}

/// C7 and C8.
fn running_problem(
    container: &ContainerSummary,
    events: Option<&[EventSummary]>,
    now: Timestamp,
) -> Option<Problem> {
    let newest = |kind| events.and_then(|events| probe_failure(container, kind, events));
    if container.probes.startup.is_some() && container.is_started != Some(true) {
        let mut text = "Startup probe has not passed yet.".to_owned();
        if let Some(event) = newest(ProbeKind::Startup) {
            text.push_str(&event_suffix(event, now));
        }
        return Some(Problem {
            tone: StatusTone::Warn,
            text,
            cause: DiagnosisCause::StartupPending,
        });
    }
    let is_ready_expected = matches!(container.kind, ContainerKind::Main | ContainerKind::Sidecar);
    if container.is_ready || !is_ready_expected {
        return None;
    }
    let mut text = "Running but not ready.".to_owned();
    match (
        newest(ProbeKind::Readiness),
        container.probes.readiness.as_ref(),
    ) {
        (Some(event), _) => text.push_str(&event_suffix(event, now)),
        (None, Some(probe)) => text.push_str(&format!(
            " The readiness probe ({}) has not passed.",
            probe_summary_text(probe)
        )),
        (None, None) => {}
    }
    Some(Problem {
        tone: StatusTone::Warn,
        text,
        cause: DiagnosisCause::NotReady,
    })
}

/// ` {message} (×N, {age} ago)` for the newest failure event. A probe that failed without output
/// leaves the message ending in a colon, which reads `failed (no output)` instead.
fn event_suffix(event: &EventSummary, now: Timestamp) -> String {
    let line = message_line(&event.message);
    let message = match line.strip_suffix(':') {
        Some(head) => format!("{head} (no output)"),
        None => line,
    };
    format!(
        " {message} (×{}, {} ago)",
        event.count,
        format_age(event.last_seen, now)
    )
}

/// The termination's run time, when both timestamps are known.
fn run_duration(termination: &Termination) -> Option<String> {
    let (started_at, finished_at) = (termination.started_at?, termination.finished_at?);
    Some(format_age(Some(started_at), finished_at))
}

fn memory_limit(container: &ContainerSummary) -> Option<&str> {
    container
        .resources
        .iter()
        .find(|resource| resource.name == "memory")
        .and_then(|resource| resource.limit.as_deref())
}

pub(crate) fn probe_result(
    pod: &PodSummary,
    container: &ContainerSummary,
    kind: ProbeKind,
    events: Option<&[EventSummary]>,
) -> ProbeResult {
    if probe_of(container, kind).is_none() {
        return ProbeResult::NotSet;
    }
    let is_running = matches!(container.state, ContainerState::Running { .. });
    if pod.status == PodStatus::Terminating || !is_running {
        return ProbeResult::Inactive;
    }
    let is_waiting_for_startup = container.probes.startup.is_some()
        && container.is_started != Some(true)
        && kind != ProbeKind::Startup;
    if is_waiting_for_startup {
        return ProbeResult::WaitingForStartup;
    }
    let failure = events.and_then(|events| probe_failure(container, kind, events));
    let failing = ProbeResult::Failing {
        failures: failure.map_or(0, |event| event.count),
    };
    match kind {
        ProbeKind::Readiness if container.is_ready => ProbeResult::Passing,
        ProbeKind::Readiness => failing,
        ProbeKind::Startup if container.is_started == Some(true) => ProbeResult::Passed,
        ProbeKind::Startup if failure.is_some() => failing,
        ProbeKind::Startup => ProbeResult::Pending,
        ProbeKind::Liveness if events.is_none() => ProbeResult::NoData,
        ProbeKind::Liveness if failure.is_some() => failing,
        ProbeKind::Liveness => ProbeResult::Passing,
    }
}

pub(crate) fn probe_of(container: &ContainerSummary, kind: ProbeKind) -> Option<&ProbeSummary> {
    match kind {
        ProbeKind::Liveness => container.probes.liveness.as_ref(),
        ProbeKind::Readiness => container.probes.readiness.as_ref(),
        ProbeKind::Startup => container.probes.startup.as_ref(),
    }
}

/// The newest probe failure event of `kind` for this container since it last started. An
/// event series is one event, so `count` of the newest match is the failure count; counts of
/// different events are never summed.
fn probe_failure<'a>(
    container: &ContainerSummary,
    kind: ProbeKind,
    events: &'a [EventSummary],
) -> Option<&'a EventSummary> {
    let started_at = match &container.state {
        ContainerState::Running { started_at } => *started_at,
        _ => None,
    };
    let prefix = format!("{} probe", kind.name());
    let matches = events.iter().filter(|event| {
        event.reason == "Unhealthy"
            && event.container.as_deref() == Some(container.name.as_str())
            && event.message.starts_with(&prefix)
            && started_at.is_none_or(|started_at| {
                event
                    .last_seen
                    .is_some_and(|last_seen| last_seen >= started_at)
            })
    });
    // Strictly newer wins, so events that tie keep list order (the watch lists newest first).
    let mut newest: Option<&EventSummary> = None;
    for event in matches {
        if newest.is_none_or(|known| event.last_seen > known.last_seen) {
            newest = Some(event);
        }
    }
    newest
}

/// CrashLoopBackOff only: the time left until the kubelet retries. The kubelet's waiting
/// message reads `back-off 5m0s restarting failed container=…` and the back-off is measured
/// from the last termination's end.
pub(crate) fn next_retry(container: &ContainerSummary, now: Timestamp) -> Option<SignedDuration> {
    let ContainerState::Waiting {
        reason: Some(StatusReason::CrashLoopBackOff),
        message: Some(message),
    } = &container.state
    else {
        return None;
    };
    let finished_at = container.last_termination.as_ref()?.finished_at?;
    let (_, after) = message.split_once("back-off ")?;
    let back_off = parse_go_duration(after.split_whitespace().next()?)?;
    let remaining = finished_at
        .as_second()
        .saturating_add(back_off)
        .saturating_sub(now.as_second());
    (remaining > 0).then(|| SignedDuration::from_secs(remaining))
}

/// Whole seconds of a Go duration made of integer `h`, `m`, and `s` parts, such as `1h0m0s`.
fn parse_go_duration(text: &str) -> Option<i64> {
    let mut total = 0_i64;
    let mut digits = String::new();
    let mut has_part = false;
    for character in text.chars() {
        if character.is_ascii_digit() {
            digits.push(character);
            continue;
        }
        let unit = match character {
            'h' => 3_600,
            'm' => 60,
            's' => 1,
            _ => return None,
        };
        let count: i64 = digits.parse().ok()?;
        total = total.saturating_add(count.saturating_mul(unit));
        digits.clear();
        has_part = true;
    }
    (digits.is_empty() && has_part).then_some(total)
}

#[cfg(test)]
#[path = "pod_diagnosis_tests.rs"]
mod pod_diagnosis_tests;
