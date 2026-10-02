//! Drawer content computed at paint time from a row's `KindObject` and the session's live
//! lists: Deployment revisions, CronJob next runs and recent jobs, DaemonSet pods that are not
//! ready. The row builders only leave
//! a `DetailRow::Live` placeholder; everything here reads the live state when it paints, so it
//! never goes stale. The pure helpers are tested without a window.

use cluster::{
    CronJobSummary, CronSchedule, DeploymentSummary, JobSummary, NodeSummary, PodSummary,
    ReplicaSetSummary,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::batch_rows::job_status_label;
use crate::cluster_session::{LiveCluster, LiveList, RelatedList};
use crate::drawer::wide_detail_row;
use crate::kind_diagnosis::{is_pod_not_ready, unready_node};
use crate::kind_row::{KindObject, KindRow, LiveContent, owns_pod};
use crate::related_objects::related_subject;
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusLabel, StatusTone, pod_status_label, readiness_text, toned_text};
use crate::table_selection::ResourceKey;

/// Bounds the render cost of a Deployment with very many ReplicaSets or a CronJob with many jobs.
const MAX_LISTED_OBJECTS: usize = 10;
/// How many upcoming runs a CronJob drawer shows.
const NEXT_RUN_COUNT: usize = 3;
/// The fixed widths of a revision row's right-hand columns.
const AGE_SLOT: Pixels = px(64.);
const STATE_SLOT: Pixels = px(54.);
const READY_SLOT: Pixels = px(34.);
const BUTTON_SLOT: Pixels = px(76.);

/// The rows of one live section. A section whose data is not loaded shows one muted note.
pub(crate) fn live_rows(
    content: LiveContent,
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    match (content, &row.object) {
        (LiveContent::Revisions, KindObject::Deployment(deployment)) => {
            revisions(kind, row, deployment, live, now, cx)
        }
        (LiveContent::NextRuns, KindObject::CronJob(cron_job)) => next_runs(cron_job, now, cx),
        (LiveContent::RecentJobs, KindObject::CronJob(cron_job)) => {
            recent_jobs_rows(kind, row, cron_job, live, now, cx)
        }
        (LiveContent::NotReadyPods, KindObject::DaemonSet(_)) => not_ready_rows(row, live, cx),
        // A placeholder on a row of another kind has nothing to show.
        _ => Vec::new(),
    }
}

/// `in 11m`: how far away the next run is, or `None` when there is none.
pub(crate) fn next_run_text(schedule: &CronSchedule, now: jiff::Timestamp) -> Option<String> {
    let next = schedule.next_after(now)?;
    Some(format!("in {}", format_age(Some(now), next.timestamp())))
}

// ---- Revisions ----

/// One ReplicaSet of a Deployment, as a revision row.
#[derive(Debug, PartialEq, Eq)]
struct Revision<'a> {
    replica_set: &'a ReplicaSetSummary,
    /// The numeric `deployment.kubernetes.io/revision`; a ReplicaSet without one sorts last.
    number: Option<u64>,
    /// The revision the Deployment runs now.
    is_current: bool,
}

/// The ReplicaSets the Deployment owns, newest revision first. A name that merely starts with the
/// Deployment's name does not count: ownership comes from the controller reference.
fn revision_rows<'a>(
    deployment: &DeploymentSummary,
    replica_sets: &'a [ReplicaSetSummary],
) -> Vec<Revision<'a>> {
    let mut revisions: Vec<Revision<'a>> = replica_sets
        .iter()
        .filter(|set| {
            set.namespace == deployment.namespace
                && set.owner.as_ref().is_some_and(|owner| {
                    owner.kind == "Deployment" && owner.name == deployment.name
                })
        })
        .map(|replica_set| Revision {
            replica_set,
            number: replica_set
                .revision
                .as_deref()
                .and_then(|text| text.parse().ok()),
            is_current: replica_set.revision.is_some()
                && replica_set.revision == deployment.revision,
        })
        .collect();
    revisions.sort_by(|a, b| {
        // `None` sorts after every number, then the newest number first.
        b.number
            .is_some()
            .cmp(&a.number.is_some())
            .then(b.number.cmp(&a.number))
            .then_with(|| a.replica_set.name.cmp(&b.replica_set.name))
    });
    revisions
}

/// The tag of an image reference: the text after the last `:` that follows the last `/`, else
/// the whole reference (a registry port such as `registry:5000/api` is not a tag).
fn image_tag(image: &str) -> &str {
    let name_start = image.rfind('/').map_or(0, |slash| slash + 1);
    match image[name_start..].rfind(':') {
        Some(colon) => &image[name_start + colon + 1..],
        None => image,
    }
}

fn revisions(
    kind: ResourceKind,
    row: &KindRow,
    deployment: &DeploymentSummary,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let Some(subject) = related_subject(kind, row) else {
        return vec![note("No ReplicaSets", cx)];
    };
    let list = match live.related_of(&subject) {
        Some(RelatedList::ReplicaSets(list)) => Some(list),
        Some(RelatedList::Jobs(_)) | None => None,
    };
    match list {
        None | Some(LiveList::Loading) => vec![note("Loading revisions…", cx)],
        Some(LiveList::Failed { message }) => {
            vec![
                note("Revisions are unavailable", cx),
                detail_note(message, cx),
            ]
        }
        Some(LiveList::Ready { items, .. }) => {
            let revisions = revision_rows(deployment, items);
            if revisions.is_empty() {
                return vec![note("No ReplicaSets", cx)];
            }
            let hidden = revisions.len().saturating_sub(MAX_LISTED_OBJECTS);
            revisions
                .iter()
                .take(MAX_LISTED_OBJECTS)
                .enumerate()
                .map(|(ix, revision)| revision_element(ix, revision, now, cx))
                .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
                .collect()
        }
    }
}

fn revision_element(
    ix: usize,
    revision: &Revision,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let set = revision.replica_set;
    let number = set.revision.as_deref().unwrap_or("—");
    let tag = set
        .containers
        .first()
        .map(|container| image_tag(&container.image))
        .filter(|tag| !tag.is_empty());
    let title = match tag {
        Some(tag) => format!("rev {number} · {tag}"),
        None => format!("rev {number}"),
    };
    let age = match set.created_at {
        Some(_) => format!("{} ago", format_age(set.created_at, now)),
        None => "—".to_owned(),
    };
    let current_tone = if set.ready >= set.desired {
        StatusTone::Ok
    } else {
        StatusTone::Warn
    };
    let hover_bg = theme.muted;
    let target = ResourceKey::of_object("ReplicaSet", Some(&set.namespace), &set.name);
    h_flex()
        .id(("revision", ix))
        .gap_2()
        .items_center()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, _, cx| {
            if let Some(target) = &target {
                shell.reveal(target.clone(), cx);
            }
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(theme.mono_font_family.clone())
                .child(title),
        )
        // Fixed slots keep the columns aligned from row to row: the current row leaves the
        // button slot empty, and the others leave the state slot empty.
        .child(
            div()
                .w(AGE_SLOT)
                .flex_shrink_0()
                .text_right()
                .text_color(theme.muted_foreground)
                .child(age),
        )
        .child(
            div()
                .w(STATE_SLOT)
                .flex_shrink_0()
                .children(revision.is_current.then(|| {
                    toned_text(
                        StatusLabel {
                            text: "current".into(),
                            tone: current_tone,
                        },
                        cx,
                    )
                })),
        )
        .child(
            div()
                .w(READY_SLOT)
                .flex_shrink_0()
                .text_right()
                .child(format!("{}/{}", set.ready, set.desired)),
        )
        .child(
            div()
                .w(BUTTON_SLOT)
                .flex_shrink_0()
                .flex()
                .justify_end()
                .children((!revision.is_current).then(|| {
                    Button::new(("roll-back", ix))
                        .label("Roll back")
                        .xsmall()
                        .ghost()
                        .disabled(true)
                        .tooltip("Read-only mode")
                })),
        )
        .into_any_element()
}

// ---- Next runs ----

/// What the Next runs section shows.
#[derive(Debug, PartialEq, Eq)]
enum NextRunsContent {
    Note(String),
    /// `(label, "in 11m")` pairs, soonest first.
    Runs(Vec<(String, String)>),
}

fn next_runs_content(cron_job: &CronJobSummary, now: jiff::Timestamp) -> NextRunsContent {
    if cron_job.is_suspended {
        return NextRunsContent::Note("Suspended: no runs are scheduled".to_owned());
    }
    let schedule = match &cron_job.timetable {
        Ok(schedule) => schedule,
        Err(error) => {
            return NextRunsContent::Note(format!("Cannot compute next runs: {error}"));
        }
    };
    let runs = schedule.next_runs(now, NEXT_RUN_COUNT);
    if runs.is_empty() {
        // An `@every` schedule has no anchor before its first run; any other has no run within
        // the controller's five-year horizon (such as February 30th).
        let text = if schedule.is_every() {
            "Next run is known after the first run"
        } else {
            "No run within the next 5 years"
        };
        return NextRunsContent::Note(text.to_owned());
    }
    NextRunsContent::Runs(
        runs.iter()
            .map(|run| {
                let label = run_label(run, now);
                let away = format!("in {}", format_age(Some(now), run.timestamp()));
                (label, away)
            })
            .collect(),
    )
}

/// `10:45 UTC` for a run on today's date in its zone, else `Oct 6 02:30 UTC`.
fn run_label(run: &jiff::Zoned, now: jiff::Timestamp) -> String {
    let today = now.to_zoned(run.time_zone().clone()).date();
    let format = if run.date() == today {
        "%H:%M %Z"
    } else {
        "%b %-d %H:%M %Z"
    };
    run.strftime(format).to_string()
}

fn next_runs(
    cron_job: &CronJobSummary,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    match next_runs_content(cron_job, now) {
        NextRunsContent::Note(text) => vec![note(&text, cx)],
        NextRunsContent::Runs(runs) => runs
            .into_iter()
            .map(|(label, away)| {
                wide_detail_row(label, div().truncate().child(away), cx).into_any_element()
            })
            .collect(),
    }
}

// ---- Recent jobs ----

/// The Jobs the CronJob owns, newest first; a Job without a creation time sorts last.
fn recent_jobs<'a>(cron_job: &CronJobSummary, jobs: &'a [JobSummary]) -> Vec<&'a JobSummary> {
    let mut owned: Vec<&JobSummary> = jobs
        .iter()
        .filter(|job| {
            job.namespace == cron_job.namespace
                && job
                    .owner
                    .as_ref()
                    .is_some_and(|owner| owner.kind == "CronJob" && owner.name == cron_job.name)
        })
        .collect();
    owned.sort_by(|a, b| {
        b.created_at
            .is_some()
            .cmp(&a.created_at.is_some())
            .then(b.created_at.cmp(&a.created_at))
            .then_with(|| a.name.cmp(&b.name))
    });
    owned
}

fn recent_jobs_rows(
    kind: ResourceKind,
    row: &KindRow,
    cron_job: &CronJobSummary,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let list = match related_subject(kind, row).and_then(|subject| live.related_of(&subject)) {
        Some(RelatedList::Jobs(list)) => Some(list),
        Some(RelatedList::ReplicaSets(_)) | None => None,
    };
    match list {
        None | Some(LiveList::Loading) => vec![note("Loading jobs…", cx)],
        Some(LiveList::Failed { message }) => {
            vec![note("Jobs are unavailable", cx), detail_note(message, cx)]
        }
        Some(LiveList::Ready { items, .. }) => {
            let jobs = recent_jobs(cron_job, items);
            if jobs.is_empty() {
                return vec![note("No jobs kept", cx)];
            }
            let hidden = jobs.len().saturating_sub(MAX_LISTED_OBJECTS);
            jobs.iter()
                .take(MAX_LISTED_OBJECTS)
                .enumerate()
                .map(|(ix, job)| job_element(ix, job, now, cx))
                .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
                .collect()
        }
    }
}

fn job_element(
    ix: usize,
    job: &JobSummary,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let hover_bg = theme.muted;
    let target = ResourceKey::of_object("Job", Some(&job.namespace), &job.name);
    let duration = job
        .started_at
        .map(|started| format_age(Some(started), job.finished_at.unwrap_or(now)));
    h_flex()
        .id(("recent-job", ix))
        .gap_2()
        .items_center()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, _, cx| {
            if let Some(target) = &target {
                shell.reveal(target.clone(), cx);
            }
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(theme.mono_font_family.clone())
                .child(job.name.clone()),
        )
        .child(toned_text(job_status_label(job.status), cx).flex_shrink_0())
        .children(duration.map(|duration| {
            div()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(format!("· {duration}"))
        }))
        .into_any_element()
}

// ---- Not ready pods ----

/// How many not-ready pods a DaemonSet drawer lists.
const MAX_NOT_READY_PODS: usize = 20;

/// The pods the row owns, in snapshot order; `None` until the pods list has loaded.
pub(crate) fn owned_pods<'a>(row: &KindRow, live: &'a LiveCluster) -> Option<Vec<&'a PodSummary>> {
    let owner = row.related_pods.as_ref()?;
    let pods = live.pods.ready_items()?;
    Some(pods.iter().filter(|pod| owns_pod(owner, pod)).collect())
}

/// What a not-ready pod reads: `node NotReady` (Bad) when its node is down, else the pod status.
fn not_ready_label(pod: &PodSummary, nodes: &[NodeSummary]) -> StatusLabel {
    match unready_node(pod, nodes) {
        Some((_, readiness)) => StatusLabel {
            text: format!("node {}", readiness_text(readiness)).into(),
            tone: StatusTone::Bad,
        },
        None => pod_status_label(pod),
    }
}

fn not_ready_rows(row: &KindRow, live: &LiveCluster, cx: &Context<AppShell>) -> Vec<AnyElement> {
    if live.pods.is_loading() {
        return vec![note("Loading pods…", cx)];
    }
    let Some(owned) = owned_pods(row, live) else {
        return vec![note("Pods are unavailable", cx)];
    };
    let not_ready: Vec<&PodSummary> = owned
        .into_iter()
        .filter(|pod| is_pod_not_ready(pod))
        .collect();
    if not_ready.is_empty() {
        return vec![note("All pods are ready", cx)];
    }
    let hidden = not_ready.len().saturating_sub(MAX_NOT_READY_PODS);
    not_ready
        .iter()
        .take(MAX_NOT_READY_PODS)
        .enumerate()
        .map(|(ix, pod)| not_ready_element(ix, pod, live.nodes.items(), cx))
        .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
        .collect()
}

fn not_ready_element(
    ix: usize,
    pod: &PodSummary,
    nodes: &[NodeSummary],
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let hover_bg = theme.muted;
    let key = ResourceKey::of_pod(pod);
    // The node tells the pods of a DaemonSet apart; an unscheduled pod has none.
    let title = pod.node_name.clone().unwrap_or_else(|| pod.name.clone());
    h_flex()
        .id(("not-ready-pod", ix))
        .gap_2()
        .items_center()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(key.clone(), cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(theme.mono_font_family.clone())
                .child(title),
        )
        .child(toned_text(not_ready_label(pod, nodes), cx).flex_shrink_0())
        .into_any_element()
}

// ---- shared ----

/// A muted explanation that wraps, such as "Loading jobs…".
fn note(text: &str, cx: &Context<AppShell>) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.to_owned())
        .into_any_element()
}

/// The reason under a "… are unavailable" note.
fn detail_note(text: &str, cx: &Context<AppShell>) -> AnyElement {
    v_flex()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.to_owned())
        .into_any_element()
}

#[cfg(test)]
#[path = "live_sections_tests.rs"]
mod live_sections_tests;
