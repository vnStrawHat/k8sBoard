//! The end of a run the app started (spec 0032).
//!
//! Trigger now, Re-run, and a Resume that lets the controller start a missed run return once the
//! Job exists, so their toast only says it was created. This follows the Job to its end and says
//! how it went: `Job x failed: exit 3`, `Job x succeeded in 12s`.

use std::time::{Duration, Instant};

use cluster::{
    ClusterConnection, ControllerRef, CronJobSummary, JobStatus, JobSummary, PodSummary,
};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::{AnyWindowHandle, AppContext as _, Context, SharedString};

use super::AppShell;
use crate::age::format_age;
use crate::batch_rows::missed_run;
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::kind_diagnosis::first_main_termination;
use crate::live_sections::run_label;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::workload_actions::next_run_label;

const POLL: Duration = Duration::from_secs(3);
const RESUME_POLL: Duration = Duration::from_secs(2);
/// A Job that runs longer than this is left alone: the Jobs list shows it.
const JOB_WATCH_LIMIT: Duration = Duration::from_secs(10 * 60);
/// The controller syncs CronJobs every 10 seconds, so a missed run starts within this.
const RESUME_WATCH_LIMIT: Duration = Duration::from_secs(30);
const JOB_KIND: &str = "Job";

/// Marks the end toast of one Job, so a second run of the same name replaces it.
struct JobToast;

/// The toast that ends a Job and whether it is good news; `None` while the Job still runs.
/// `exit_code` is the one a failed pod of the Job reported.
pub(crate) fn job_end_text(job: &JobSummary, exit_code: Option<i32>) -> Option<(String, bool)> {
    match job.status {
        JobStatus::Complete => {
            let text = match job.started_at {
                Some(started) => format!(
                    "Job {} succeeded in {}",
                    job.name,
                    format_age(Some(started), job.finished_at.unwrap_or(started))
                ),
                None => format!("Job {} succeeded", job.name),
            };
            Some((text, true))
        }
        JobStatus::Failed => Some((
            format!(
                "Job {} failed: {}",
                job.name,
                failure_reason(job, exit_code)
            ),
            false,
        )),
        JobStatus::Running | JobStatus::Failing | JobStatus::Suspended => None,
    }
}

/// The exit code, else the reason of the `Failed` condition in words.
fn failure_reason(job: &JobSummary, exit_code: Option<i32>) -> String {
    if let Some(code) = exit_code {
        return format!("exit {code}");
    }
    let reason = job
        .conditions
        .iter()
        .find(|condition| condition.name == "Failed" && condition.is_true)
        .and_then(|condition| condition.reason.as_deref());
    match reason {
        Some("BackoffLimitExceeded") => "backoff limit reached".to_owned(),
        Some("DeadlineExceeded") => "active deadline exceeded".to_owned(),
        Some(other) => other.to_owned(),
        None => "see its pods".to_owned(),
    }
}

/// The exit code of the newest pod of `job` whose main container ended with an error.
pub(crate) fn failed_exit_code(pods: &[PodSummary], job: &JobSummary) -> Option<i32> {
    let owned = |pod: &&PodSummary| {
        pod.namespace == job.namespace
            && pod.controller.as_ref()
                == Some(&ControllerRef {
                    kind: JOB_KIND.to_owned(),
                    name: job.name.clone(),
                })
    };
    pods.iter()
        .filter(owned)
        .filter_map(|pod| {
            let termination = first_main_termination(pod).filter(|term| term.exit_code != 0)?;
            Some((pod.created_at, termination.exit_code))
        })
        .max_by_key(|(created_at, _)| *created_at)
        .map(|(_, code)| code)
}

/// What the app tells once a Resume landed and the CronJob was read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResumeEnd {
    /// The missed run started: the Job the controller made for it.
    Started { job: Option<String> },
    /// The run is past its deadline, or none came due: only the next run is left to say.
    Resumed { skipped: Option<String> },
}

/// The toast after a Resume. `next` is the label of the next run, `None` when the schedule names
/// none.
pub(crate) fn resume_end_text(cron_job: &str, end: &ResumeEnd, next: Option<&str>) -> String {
    let next = next.map_or(String::new(), |next| format!("; next run {next}"));
    match end {
        ResumeEnd::Started { job: Some(job) } => format!("Started job {job} (missed run)"),
        ResumeEnd::Started { job: None } => {
            format!("Resumed cronjob {cron_job}; the missed run started{next}")
        }
        ResumeEnd::Resumed { skipped: Some(run) } => {
            format!(
                "Resumed cronjob {cron_job}; the {run} run was past its deadline and is skipped{next}"
            )
        }
        ResumeEnd::Resumed { skipped: None } => format!("Resumed cronjob {cron_job}{next}"),
    }
}

/// The Job a Resume made, once the CronJob says it scheduled a run since `before` (its last
/// schedule when the Resume was confirmed; a suspended CronJob schedules nothing, so any change is
/// the controller's answer to the Resume): the last active Job, or `None` when it already finished
/// and left the list.
pub(crate) fn started_job(
    cron_job: &CronJobSummary,
    before: Option<jiff::Timestamp>,
) -> Option<Option<&str>> {
    let has_scheduled = cron_job.last_schedule_at.is_some() && cron_job.last_schedule_at != before;
    has_scheduled.then(|| cron_job.active_jobs.last().map(String::as_str))
}

/// What a suspended CronJob had scheduled when its Resume was confirmed. The controller reacts to
/// the Resume at once, so the read after the commit may already show the run it started: only this
/// tells that run from the old one.
pub(crate) struct ResumeBaseline {
    cluster: ClusterRef,
    namespace: String,
    name: String,
    last_schedule_at: Option<jiff::Timestamp>,
}

/// A success or warning toast with a View button that reveals `subject`.
fn notify_job(
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
    text: String,
    is_success: bool,
    subject: Option<(gpui_kit::WeakEntity<AppShell>, ClusterObject)>,
    id: SharedString,
) {
    let notification = if is_success {
        Notification::success(text)
    } else {
        Notification::warning(text)
    };
    let notification = match subject {
        Some((shell, subject)) => notification.action(move |_, _, cx| {
            let (shell, subject) = (shell.clone(), subject.clone());
            Button::new("job-view")
                .label("View")
                .small()
                .outline()
                .on_click(cx.listener(move |notification, _, window, cx| {
                    notification.dismiss(window, cx);
                    let subject = subject.clone();
                    let _ = shell.update(cx, |shell, cx| shell.reveal_object(subject, cx));
                }))
        }),
        None => notification,
    };
    window.push_notification(notification.id1::<JobToast>(id), cx);
}

impl AppShell {
    /// Follows `namespace/name` of `cluster` to its end and announces how it went. Silent when the
    /// cluster closes first, a read fails, or the Job outlasts the limit.
    pub(crate) fn watch_job(
        &mut self,
        cluster: ClusterRef,
        namespace: String,
        name: String,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_of(&cluster, cx) else {
            return;
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        cx.spawn(async move |shell, cx| {
            let started = Instant::now();
            while started.elapsed() < JOB_WATCH_LIMIT {
                let read = runtime.spawn({
                    let (connection, namespace, name) =
                        (connection.clone(), namespace.clone(), name.clone());
                    async move { connection.job(&namespace, &name).await }
                });
                let Ok(Ok(job)) = read.await else {
                    return;
                };
                let end = shell.read_with(cx, |shell, cx| {
                    let live = shell.live_of(&cluster, cx)?;
                    Some(job_end_text(
                        &job,
                        failed_exit_code(live.pods.items(), &job),
                    ))
                });
                let Ok(end) = end else {
                    return;
                };
                if let Some((text, is_success)) = end.flatten() {
                    let key = ResourceKey::of_object(JOB_KIND, Some(&namespace), &name);
                    let subject = key.map(|key| (shell.clone(), ClusterObject::new(cluster, key)));
                    let id: SharedString = format!("{namespace}/{name}").into();
                    let _ = cx.update_window(window, |_, window, cx| {
                        notify_job(window, cx, text, is_success, subject, id);
                    });
                    return;
                }
                cx.background_executor().timer(POLL).await;
            }
        })
        .detach();
    }

    /// Remembers what `cron_job` had scheduled, for the toast that follows its Resume.
    pub(crate) fn note_resume_baseline(&mut self, cluster: &ClusterRef, cron_job: &CronJobSummary) {
        self.resume_baseline = Some(ResumeBaseline {
            cluster: cluster.clone(),
            namespace: cron_job.namespace.clone(),
            name: cron_job.name.clone(),
            last_schedule_at: cron_job.last_schedule_at,
        });
    }

    /// The baseline of the Resume of `cluster`/`namespace`/`name`, once; `None` for another object.
    fn take_resume_baseline(
        &mut self,
        cluster: &ClusterRef,
        namespace: &str,
        name: &str,
    ) -> Option<Option<jiff::Timestamp>> {
        let baseline = self.resume_baseline.take()?;
        (baseline.cluster == *cluster && baseline.namespace == namespace && baseline.name == name)
            .then_some(baseline.last_schedule_at)
    }

    /// After a Resume of the CronJob `namespace/name`: reads it back and says what the controller
    /// did with the run that came due while it was suspended, then follows the Job it started.
    pub(crate) fn watch_resume(
        &mut self,
        cluster: ClusterRef,
        namespace: String,
        name: String,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let baseline = self.take_resume_baseline(&cluster, &namespace, &name);
        let Some(connection) = self.connection_of(&cluster, cx) else {
            return;
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        cx.spawn(async move |shell, cx| {
            let Some(mut cron_job) = read_cron_job(&runtime, &connection, &namespace, &name).await
            else {
                return;
            };
            // Without a baseline (a Resume that did not come through the confirm) the first read
            // is the one to compare with.
            let before = baseline.unwrap_or(cron_job.last_schedule_at);
            let started = Instant::now();
            let mut is_decided = false;
            loop {
                let now = jiff::Timestamp::now();
                let next = next_run_label(&cron_job, now);
                if let Some(job) = started_job(&cron_job, before) {
                    let end = ResumeEnd::Started {
                        job: job.map(str::to_owned),
                    };
                    let text = resume_end_text(&name, &end, next.as_deref());
                    let subject = job.and_then(|job| {
                        let key = ResourceKey::of_object(JOB_KIND, Some(&namespace), job)?;
                        Some((shell.clone(), ClusterObject::new(cluster.clone(), key)))
                    });
                    notify_resume(cx, window, text, true, subject);
                    if let Some(job) = job {
                        let job = job.to_owned();
                        let _ = shell.update(cx, |shell, cx| {
                            shell.watch_job(cluster, namespace, job, window, cx);
                        });
                    }
                    return;
                }
                // The controller has not scheduled anything yet: a run that came due while the
                // CronJob was suspended is the one it starts, if its deadline still allows.
                if !is_decided {
                    is_decided = true;
                    let run = missed_run(&cron_job, now);
                    if !run.is_some_and(|run| run.starts) {
                        let skipped = run.map(|run| {
                            run_label(&run.at.to_zoned(jiff::tz::TimeZone::system()), now)
                        });
                        let end = ResumeEnd::Resumed { skipped };
                        let text = resume_end_text(&name, &end, next.as_deref());
                        notify_resume(cx, window, text, true, None);
                        return;
                    }
                }
                if started.elapsed() >= RESUME_WATCH_LIMIT {
                    let text =
                        format!("Resumed cronjob {name}; the missed run has not started yet");
                    notify_resume(cx, window, text, false, None);
                    return;
                }
                cx.background_executor().timer(RESUME_POLL).await;
                let Some(read) = read_cron_job(&runtime, &connection, &namespace, &name).await
                else {
                    return;
                };
                cron_job = read;
            }
        })
        .detach();
    }
}

async fn read_cron_job(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    namespace: &str,
    name: &str,
) -> Option<CronJobSummary> {
    let read = runtime.spawn({
        let (connection, namespace, name) =
            (connection.clone(), namespace.to_owned(), name.to_owned());
        async move { connection.cron_job(&namespace, &name).await }
    });
    read.await.ok()?.ok()
}

fn notify_resume(
    cx: &mut gpui_kit::AsyncApp,
    window: AnyWindowHandle,
    text: String,
    is_success: bool,
    subject: Option<(gpui_kit::WeakEntity<AppShell>, ClusterObject)>,
) {
    let id: SharedString = format!("resume {text}").into();
    let _ = cx.update_window(window, |_, window, cx| {
        notify_job(window, cx, text, is_success, subject, id);
    });
}

#[cfg(test)]
#[path = "job_watch_tests.rs"]
mod job_watch_tests;
