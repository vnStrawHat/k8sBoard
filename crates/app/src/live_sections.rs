//! Drawer content computed at paint time from a row's `KindObject` and the session's live
//! lists: Deployment revisions, CronJob next runs and recent jobs, DaemonSet pods that are not
//! ready, Service endpoints. The row builders only leave a `DetailRow::Live` placeholder;
//! everything here reads the live state when it paints, so it never goes stale. The pure helpers
//! are tested without a window.

use cluster::{
    ConfigMapSummary, ConfigMapValues, CronJobSummary, CronSchedule, DeploymentSummary,
    EndpointSliceSummary, JobSummary, NodeSummary, PodSummary, ReplicaSetSummary, ServiceSummary,
    ValuePreview,
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
use crate::cluster_session::{
    CompanionLists, CompanionPlan, LiveCluster, LiveList, RelatedList, companion_plan,
};
use crate::config_map_rows::{format_bytes, key_size_text};
use crate::drawer::{link_text, wide_detail_row};
use crate::kind_diagnosis::{is_pod_not_ready, unready_node};
use crate::kind_join::{
    EndpointState, UsedBy, config_map_users, endpoint_entries, endpoint_ports, service_slices,
    users_of,
};
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
        (LiveContent::Endpoints, KindObject::Service(service)) => {
            endpoints(kind, service, live, cx)
        }
        (LiveContent::UsedBy, KindObject::ConfigMap(config_map)) => {
            used_by_rows(config_map, live, cx)
        }
        (LiveContent::ConfigMapData, KindObject::ConfigMap(config_map)) => {
            config_map_data_rows(kind, row, config_map, live, cx)
        }
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
        Some(RelatedList::Jobs(_) | RelatedList::ConfigMapValues(_)) | None => None,
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
        Some(RelatedList::ReplicaSets(_) | RelatedList::ConfigMapValues(_)) | None => None,
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

// ---- Endpoints ----

/// How many endpoints a Service drawer lists.
const MAX_LISTED_ENDPOINTS: usize = 50;

/// One endpoint of a Service as the drawer lists it.
#[derive(Debug, PartialEq, Eq)]
struct EndpointRow {
    /// `10.0.0.5:8080 · api-7d9f8c-x2k4q`; the port moves to a "Ports" field when there are
    /// several.
    text: String,
    state: EndpointState,
    /// The pod behind the endpoint, when it names one.
    pod: Option<ResourceKey>,
}

#[derive(Debug, PartialEq, Eq)]
struct EndpointsContent {
    /// `8080/TCP, 9090/TCP`, only when the Service has several ports.
    ports: Option<String>,
    rows: Vec<EndpointRow>,
}

/// The endpoints of `service` from `slices`, each pod once, in slice order.
fn endpoints_content(
    service: &ServiceSummary,
    slices: &[EndpointSliceSummary],
) -> EndpointsContent {
    let slices = service_slices(service, slices);
    let mut ports = endpoint_ports(&slices);
    // The Ports section above lists them in the Service's order; endpoint ports match by name.
    ports.sort_by_key(|port| {
        service
            .ports
            .iter()
            .position(|own| own.name == port.name)
            .unwrap_or(usize::MAX)
    });
    // With one port every row shows it, so the address and the port read as one `ip:port`.
    let single_port = match ports.as_slice() {
        [only] => only.port,
        _ => None,
    };
    let ports = (ports.len() > 1).then(|| {
        ports
            .iter()
            .filter_map(|port| Some(format!("{}/{}", port.port?, port.protocol)))
            .collect::<Vec<_>>()
            .join(", ")
    });
    let rows =
        endpoint_entries(&slices)
            .into_iter()
            .map(|entry| {
                let address = match single_port {
                    // An IPv6 address holds colons, so it needs brackets before a port.
                    Some(port) if entry.endpoint.address.contains(':') => {
                        format!("[{}]:{port}", entry.endpoint.address)
                    }
                    Some(port) => format!("{}:{port}", entry.endpoint.address),
                    None => entry.endpoint.address.clone(),
                };
                let text = match &entry.endpoint.pod {
                    Some(pod) => format!("{address} · {pod}"),
                    None => address,
                };
                EndpointRow {
                    text,
                    state: entry.state,
                    pod: entry.endpoint.pod.as_deref().and_then(|pod| {
                        ResourceKey::of_object("Pod", Some(&service.namespace), pod)
                    }),
                }
            })
            .collect();
    EndpointsContent { ports, rows }
}

fn endpoint_state_label(state: EndpointState) -> StatusLabel {
    let (text, tone) = match state {
        EndpointState::Ready => ("ready", StatusTone::Ok),
        EndpointState::NotReady => ("not ready", StatusTone::Bad),
        EndpointState::Terminating => ("terminating", StatusTone::Done),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

fn endpoints(
    kind: ResourceKind,
    service: &ServiceSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let Some(slices) = live.companion().and_then(CompanionLists::endpoint_slices) else {
        // Without a companion the report denied it, or the explorer has not started it yet.
        if let CompanionPlan::Denied(check) = companion_plan(kind, &live.access) {
            return vec![note(&format!("Not permitted: {check}"), cx)];
        }
        return vec![note("Loading endpoints…", cx)];
    };
    match slices {
        LiveList::Loading => vec![note("Loading endpoints…", cx)],
        LiveList::Failed { message } => vec![
            note("Endpoints are unavailable", cx),
            detail_note(message, cx),
        ],
        LiveList::Ready { items, .. } => {
            let content = endpoints_content(service, items);
            if content.rows.is_empty() {
                return vec![note("No endpoints", cx)];
            }
            let hidden = content.rows.len().saturating_sub(MAX_LISTED_ENDPOINTS);
            content
                .ports
                .map(|ports| {
                    wide_detail_row("Ports", div().truncate().child(ports), cx).into_any_element()
                })
                .into_iter()
                .chain(
                    content
                        .rows
                        .iter()
                        .take(MAX_LISTED_ENDPOINTS)
                        .enumerate()
                        .map(|(ix, row)| endpoint_element(ix, row, cx)),
                )
                .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
                .collect()
        }
    }
}

fn endpoint_element(ix: usize, row: &EndpointRow, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let mut element = h_flex()
        .id(("endpoint", ix))
        .gap_2()
        .items_center()
        .py_1()
        .rounded(theme.radius)
        .text_sm();
    if let Some(target) = row.pod.clone() {
        let hover_bg = theme.muted;
        element = element
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(target.clone(), cx)));
    }
    element
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(theme.mono_font_family.clone())
                .child(row.text.clone()),
        )
        .child(toned_text(endpoint_state_label(row.state), cx).flex_shrink_0())
        .into_any_element()
}

// ---- ConfigMap data ----

/// One key of a ConfigMap as the Data section shows it. No `Debug` outside tests: the text can
/// be a config value.
#[derive(PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
struct DataLine {
    key: String,
    text: String,
}

/// A line per key of the summary: the value preview when the related watch has delivered it, else
/// the size. The previews are never logged; they can be sensitive.
fn data_lines(config_map: &ConfigMapSummary, values: Option<&ConfigMapValues>) -> Vec<DataLine> {
    config_map
        .keys
        .iter()
        .map(|key| {
            let preview = values
                .and_then(|values| values.entries.iter().find(|entry| entry.key == key.name))
                .map(|entry| preview_text(&entry.preview));
            DataLine {
                key: key.name.clone(),
                text: preview.unwrap_or_else(|| key_size_text(key)),
            }
        })
        .collect()
}

/// A single line as written, else its kind and size: `JSON · 412 B`, `text · 3 lines · 1.2 KiB`,
/// `binary · 2.0 KiB`.
fn preview_text(preview: &ValuePreview) -> String {
    match preview {
        ValuePreview::Line(line) => line.clone(),
        ValuePreview::Json { size_bytes } => format!("JSON · {}", format_bytes(*size_bytes)),
        ValuePreview::Text { size_bytes, lines } => format!(
            "text · {lines} {} · {}",
            if *lines == 1 { "line" } else { "lines" },
            format_bytes(*size_bytes)
        ),
        ValuePreview::Binary { size_bytes } => format!("binary · {}", format_bytes(*size_bytes)),
    }
}

fn config_map_data_rows(
    kind: ResourceKind,
    row: &KindRow,
    config_map: &ConfigMapSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    if config_map.keys.is_empty() {
        return vec![note("No keys", cx)];
    }
    let list = match related_subject(kind, row).and_then(|subject| live.related_of(&subject)) {
        Some(RelatedList::ConfigMapValues(list)) => Some(list),
        Some(RelatedList::ReplicaSets(_) | RelatedList::Jobs(_)) | None => None,
    };
    let values = list.and_then(|list| {
        list.ready_items()?.iter().find(|values| {
            values.namespace == config_map.namespace && values.name == config_map.name
        })
    });
    let mono = cx.theme().mono_font_family.clone();
    let mut rows: Vec<AnyElement> = data_lines(config_map, values)
        .into_iter()
        .map(|line| {
            let value = div().truncate().font_family(mono.clone()).child(line.text);
            wide_detail_row(line.key, value, cx).into_any_element()
        })
        .collect();
    // Until the values arrive the sizes stand in, so a failure only adds why.
    if let Some(message) = list.and_then(LiveList::failure) {
        rows.push(note(&format!("Values are unavailable: {message}"), cx));
    }
    rows
}

// ---- Used by ----

/// How many users a ConfigMap drawer lists.
const MAX_LISTED_USERS: usize = 20;

fn used_by_rows(
    config_map: &ConfigMapSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    if live.pods.is_loading() {
        return vec![note("Loading pods…", cx)];
    }
    let Some(pods) = live.pods.ready_items() else {
        return vec![note("Pods are unavailable", cx)];
    };
    let scope = live.scope_label();
    // Only the pods of the ConfigMap's namespace can use it, so the index stays small per paint.
    let users = config_map_users(
        pods.iter()
            .filter(|pod| pod.namespace == config_map.namespace),
    );
    let users: Vec<&UsedBy> = users_of(&users, &config_map.namespace, &config_map.name).collect();
    if users.is_empty() {
        return vec![note(&format!("Not used by any pod in {scope}"), cx)];
    }
    let hidden = users.len().saturating_sub(MAX_LISTED_USERS);
    users
        .iter()
        .take(MAX_LISTED_USERS)
        .enumerate()
        .map(|(ix, used_by)| used_by_element(ix, used_by, cx))
        .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
        .chain(std::iter::once(note(&format!("From pods in {scope}"), cx)))
        .collect()
}

fn used_by_element(ix: usize, used_by: &UsedBy, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let ways = used_by.ways.iter().copied().collect::<Vec<_>>().join(", ");
    let owner = match used_by.target.clone() {
        Some(target) => link_text(ix, &used_by.owner.clone().into(), target, cx),
        None => div()
            .truncate()
            .font_family(theme.mono_font_family.clone())
            .child(used_by.owner.clone())
            .into_any_element(),
    };
    h_flex()
        .id(("used-by", ix))
        .gap_2()
        .items_center()
        .py_1()
        .text_sm()
        .child(div().flex_1().min_w_0().child(owner))
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(ways),
        )
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
