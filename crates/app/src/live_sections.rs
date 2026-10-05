//! Drawer content computed at paint time from a row's `KindObject` and the session's live
//! lists: Deployment revisions, CronJob next runs and recent jobs, DaemonSet pods that are not
//! ready, Service endpoints. The row builders only leave a `DetailRow::Live` placeholder;
//! everything here reads the live state when it paints, so it never goes stale. The pure helpers
//! are tested without a window.

use cluster::{
    AccessCheck, BindingSummary, BroadGroup, ConfigMapSummary, ConfigMapValues, CronJobSummary,
    CronSchedule, DeploymentSummary, EndpointSliceSummary, EventSummary, Identity, IngressSummary,
    JobSummary, LimitRangeLimit, LimitRangeSummary, NodeSummary, PersistentVolumeClaimSummary,
    PersistentVolumeSummary, PodDisruptionBudgetSummary, PodSummary, PvcUsage, RbacSnapshot,
    ReplicaSetSummary, ResourceQuotaSummary, RoleSummary, SecretSummary, ServiceAccountSummary,
    ServiceSummary, Subject, SubjectKind, ValuePreview, VolumeSource,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};
use jiff::tz::TimeZone;
use std::cell::RefCell;
use std::rc::Rc;

use crate::access_bindings::{
    BindingIndex, BindingsStatus, BoundRole, RoleSubject, binding_key, binding_text,
    bindings_status, pod_account, role_subjects, role_text, subject_text,
};
use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::batch_rows::job_status_label;
use crate::cluster_metrics::FeedStatus;
use crate::cluster_session::{
    CompanionPlan, CompanionSource, LiveCluster, LiveList, RbacState, RelatedList, companion_plan,
    denied_related_check,
};
use crate::config_map_rows::{format_bytes, key_size_text};
use crate::custom_rows::{FieldsSide, conditions_rows, field_list_rows};
use crate::drawer::{link_name, link_text, open_link, truncated_text, wide_detail_row};
use crate::helm_release_view::ValuesLayout;
use crate::helm_rows::{HistoryModel, HistoryRow, history_model};
use crate::kind_diagnosis::{is_pod_not_ready, unready_node};
use crate::kind_drawer::{DrawerPaint, bar_row, live_detail_rows};
use crate::kind_join::{
    EndpointState, UsedBy, WAY_ENV, WAY_ENV_FROM, config_map_users, endpoint_entries,
    endpoint_ports, secret_user_list, secret_users, service_slices, users_of,
};
use crate::kind_join::{UsageSample, claim_sample, is_shared_filesystem};
use crate::kind_row::{DetailRow, KindObject, KindRow, LiveContent, owns_pod, percent};
use crate::network_rows::{TlsSecrets, ingress_tls_rows};
use crate::object_events::event_subject;
use crate::permission_table::{CanDoChips, can_do_chips, permission_table};
use crate::policy_rows::{fullest_item, quota_text};
use crate::related_objects::{RelatedSubject, related_subject};
use crate::resource_actions::ActionAvailability;
use crate::resource_kind::ResourceKind;
use crate::revision_diff::{RevisionDiffRequest, RevisionSide, diff_request};
use crate::secret_rows::{MASK, MaskedKeyRow, certificate_rows, secret_data_rows};
use crate::status_tone::{
    StatusLabel, StatusTone, pod_status_label, readiness_text, tone_color, toned_text,
};
use crate::storage_rows::phase_label;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::usage_format::{Measure, format_percent, usage_tone};
use crate::who_can_view::coverage_notes;
use crate::workload_actions::{PAUSED_REASON, RevisionTarget, image_tag};

/// Bounds the render cost of a Deployment with very many ReplicaSets or a CronJob with many jobs.
const MAX_LISTED_OBJECTS: usize = 10;
/// How many upcoming runs a CronJob drawer shows.
const NEXT_RUN_COUNT: usize = 3;
/// The fixed widths of a revision row's right-hand columns.
const AGE_SLOT: Pixels = px(64.);
const STATE_SLOT: Pixels = px(54.);
const READY_SLOT: Pixels = px(34.);
const BUTTON_SLOT: Pixels = px(76.);
const DIFF_BUTTON_SLOT: Pixels = px(48.);

/// The rows of one live section. A section whose data is not loaded shows one muted note.
pub(crate) fn live_rows(
    content: LiveContent,
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    now: jiff::Timestamp,
    roll_back: Option<&RollBackGate>,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    match (content, &row.object) {
        (LiveContent::Revisions, KindObject::Deployment(deployment)) => {
            revisions(kind, row, deployment, live, now, roll_back, cx)
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
        (LiveContent::UsedBy, KindObject::Secret(secret)) => secret_used_by_rows(secret, live, cx),
        (LiveContent::SecretData, KindObject::Secret(secret)) => masked_rows(secret, cx),
        (LiveContent::IngressTls, KindObject::Ingress(ingress)) => {
            ingress_tls_section(ingress, kind, row, live, now, cx)
        }
        (LiveContent::Certificate, KindObject::Secret(secret)) => live_detail_rows(
            &certificate_rows(&secret.details),
            CERTIFICATE_ID_BASE,
            &DrawerPaint::new(kind, row, live, now),
            cx,
        ),
        (LiveContent::SelectedPods, KindObject::PodDisruptionBudget(budget)) => {
            selected_pods_rows(budget, live, cx)
        }
        (LiveContent::ConfigMapData, KindObject::ConfigMap(config_map)) => {
            config_map_data_rows(kind, row, config_map, live, cx)
        }
        (LiveContent::ScalingEvents, KindObject::HorizontalPodAutoscaler(_)) => {
            scaling_events_rows(kind, row, live, now, cx)
        }
        (LiveContent::BlockedCreations, KindObject::ResourceQuota(quota)) => {
            match related_subject(kind, row) {
                Some(subject) => blocked_creations_rows(&subject, &quota.name, live, now, cx),
                None => Vec::new(),
            }
        }
        (LiveContent::ClaimUsage, KindObject::PersistentVolumeClaim(claim)) => {
            claim_usage_content(claim, live, now, cx)
        }
        (LiveContent::MountedBy, KindObject::PersistentVolumeClaim(claim)) => {
            mounted_by_rows(claim, live, cx)
        }
        (LiveContent::RoleBindings, KindObject::Role(role)) => {
            role_bindings_rows(kind, role, live, cx)
        }
        (LiveContent::RoleSubjects, KindObject::Role(role)) => {
            role_subjects_rows(kind, role, live, cx)
        }
        (LiveContent::BoundRoles, KindObject::ServiceAccount(account)) => {
            bound_roles_rows(kind, account, live, cx)
        }
        (LiveContent::CanDo, KindObject::ServiceAccount(account)) => can_do_rows(account, live, cx),
        (LiveContent::ServiceAccountPods, KindObject::ServiceAccount(account)) => {
            account_pods_rows(account, live, cx)
        }
        (LiveContent::CustomConditions, KindObject::Custom(summary)) => live_detail_rows(
            &conditions_rows(&summary.conditions),
            CUSTOM_CONDITIONS_ID_BASE,
            &DrawerPaint::new(kind, row, live, now),
            cx,
        ),
        (LiveContent::CustomStatus, KindObject::Custom(_)) => custom_fields_section(
            FieldsSide::Status,
            CUSTOM_STATUS_ID_BASE,
            kind,
            row,
            live,
            now,
            cx,
        ),
        (LiveContent::CustomSpec, KindObject::Custom(_)) => custom_fields_section(
            FieldsSide::Spec,
            CUSTOM_SPEC_ID_BASE,
            kind,
            row,
            live,
            now,
            cx,
        ),
        // A StorageClass row holds no summary either: its name is the class.
        (LiveContent::ClassVolumes, _) => class_volumes_rows(kind, &row.name, live, cx),
        // The Namespaces row holds no summary, so its name is the namespace.
        (LiveContent::NamespaceQuotas, _) => namespace_quota_rows(&row.name, live, cx),
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

/// What the Roll back buttons of a drawer need: the Deployment they act on, in its own cluster,
/// and whether the gate of that cluster lets Roll back run. The drawer reads it from the session
/// of its subject, never the primary.
pub(crate) struct RollBackGate {
    pub(crate) subject: ClusterObject,
    pub(crate) availability: ActionAvailability,
}

/// The ReplicaSet list the drawer watches for this row, as it stands: `None` while the drawer
/// watches another object or none.
fn replica_set_list<'a>(
    kind: ResourceKind,
    row: &KindRow,
    live: &'a LiveCluster,
) -> Option<&'a LiveList<ReplicaSetSummary>> {
    let subject = related_subject(kind, row)?;
    match live.related_of(&subject) {
        Some(RelatedList::ReplicaSets(list)) => Some(list),
        Some(
            RelatedList::Jobs(_)
            | RelatedList::ConfigMapValues(_)
            | RelatedList::Events(_)
            | RelatedList::NamespaceLimits { .. }
            | RelatedList::HelmHistory(_)
            | RelatedList::CustomFields(_),
        )
        | None => None,
    }
}

/// The ReplicaSets of a Deployment row once its drawer has loaded them; `None` before that, which
/// is what keeps Roll back off without starting a list for it.
pub(crate) fn loaded_replica_sets<'a>(
    kind: ResourceKind,
    row: &KindRow,
    live: &'a LiveCluster,
) -> Option<&'a [ReplicaSetSummary]> {
    replica_set_list(kind, row, live)?.ready_items()
}

fn revisions(
    kind: ResourceKind,
    row: &KindRow,
    deployment: &DeploymentSummary,
    live: &LiveCluster,
    now: jiff::Timestamp,
    roll_back: Option<&RollBackGate>,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    if related_subject(kind, row).is_none() {
        return vec![note("No ReplicaSets", cx)];
    }
    let list = replica_set_list(kind, row, live);
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
            let key = ResourceKey::of_row(kind, row);
            revisions
                .iter()
                .take(MAX_LISTED_OBJECTS)
                .enumerate()
                .map(|(ix, revision)| {
                    let button = roll_back_button(deployment, revision, roll_back);
                    let diff = diff_of_revision(&key, revision, &revisions);
                    revision_element(ix, revision, now, button, diff, cx)
                })
                .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
                .collect()
        }
    }
}

/// The diff a revision row opens against the revision the Deployment runs: none for the current row,
/// and none when no row is current. Pure.
fn diff_of_revision(
    deployment: &ResourceKey,
    revision: &Revision,
    all: &[Revision],
) -> Option<RevisionDiffRequest> {
    if revision.is_current {
        return None;
    }
    let current = all.iter().find(|candidate| candidate.is_current)?;
    Some(diff_request(
        deployment.clone(),
        RevisionSide::of(revision.replica_set, false),
        RevisionSide::of(current.replica_set, true),
    ))
}

/// What the Roll back button of one revision does.
#[derive(Debug, PartialEq, Eq)]
enum RollBackButton {
    /// Rolls the Deployment back to this revision, after the confirm dialog.
    Enabled(ClusterObject, RevisionTarget),
    Disabled(SharedString),
}

/// The gate of the drawer's cluster decides first (permission, lock), then the Deployment's own
/// state, then whether the ReplicaSet can be named by a revision number.
fn roll_back_button(
    deployment: &DeploymentSummary,
    revision: &Revision,
    gate: Option<&RollBackGate>,
) -> RollBackButton {
    let Some(gate) = gate else {
        return RollBackButton::Disabled("Not connected".into());
    };
    if let ActionAvailability::Disabled { reason } = &gate.availability {
        return RollBackButton::Disabled(reason.clone());
    }
    if deployment.is_paused {
        return RollBackButton::Disabled(PAUSED_REASON.into());
    }
    let Some(number) = revision.number else {
        return RollBackButton::Disabled("This ReplicaSet has no revision number".into());
    };
    let set = revision.replica_set;
    RollBackButton::Enabled(
        gate.subject.clone(),
        RevisionTarget {
            replica_set: set.name.clone(),
            revision: number,
            tag: set
                .containers
                .first()
                .map(|container| image_tag(&container.image))
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned),
        },
    )
}

fn revision_element(
    ix: usize,
    revision: &Revision,
    now: jiff::Timestamp,
    button: RollBackButton,
    diff: Option<RevisionDiffRequest>,
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
        .on_click(cx.listener(move |shell, _, window, cx| {
            if let Some(target) = &target {
                open_link(shell, target.clone(), window, cx);
            }
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(link_name(ix, &title, cx)),
        )
        // Fixed slots keep the columns aligned from row to row: the current row leaves the
        // Roll back slot empty.
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
                // The current row has the label and every other row the Diff button, so the
                // two never meet in this slot.
                .children(if revision.is_current {
                    Some(
                        toned_text(
                            StatusLabel {
                                text: "current".into(),
                                tone: current_tone,
                            },
                            cx,
                        )
                        .into_any_element(),
                    )
                } else {
                    diff.map(|request| {
                        Button::new(("revision-diff", ix))
                            .label("Diff")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(move |shell, _, window, cx| {
                                // The row behind the button reveals its ReplicaSet on a click.
                                cx.stop_propagation();
                                shell.open_revision_diff(request.clone(), window, cx);
                            }))
                            .into_any_element()
                    })
                }),
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
                    let roll_back = Button::new(("roll-back", ix))
                        .label("Roll back")
                        .xsmall()
                        .ghost();
                    match button {
                        RollBackButton::Disabled(reason) => {
                            roll_back.disabled(true).tooltip(reason)
                        }
                        RollBackButton::Enabled(subject, target) => {
                            roll_back.on_click(cx.listener(move |shell, _, window, cx| {
                                // The row behind the button reveals its ReplicaSet on a click.
                                cx.stop_propagation();
                                shell.start_roll_back(&subject, &target, window, cx);
                            }))
                        }
                    }
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
        Some(
            RelatedList::ReplicaSets(_)
            | RelatedList::ConfigMapValues(_)
            | RelatedList::Events(_)
            | RelatedList::NamespaceLimits { .. }
            | RelatedList::HelmHistory(_)
            | RelatedList::CustomFields(_),
        )
        | None => None,
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
        .on_click(cx.listener(move |shell, _, window, cx| {
            if let Some(target) = &target {
                open_link(shell, target.clone(), window, cx);
            }
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(link_name(ix, &job.name, cx)),
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
        .on_click(cx.listener(move |shell, _, window, cx| {
            open_link(shell, key.clone(), window, cx);
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(link_name(ix, &title, cx)),
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
    /// `10.0.0.5:8080`; the port moves to a "Ports" field when there are several.
    address: String,
    state: EndpointState,
    /// The pod behind the endpoint, when it names one; the row draws its name as a link.
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
                EndpointRow {
                    address,
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
    let source = live.companion_source(kind);
    let Some(slices) = source
        .as_ref()
        .and_then(|source| source.lists().endpoint_slices())
    else {
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
    let mut cell = h_flex()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .font_family(theme.mono_font_family.clone())
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.muted_foreground)
                .child(row.address.clone()),
        );
    if let Some(target) = row.pod.clone() {
        let hover_bg = theme.muted;
        if let ResourceKey::Pod { name, .. } = &target {
            cell = cell
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child(" · "),
                )
                .child(link_name(ix, name, cx));
        }
        element = element
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .on_click(cx.listener(move |shell, _, window, cx| {
                open_link(shell, target.clone(), window, cx);
            }));
    }
    element
        .child(cell)
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
        Some(
            RelatedList::ReplicaSets(_)
            | RelatedList::Jobs(_)
            | RelatedList::Events(_)
            | RelatedList::NamespaceLimits { .. }
            | RelatedList::HelmHistory(_)
            | RelatedList::CustomFields(_),
        )
        | None => None,
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
        .chain(restart_hint(&users).map(|hint| note(&hint, cx)))
        .chain(std::iter::once(note(&format!("From pods in {scope}"), cx)))
        .collect()
}

/// How many owners the restart hint names before it counts the rest.
const HINT_LISTED_OWNERS: usize = 3;

/// The note under Used by when some user reads the ConfigMap through env: those values are read
/// once at container start. CronJob and Job owners are left out (each run starts fresh), and so are
/// bare `pod/{name}` owners (a pod without a controller is not restarted; it is replaced), and
/// orphan `replicaset/{name}` owners (a ReplicaSet without a Deployment is not restarted either). Pure.
fn restart_hint(users: &[&UsedBy]) -> Option<String> {
    let readers: Vec<&str> = users
        .iter()
        .filter(|used_by| used_by.ways.contains(WAY_ENV) || used_by.ways.contains(WAY_ENV_FROM))
        .map(|used_by| used_by.owner.as_str())
        .filter(|owner| {
            !["cronjob/", "job/", "pod/", "replicaset/"]
                .iter()
                .any(|prefix| owner.starts_with(prefix))
        })
        .collect();
    if readers.is_empty() {
        return None;
    }
    let listed = readers[..readers.len().min(HINT_LISTED_OWNERS)].join(", ");
    let more = match readers.len().saturating_sub(HINT_LISTED_OWNERS) {
        0 => String::new(),
        count => format!(" and {count} more"),
    };
    Some(format!(
        "Env values are read when a container starts: restart {listed}{more} to use a change. Mounted files update on their own (not with subPath)."
    ))
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

// ---- Custom objects ----

/// One side of a custom object, from the related fields watch of the open drawer.
fn custom_fields_section(
    side: FieldsSide,
    id_base: usize,
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let list = related_subject(kind, row)
        .and_then(|subject| live.related_of(&subject))
        .and_then(RelatedList::custom_fields);
    let rows = field_list_rows(list, side, row.namespace.as_deref());
    live_detail_rows(&rows, id_base, &DrawerPaint::new(kind, row, live, now), cx)
}

// ---- Secrets ----

/// Element ids of the Certificate section's rows start here, clear of the drawer's own.
const CERTIFICATE_ID_BASE: usize = 10_000;
/// The sections of a custom object; their element ids start here.
const CUSTOM_CONDITIONS_ID_BASE: usize = 30_000;
const CUSTOM_STATUS_ID_BASE: usize = 40_000;
const CUSTOM_SPEC_ID_BASE: usize = 50_000;
/// The TLS section of an Ingress; its element ids start here.
const INGRESS_TLS_ID_BASE: usize = 20_000;
const UNUSED_NOTE: &str = "No pod or ingress in this namespace uses it. Workloads with no running pod, CronJob templates, Gateway API and Istio references, and readers through the API are not checked.";
/// The first note when the ingresses could not be checked.
const UNUSED_NOTE_PODS_ONLY: &str = "No pod in this namespace uses it. Workloads with no running pod, CronJob templates, Gateway API and Istio references, and readers through the API are not checked.";

/// The masked keys of a Secret: name, the fixed mask, and the size. Used until the values view
/// takes the section over, and never with a value.
fn masked_rows(secret: &SecretSummary, cx: &Context<AppShell>) -> Vec<AnyElement> {
    secret_data_rows(&secret.keys)
        .iter()
        .enumerate()
        .map(|(ix, row)| masked_key_element(ix, row, cx))
        .collect()
}

fn masked_key_element(ix: usize, row: &MaskedKeyRow, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let size = if row.is_binary {
        format!("{} · binary", row.size)
    } else {
        row.size.clone()
    };
    h_flex()
        .id(("secret-key", ix))
        .gap_2()
        .items_center()
        .py_1()
        .text_sm()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(theme.mono_font_family.clone())
                .child(row.key.clone()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .font_family(theme.mono_font_family.clone())
                .child(MASK),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(size),
        )
        .into_any_element()
}

/// How far the Ingresses companion of the Secrets screen has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IngressesState {
    Loading,
    Ready,
    Denied,
    Unavailable,
}

fn ingresses_state(list: Option<&LiveList<IngressSummary>>, plan: CompanionPlan) -> IngressesState {
    match list {
        Some(list) if list.ready_items().is_some() => IngressesState::Ready,
        Some(list) if list.is_loading() => IngressesState::Loading,
        _ if matches!(plan, CompanionPlan::Denied(_)) => IngressesState::Denied,
        // Failed, or not started yet: the explorer starts it with the screen.
        _ => IngressesState::Unavailable,
    }
}

/// The notes of a Secret nobody uses, by what is known about the ingresses.
fn unused_notes(ingresses: IngressesState) -> Vec<&'static str> {
    match ingresses {
        IngressesState::Loading => vec!["Loading…"],
        IngressesState::Ready => vec![UNUSED_NOTE],
        IngressesState::Denied => vec![UNUSED_NOTE_PODS_ONLY, "Not permitted: list ingresses"],
        IngressesState::Unavailable => vec![UNUSED_NOTE_PODS_ONLY, "Ingresses are unavailable"],
    }
}

fn secret_used_by_rows(
    secret: &SecretSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    if live.pods.is_loading() {
        return vec![note("Loading…", cx)];
    }
    let Some(pods) = live.pods.ready_items() else {
        return vec![note("Pods are unavailable", cx)];
    };
    let source = live.companion_source(ResourceKind::Secrets);
    let list = source
        .as_ref()
        .and_then(|source| source.lists().ingresses());
    // Only the pods and ingresses of the Secret's namespace can use it.
    let users = secret_users(
        pods.iter().filter(|pod| pod.namespace == secret.namespace),
        list.and_then(LiveList::ready_items)
            .into_iter()
            .flatten()
            .filter(|ingress| ingress.namespace == secret.namespace),
    );
    let users = secret_user_list(secret, &users);
    if users.is_empty() {
        let plan = companion_plan(ResourceKind::Secrets, &live.access);
        return unused_notes(ingresses_state(list, plan))
            .into_iter()
            .map(|text| note(text, cx))
            .collect();
    }
    let hidden = users.len().saturating_sub(MAX_LISTED_USERS);
    users
        .iter()
        .take(MAX_LISTED_USERS)
        .enumerate()
        .map(|(ix, used_by)| used_by_element(ix, used_by, cx))
        .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
        .collect()
}

/// The TLS section of an Ingress: its entries, and what the TLS secrets companion says about the
/// secrets they name.
fn ingress_tls_section(
    ingress: &IngressSummary,
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let source = live.companion_source(kind);
    let list = source
        .as_ref()
        .and_then(|source| source.lists().tls_secrets());
    let secrets = list.and_then(LiveList::ready_items);
    let state = match (secrets, list) {
        (Some(secrets), _) => TlsSecrets::Ready(secrets),
        (None, Some(list)) if list.is_loading() => TlsSecrets::Loading,
        (None, _)
            if matches!(
                companion_plan(ResourceKind::Ingresses, &live.access),
                CompanionPlan::Denied(_)
            ) =>
        {
            TlsSecrets::Denied
        }
        (None, Some(_)) => TlsSecrets::Unavailable,
        // Not started yet: the explorer starts it with the screen.
        (None, None) => TlsSecrets::Loading,
    };
    let rows = ingress_tls_rows(ingress, &state);
    live_detail_rows(
        &rows,
        INGRESS_TLS_ID_BASE,
        &DrawerPaint::new(kind, row, live, now),
        cx,
    )
}

// ---- Selected pods ----

/// How many selected pods a PodDisruptionBudget drawer lists.
const MAX_LISTED_SELECTED_PODS: usize = 50;

/// A pod a budget selects, with its health as the PDB controller reads it (the Ready condition).
#[derive(Debug, PartialEq, Eq)]
struct SelectedPod<'a> {
    pod: &'a PodSummary,
    is_healthy: bool,
}

/// The pods of the budget's namespace that its selector matches, unhealthy first, then by name.
/// A budget without a selector selects none.
fn selected_pods<'a>(
    budget: &PodDisruptionBudgetSummary,
    pods: &'a [PodSummary],
) -> Vec<SelectedPod<'a>> {
    let Some(selector) = &budget.selector else {
        return Vec::new();
    };
    let mut selected: Vec<SelectedPod<'a>> = pods
        .iter()
        .filter(|pod| pod.namespace == budget.namespace && selector.matches(&pod.labels))
        .map(|pod| SelectedPod {
            pod,
            is_healthy: pod
                .conditions
                .iter()
                .any(|condition| condition.name == "Ready" && condition.is_true),
        })
        .collect();
    selected.sort_by(|a, b| {
        a.is_healthy
            .cmp(&b.is_healthy)
            .then_with(|| a.pod.name.cmp(&b.pod.name))
    });
    selected
}

fn selected_pods_rows(
    budget: &PodDisruptionBudgetSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    if live.pods.is_loading() {
        return vec![note("Loading pods…", cx)];
    }
    let Some(pods) = live.pods.ready_items() else {
        return vec![note("Pods are unavailable", cx)];
    };
    let selected = selected_pods(budget, pods);
    if selected.is_empty() {
        return vec![note("No pods match", cx)];
    }
    let healthy = selected.iter().filter(|entry| entry.is_healthy).count();
    let hidden = selected.len().saturating_sub(MAX_LISTED_SELECTED_PODS);
    let noun = if selected.len() == 1 { "pod" } else { "pods" };
    std::iter::once(note(
        &format!("{} {noun} · {healthy} healthy", selected.len()),
        cx,
    ))
    .chain(
        selected
            .iter()
            .take(MAX_LISTED_SELECTED_PODS)
            .enumerate()
            .map(|(ix, entry)| selected_pod_element(ix, entry, cx)),
    )
    .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
    .collect()
}

fn selected_pod_element(ix: usize, entry: &SelectedPod, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let hover_bg = theme.muted;
    let key = ResourceKey::of_pod(entry.pod);
    let (text, tone) = if entry.is_healthy {
        ("healthy", StatusTone::Ok)
    } else {
        ("unhealthy", StatusTone::Bad)
    };
    h_flex()
        .id(("selected-pod", ix))
        .gap_2()
        .items_center()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, window, cx| {
            open_link(shell, key.clone(), window, cx);
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(link_name(ix, &entry.pod.name, cx)),
        )
        .child(
            toned_text(
                StatusLabel {
                    text: text.into(),
                    tone,
                },
                cx,
            )
            .flex_shrink_0(),
        )
        .into_any_element()
}

// ---- Claim usage ----

/// The Used and Inodes bars of a claim, then when the kubelet sampled them; or one note that
/// says why there are none. Pure: `Bar` and `Note` rows are what the painter turns into elements.
fn claim_usage_rows(
    claim: &PersistentVolumeClaimSummary,
    usage: Option<&PvcUsage>,
    feed: &FeedStatus,
    now: jiff::Timestamp,
) -> Vec<DetailRow> {
    let note = |text: String| vec![DetailRow::Note(text.into())];
    if claim.volume_mode.as_deref() == Some("Block") {
        return note("Block volumes report no usage".to_owned());
    }
    if claim.phase != "Bound" {
        return note("No usage until the claim is bound".to_owned());
    }
    let Some((usage, sample)) = usage.and_then(|usage| Some((usage, claim_sample(usage)?))) else {
        return note(match feed {
            FeedStatus::Unavailable(reason) => format!("Usage unavailable: {reason}"),
            _ => "No usage data yet: kubelet stats appear once a running pod mounts the claim"
                .to_owned(),
        });
    };
    let UsageSample {
        used,
        capacity,
        ratio,
    } = sample;
    let is_shared = is_shared_filesystem(usage, claim.capacity.as_deref());
    let mut rows = vec![DetailRow::Bar {
        label: if is_shared { "Node filesystem" } else { "Used" }.into(),
        percent: percent(ratio),
        text: Measure::Bytes
            .format_pair(used.bytes() as f64, capacity.bytes() as f64, " of ")
            .into(),
        tone: usage_tone(ratio),
    }];
    if let (Some(inodes_used), Some(inodes)) = (usage.inodes_used, usage.inodes)
        && inodes > 0
    {
        let inode_ratio = inodes_used as f64 / inodes as f64;
        rows.push(DetailRow::Bar {
            label: "Inodes".into(),
            percent: percent(inode_ratio),
            text: format_percent(inode_ratio).into(),
            tone: usage_tone(inode_ratio),
        });
    }
    if is_shared {
        rows.push(DetailRow::Note(
            "Shared with the node: the claim has no quota of its own".into(),
        ));
    }
    if let Some(sampled_at) = usage.sampled_at {
        rows.push(DetailRow::Note(
            format!("Sampled {} ago", format_age(Some(sampled_at), now)).into(),
        ));
    }
    rows
}

fn claim_usage_content(
    claim: &PersistentVolumeClaimSummary,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let kubelet = &live.metrics.kubelet;
    let usage = kubelet.history.pvc_usage(&claim.namespace, &claim.name);
    claim_usage_rows(claim, usage, &kubelet.status, now)
        .into_iter()
        .enumerate()
        .map(|(ix, row)| match row {
            DetailRow::Bar {
                label,
                percent,
                text,
                tone,
            } => bar_row(&label, percent, &text, tone, ix, cx),
            DetailRow::Note(text) => note(&text, cx),
            _ => div().into_any_element(),
        })
        .collect()
}

// ---- Mounted by ----

/// How many pods a PVC drawer lists.
const MAX_LISTED_MOUNTS: usize = 50;

/// The pods of `namespace` that mount `claim` in any container, by name, one entry per pod with
/// the first mount path. Block-mode claims are `volumeDevices`, which pod summaries do not keep.
pub(crate) fn claim_pods<'a>(
    namespace: &str,
    claim: &str,
    pods: &'a [PodSummary],
) -> Vec<(&'a PodSummary, &'a str)> {
    let mut mounting: Vec<(&PodSummary, &str)> = pods
        .iter()
        .filter(|pod| pod.namespace == namespace)
        .filter_map(|pod| {
            let mount = pod
                .containers
                .iter()
                .flat_map(|container| &container.mounts)
                .find(|mount| {
                    matches!(&mount.source,
                        VolumeSource::PersistentVolumeClaim { claim: name } if name == claim)
                })?;
            Some((pod, mount.path.as_str()))
        })
        .collect();
    mounting.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    mounting
}

fn mounted_by_rows(
    claim: &PersistentVolumeClaimSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    if live.pods.is_loading() {
        return vec![note("Loading pods…", cx)];
    }
    let Some(pods) = live.pods.ready_items() else {
        return vec![note("Pods are unavailable", cx)];
    };
    let mounting = claim_pods(&claim.namespace, &claim.name, pods);
    if mounting.is_empty() {
        let is_block = claim.volume_mode.as_deref() == Some("Block");
        return std::iter::once(note("Not mounted by any pod", cx))
            .chain(is_block.then(|| note("Block volumes are not listed", cx)))
            .collect();
    }
    let hidden = mounting.len().saturating_sub(MAX_LISTED_MOUNTS);
    mounting
        .iter()
        .take(MAX_LISTED_MOUNTS)
        .enumerate()
        .map(|(ix, (pod, path))| mounting_pod_element(ix, pod, path, cx))
        .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
        .collect()
}

fn mounting_pod_element(
    ix: usize,
    pod: &PodSummary,
    path: &str,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let hover_bg = theme.muted;
    let key = ResourceKey::of_pod(pod);
    let node = pod.node_name.as_deref().unwrap_or("unscheduled");
    h_flex()
        .id(("mounted-by", ix))
        .gap_2()
        .items_center()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, window, cx| {
            open_link(shell, key.clone(), window, cx);
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(link_name(ix, &pod.name, cx)),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.muted_foreground)
                .child(format!("on {node} · {path}")),
        )
        .into_any_element()
}

// ---- Class volumes ----

/// How many persistent volumes a StorageClass drawer lists.
const MAX_LISTED_VOLUMES: usize = 20;

/// The volumes of `class`, in list order.
fn class_volumes<'a>(
    class: &str,
    volumes: &'a [PersistentVolumeSummary],
) -> Vec<&'a PersistentVolumeSummary> {
    volumes
        .iter()
        .filter(|volume| volume.storage_class.as_deref() == Some(class))
        .collect()
}

/// `12 · 10 bound · 2 released`; the bound and released parts are skipped when zero.
fn class_volumes_text(volumes: &[&PersistentVolumeSummary]) -> String {
    let count = |phase: &str| {
        volumes
            .iter()
            .filter(|volume| volume.phase == phase)
            .count()
    };
    let mut parts = vec![volumes.len().to_string()];
    for (phase, label) in [("Bound", "bound"), ("Released", "released")] {
        let phase_count = count(phase);
        if phase_count > 0 {
            parts.push(format!("{phase_count} {label}"));
        }
    }
    parts.join(" · ")
}

fn class_volumes_rows(
    kind: ResourceKind,
    class: &str,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let source = live.companion_source(kind);
    let Some(volumes) = source
        .as_ref()
        .and_then(|source| source.lists().persistent_volumes())
    else {
        // Without a companion the report denied it, or the explorer has not started it yet.
        if let CompanionPlan::Denied(check) = companion_plan(kind, &live.access) {
            return vec![note(&format!("Not permitted: {check}"), cx)];
        }
        return vec![note("Loading volumes…", cx)];
    };
    match volumes {
        LiveList::Loading => vec![note("Loading volumes…", cx)],
        LiveList::Failed { message } => vec![
            note("Volumes are unavailable", cx),
            detail_note(message, cx),
        ],
        LiveList::Ready { items, .. } => {
            let own = class_volumes(class, items);
            let hidden = own.len().saturating_sub(MAX_LISTED_VOLUMES);
            std::iter::once(
                wide_detail_row(
                    "Persistent volumes",
                    div().truncate().child(class_volumes_text(&own)),
                    cx,
                )
                .into_any_element(),
            )
            .chain(
                own.iter()
                    .take(MAX_LISTED_VOLUMES)
                    .enumerate()
                    .map(|(ix, volume)| class_volume_element(ix, volume, cx)),
            )
            .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
            .collect()
        }
    }
}

/// A volume as a link to its drawer, with its phase.
fn class_volume_element(
    ix: usize,
    volume: &PersistentVolumeSummary,
    cx: &Context<AppShell>,
) -> AnyElement {
    let name = volume.name.clone().into();
    let target = ResourceKey::Kind {
        kind: ResourceKind::PersistentVolumes,
        namespace: None,
        name: volume.name.clone(),
    };
    let link = link_text(ix, &name, target, cx);
    h_flex()
        .id(("class-volume", ix))
        .gap_2()
        .items_center()
        .py_1()
        .text_sm()
        .child(div().flex_1().min_w_0().child(link))
        .child(toned_text(phase_label(&volume.phase, volume.is_terminating), cx).flex_shrink_0())
        .into_any_element()
}

// ---- Scaling events ----

/// How many scaling events an HPA drawer lists.
const MAX_SCALING_EVENTS: usize = 10;
/// How many blocked creations a ResourceQuota drawer lists.
const MAX_BLOCKED_CREATIONS: usize = 20;
/// Characters of a scaling event message, and of a blocked creation message.
const SCALING_MESSAGE_CHARS: usize = 120;
const REJECTION_MESSAGE_CHARS: usize = 160;
const RESCALE_REASON: &str = "SuccessfulRescale";

/// `text` cut at `limit` characters with an ellipsis.
fn cut_text(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit).collect();
    cut.push('…');
    cut
}

/// The rescale events, newest first, at most `MAX_SCALING_EVENTS`.
fn scaling_events(events: &[EventSummary]) -> Vec<&EventSummary> {
    let mut rescales: Vec<&EventSummary> = events
        .iter()
        .filter(|event| event.reason == RESCALE_REASON)
        .collect();
    rescales.sort_by_key(|event| std::cmp::Reverse(event.last_seen));
    rescales.truncate(MAX_SCALING_EVENTS);
    rescales
}

/// `10:45 UTC` for an event of today's date in `zone`, else `Oct 6 02:30 UTC`; `—` without a time.
fn event_time_label(at: Option<jiff::Timestamp>, now: jiff::Timestamp, zone: &TimeZone) -> String {
    match at {
        Some(at) => run_label(&at.to_zoned(zone.clone()), now),
        None => "—".to_owned(),
    }
}

fn scaling_events_rows(
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let events =
        event_subject(&ResourceKey::of_row(kind, row)).and_then(|subject| live.events_of(&subject));
    match events {
        None | Some(LiveList::Loading) => vec![note("Loading events…", cx)],
        Some(LiveList::Failed { .. }) => vec![note("Events are unavailable", cx)],
        Some(LiveList::Ready { items, .. }) => {
            let rescales = scaling_events(items);
            if rescales.is_empty() {
                return vec![note("No scaling events kept", cx)];
            }
            let zone = TimeZone::system();
            rescales
                .into_iter()
                .map(|event| {
                    let message = event.message.lines().next().unwrap_or_default();
                    wide_detail_row(
                        event_time_label(event.last_seen, now, &zone),
                        div()
                            .truncate()
                            .child(cut_text(message, SCALING_MESSAGE_CHARS)),
                        cx,
                    )
                    .into_any_element()
                })
                .collect()
        }
    }
}

/// Ids for the links that live content paints, high so they never meet the drawer's own link ids.
const LIVE_LINK_ID_BASE: usize = 10_000;

// ---- Blocked creations ----

/// The two phrases of the API server's admission that name a quota, built once per quota: `exceeded
/// quota: {quota}` followed by `,` or the end, or `failed quota: {quota}:`. The follow-up
/// character keeps `compute-quota` from matching `compute-quota-2`.
struct QuotaNeedles {
    exceeded: String,
    failed: String,
}

impl QuotaNeedles {
    fn of(quota: &str) -> Self {
        Self {
            exceeded: format!("exceeded quota: {quota}"),
            failed: format!("failed quota: {quota}:"),
        }
    }

    fn matches(&self, message: &str) -> bool {
        let is_exceeded = message.match_indices(&self.exceeded).any(|(start, _)| {
            let rest = &message[start + self.exceeded.len()..];
            rest.is_empty() || rest.starts_with(',')
        });
        is_exceeded || message.contains(&self.failed)
    }
}

/// The events that name `quota`, newest first, at most `MAX_BLOCKED_CREATIONS`.
fn blocked_creations<'a>(events: &'a [EventSummary], quota: &str) -> Vec<&'a EventSummary> {
    let needles = QuotaNeedles::of(quota);
    let mut blocked: Vec<&EventSummary> = events
        .iter()
        .filter(|event| needles.matches(&event.message))
        .collect();
    blocked.sort_by_key(|event| std::cmp::Reverse(event.last_seen));
    blocked.truncate(MAX_BLOCKED_CREATIONS);
    blocked
}

fn blocked_creations_rows(
    subject: &RelatedSubject,
    quota: &str,
    live: &LiveCluster,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    if let Some(check) = denied_related_check(subject, &live.access) {
        return vec![note(&format!("Not permitted: {check}"), cx)];
    }
    let list = live.related_of(subject).and_then(RelatedList::events);
    let mut rows = match list {
        None | Some(LiveList::Loading) => vec![note("Loading events…", cx)],
        Some(LiveList::Failed { message }) => {
            vec![note("Events are unavailable", cx), detail_note(message, cx)]
        }
        Some(LiveList::Ready { items, .. }) => {
            let blocked = blocked_creations(items, quota);
            if blocked.is_empty() {
                vec![note("No creations blocked recently", cx)]
            } else {
                let zone = TimeZone::system();
                blocked
                    .into_iter()
                    .enumerate()
                    .map(|(ix, event)| blocked_creation_element(ix, event, now, &zone, cx))
                    .collect()
            }
        }
    };
    rows.push(note(
        "From FailedCreate events of controllers that the API server still keeps; a pod created \
         directly is rejected without an event",
        cx,
    ));
    rows
}

fn blocked_creation_element(
    ix: usize,
    event: &EventSummary,
    now: jiff::Timestamp,
    zone: &TimeZone,
    cx: &Context<AppShell>,
) -> AnyElement {
    let label = event_time_label(event.last_seen, now, zone);
    let text = format!("{}/{}", event.object.kind.to_lowercase(), event.object.name);
    let target = ResourceKey::of_object(
        &event.object.kind,
        event.object.namespace.as_deref(),
        &event.object.name,
    );
    let object = match target {
        // The ids of live links start high, so they never meet the drawer's own link ids.
        Some(target) => wide_detail_row(
            label,
            link_text(LIVE_LINK_ID_BASE + ix, &text.into(), target, cx),
            cx,
        )
        .into_any_element(),
        None => wide_detail_row(label, div().truncate().child(text), cx).into_any_element(),
    };
    v_flex()
        .child(object)
        .child(note(&cut_text(&event.message, REJECTION_MESSAGE_CHARS), cx))
        .into_any_element()
}

// ---- Namespace quotas ----

/// `requests.cpu 3.1 / 4 cores (78%)` for the fullest item of a quota; the first item without
/// usage; `no limits` for none.
fn quota_summary_text(quota: &ResourceQuotaSummary) -> String {
    if let Some((item, ratio)) = fullest_item(quota) {
        return format!(
            "{} {} ({})",
            item.resource,
            quota_text(item),
            format_percent(ratio)
        );
    }
    match quota.items.first() {
        Some(item) => format!("{} {}", item.resource, quota_text(item)),
        None => "no limits".to_owned(),
    }
}

/// What one half of the Quota section says before it has rows to show.
#[derive(Debug, PartialEq, Eq)]
enum QuotaHalf<'a, T> {
    Note(String),
    Items(&'a [T]),
}

/// The words of one half's notes.
struct HalfWords {
    loading: &'static str,
    unavailable: &'static str,
    none: &'static str,
}

const QUOTA_WORDS: HalfWords = HalfWords {
    loading: "Loading quotas…",
    unavailable: "Quotas are unavailable",
    none: "No ResourceQuota",
};

const LIMIT_RANGE_WORDS: HalfWords = HalfWords {
    loading: "Loading limit ranges…",
    unavailable: "Limit ranges are unavailable",
    none: "No LimitRange",
};

/// One half of the Quota section: the denied check first (its watch never started), then the state
/// of its own list, independently of the other half. Pure.
fn quota_half<'a, T>(
    check: AccessCheck,
    is_open: bool,
    list: Option<&'a LiveList<T>>,
    words: &HalfWords,
) -> QuotaHalf<'a, T> {
    if !is_open {
        return QuotaHalf::Note(format!("Not permitted: {check}"));
    }
    match list {
        None | Some(LiveList::Loading) => QuotaHalf::Note(words.loading.to_owned()),
        Some(LiveList::Failed { .. }) => QuotaHalf::Note(words.unavailable.to_owned()),
        Some(LiveList::Ready { items, .. }) if items.is_empty() => {
            QuotaHalf::Note(words.none.to_owned())
        }
        Some(LiveList::Ready { items, .. }) => QuotaHalf::Items(items),
    }
}

/// `Container: default cpu 500m, memory 512Mi · request cpu 100m · max cpu 2`; items joined by
/// `; `, empty maps left out, a LimitRange without limits reads `no limits`. Pure.
fn limit_range_text(limit_range: &LimitRangeSummary) -> String {
    if limit_range.limits.is_empty() {
        return "no limits".to_owned();
    }
    limit_range
        .limits
        .iter()
        .map(limit_text)
        .collect::<Vec<_>>()
        .join("; ")
}

fn limit_text(limit: &LimitRangeLimit) -> String {
    let parts: Vec<String> = [
        ("default", &limit.default),
        ("request", &limit.default_request),
        ("max", &limit.max),
        ("min", &limit.min),
    ]
    .into_iter()
    .filter(|(_, quantities)| !quantities.is_empty())
    .map(|(label, quantities)| {
        let pairs: Vec<String> = quantities
            .iter()
            .map(|(resource, quantity)| format!("{resource} {quantity}"))
            .collect();
        format!("{label} {}", pairs.join(", "))
    })
    .collect();
    if parts.is_empty() {
        return limit.kind.clone();
    }
    format!("{}: {}", limit.kind, parts.join(" · "))
}

fn namespace_quota_rows(
    namespace: &str,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let subject = RelatedSubject::NamespaceQuotas {
        namespace: namespace.to_owned(),
    };
    let gates = live.namespace_gates();
    let lists = live
        .related_of(&subject)
        .and_then(RelatedList::namespace_limits);
    let quotas = quota_half(
        AccessCheck::ListResourceQuotas,
        gates.quotas,
        lists.map(|(quotas, _)| quotas),
        &QUOTA_WORDS,
    );
    let limit_ranges = quota_half(
        AccessCheck::ListLimitRanges,
        gates.limit_ranges,
        lists.map(|(_, limit_ranges)| limit_ranges),
        &LIMIT_RANGE_WORDS,
    );
    let quota_rows = match quotas {
        QuotaHalf::Note(text) => vec![note(&text, cx)],
        QuotaHalf::Items(quotas) => quotas
            .iter()
            .enumerate()
            .map(|(ix, quota)| {
                let text = quota_summary_text(quota);
                // A quota is a namespaced kind with a screen, so its key is built directly.
                let target = ResourceKey::Kind {
                    kind: ResourceKind::ResourceQuotas,
                    namespace: Some(namespace.to_owned()),
                    name: quota.name.clone(),
                };
                wide_detail_row(
                    quota.name.clone(),
                    link_text(LIVE_LINK_ID_BASE + ix, &text.into(), target, cx),
                    cx,
                )
                .into_any_element()
            })
            .collect(),
    };
    let limit_range_rows = match limit_ranges {
        QuotaHalf::Note(text) => vec![note(&text, cx)],
        // No link: k8sBoard has no LimitRange screen. The full text is the tooltip.
        QuotaHalf::Items(limit_ranges) => limit_ranges
            .iter()
            .enumerate()
            .map(|(ix, limit_range)| {
                wide_detail_row(
                    limit_range.name.clone(),
                    truncated_text(("limit-range", ix), limit_range_text(limit_range)),
                    cx,
                )
                .into_any_element()
            })
            .collect(),
    };
    quota_rows.into_iter().chain(limit_range_rows).collect()
}

// ---- Bindings ----

/// How many bindings or subjects an access-control drawer lists.
const MAX_LISTED_BINDINGS: usize = 50;

/// The bindings that name `role`, by name.
fn role_binding_list<'a>(index: &BindingIndex<'a>, role: &RoleSummary) -> Vec<&'a BindingSummary> {
    let mut bindings = index.bindings_of_role(role);
    bindings.sort_by(|a, b| a.name.cmp(&b.name));
    bindings
}

/// The Warn "review" tag: a service account that holds a role granting everything.
fn needs_review(role: &RoleSummary, subject: &RoleSubject) -> bool {
    role.grants_everything() && subject.is_service_account
}

/// Runs `rows` once the Bindings companion of `kind` is ready; before that, one note says why not.
fn with_bindings(
    kind: ResourceKind,
    live: &LiveCluster,
    cx: &Context<AppShell>,
    rows: impl FnOnce(&BindingIndex) -> Vec<AnyElement>,
) -> Vec<AnyElement> {
    let source = live.companion_source(kind);
    match bindings_status(
        kind,
        &live.access,
        source.as_ref().map(CompanionSource::lists),
    ) {
        BindingsStatus::Ready(lists) => rows(&BindingIndex::build(&lists)),
        BindingsStatus::Loading => vec![note("Loading bindings…", cx)],
        BindingsStatus::Failed(message) => vec![
            note("Bindings are unavailable", cx),
            detail_note(message, cx),
        ],
        BindingsStatus::Denied(checks) => {
            let names: Vec<String> = checks.iter().map(ToString::to_string).collect();
            vec![note(&format!("Not permitted: {}", names.join(", ")), cx)]
        }
    }
}

fn role_bindings_rows(
    kind: ResourceKind,
    role: &RoleSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    with_bindings(kind, live, cx, |index| {
        let bindings = role_binding_list(index, role);
        if bindings.is_empty() {
            return vec![note("Not bound", cx)];
        }
        let hidden = bindings.len().saturating_sub(MAX_LISTED_BINDINGS);
        bindings
            .iter()
            .take(MAX_LISTED_BINDINGS)
            .enumerate()
            .map(|(ix, binding)| binding_element(ix, binding, cx))
            .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
            .collect()
    })
}

/// A binding as a link to its drawer, with who it binds.
fn binding_element(ix: usize, binding: &BindingSummary, cx: &Context<AppShell>) -> AnyElement {
    let subjects: Vec<String> = binding.subjects.iter().map(subject_text).collect();
    let link = link_text(ix, &binding_text(binding).into(), binding_key(binding), cx);
    // The subjects sit under the link: beside it, a long binding name would be cut to nothing.
    v_flex()
        .id(("role-binding", ix))
        .py_1()
        .text_sm()
        .child(link)
        .child(
            div()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(format!("→ {}", subjects.join(", "))),
        )
        .into_any_element()
}

fn role_subjects_rows(
    kind: ResourceKind,
    role: &RoleSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    with_bindings(kind, live, cx, |index| {
        let subjects = role_subjects(&index.bindings_of_role(role));
        if subjects.is_empty() {
            return vec![note("Not bound", cx)];
        }
        let hidden = subjects.len().saturating_sub(MAX_LISTED_BINDINGS);
        subjects
            .iter()
            .take(MAX_LISTED_BINDINGS)
            .enumerate()
            .map(|(ix, subject)| role_subject_element(ix, subject, needs_review(role, subject), cx))
            .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
            .chain(std::iter::once(note(
                &bindings_scope_note(&live.scope_label()),
                cx,
            )))
            .collect()
    })
}

/// The reminder under a binding list: RoleBindings come from the session scope only.
fn bindings_scope_note(scope: &str) -> String {
    format!("Role bindings from {scope}")
}

/// A subject of a ClusterRole, with the binding that gives it the role as a link under it.
fn role_subject_element(
    ix: usize,
    subject: &RoleSubject,
    is_review: bool,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let binding = binding_text(subject.binding).into();
    let link = link_text(ix, &binding, binding_key(subject.binding), cx);
    v_flex()
        .id(("role-subject", ix))
        .py_1()
        .text_sm()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(theme.mono_font_family.clone())
                        .child(subject.text.clone()),
                )
                .children(is_review.then(|| {
                    toned_text(
                        StatusLabel {
                            text: "review".into(),
                            tone: StatusTone::Warn,
                        },
                        cx,
                    )
                    .flex_shrink_0()
                })),
        )
        .child(
            h_flex()
                .gap_1()
                .text_color(theme.muted_foreground)
                .child(div().flex_shrink_0().child("via"))
                .child(div().min_w_0().child(link)),
        )
        .into_any_element()
}

// ---- Service accounts ----

fn bound_roles_rows(
    kind: ResourceKind,
    account: &ServiceAccountSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    with_bindings(kind, live, cx, |index| {
        let roles = index.roles_held(&account.namespace, &account.name);
        if roles.is_empty() {
            return vec![note("No roles bound", cx)];
        }
        let hidden = roles.len().saturating_sub(MAX_LISTED_BINDINGS);
        roles
            .iter()
            .take(MAX_LISTED_BINDINGS)
            .enumerate()
            .map(|(ix, bound)| bound_role_element(ix, bound, cx))
            .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
            .chain(std::iter::once(note(
                &bindings_scope_note(&live.scope_label()),
                cx,
            )))
            .collect()
    })
}

/// What the Can do section shows for one account.
enum CanDoContent {
    /// The snapshot is not ready: one muted line.
    Line(String),
    Ready(Rc<CanDoSummary>),
}

struct CanDoSummary {
    chips: CanDoChips,
    /// Warn lines for what the snapshot could not list.
    warnings: Vec<SharedString>,
}

/// The chips of the last account painted, kept because the drawer repaints often and the
/// evaluation walks every binding. The cache holds the snapshot it was computed from, so a
/// refreshed snapshot (a different `Rc`) always recomputes and an address is never reused.
pub(crate) struct CanDoCache {
    snapshot: Rc<RbacSnapshot>,
    namespace: String,
    name: String,
    summary: Rc<CanDoSummary>,
}

pub(crate) type CanDoCell = RefCell<Option<CanDoCache>>;

/// The chips of what `namespace/name` can do in its own namespace and cluster-wide. Grants that
/// reach it only through `system:authenticated` are left out: the basic-user review grants sit on
/// every account and would drown the real ones (the Check permissions dialog keeps them).
fn can_do_content(
    state: &RbacState,
    namespace: &str,
    name: &str,
    cache: &CanDoCell,
) -> CanDoContent {
    let (snapshot, _) = match state {
        RbacState::Idle | RbacState::Loading { .. } => {
            return CanDoContent::Line("Listing RBAC objects…".to_owned());
        }
        RbacState::Failed(message) => {
            return CanDoContent::Line(format!("RBAC objects are unavailable: {message}"));
        }
        RbacState::Ready {
            snapshot,
            listed_at,
        } => (snapshot, listed_at),
    };
    let cached = cache.borrow().as_ref().and_then(|cached| {
        let is_current = Rc::ptr_eq(&cached.snapshot, snapshot)
            && cached.namespace == namespace
            && cached.name == name;
        is_current.then(|| Rc::clone(&cached.summary))
    });
    if let Some(summary) = cached {
        return CanDoContent::Ready(summary);
    }
    let identity = Identity::service_account(namespace, name);
    let rules = snapshot.rules_of(&identity, Some(namespace));
    let own = rules
        .iter()
        .filter(|effective| !is_authenticated_group(effective.subject))
        .map(|effective| effective.rule);
    let summary = Rc::new(CanDoSummary {
        chips: can_do_chips(&permission_table(own)),
        warnings: coverage_notes(&snapshot.coverage, Some(namespace)),
    });
    *cache.borrow_mut() = Some(CanDoCache {
        snapshot: Rc::clone(snapshot),
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        summary: Rc::clone(&summary),
    });
    CanDoContent::Ready(summary)
}

fn is_authenticated_group(subject: &Subject) -> bool {
    subject.kind == SubjectKind::Group
        && matches!(subject.broad_group(), Some(BroadGroup::Authenticated))
}

fn can_do_rows(
    account: &ServiceAccountSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    match can_do_content(&live.rbac, &account.namespace, &account.name, &live.can_do) {
        CanDoContent::Line(text) => vec![note(&text, cx)],
        CanDoContent::Ready(summary) => {
            let CanDoSummary { chips, warnings } = &*summary;
            let mut rows = Vec::new();
            if chips.chips.is_empty() {
                rows.push(note("No permissions from RBAC bindings", cx));
            } else {
                let more = (chips.more > 0).then(|| (format!("+{} more", chips.more).into(), None));
                rows.push(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .children(
                            chips
                                .chips
                                .iter()
                                .cloned()
                                .chain(more)
                                .map(|(text, tone)| can_do_chip(text, tone, cx)),
                        )
                        .into_any_element(),
                );
            }
            rows.extend(warnings.iter().map(|warning| {
                div()
                    .text_sm()
                    .text_color(tone_color(StatusTone::Warn, cx))
                    .child(warning.clone())
                    .into_any_element()
            }));
            rows.push(note(
                &format!(
                    "In {} and cluster-wide · computed from RBAC objects. Check permissions shows the full table.",
                    account.namespace
                ),
                cx,
            ));
            rows
        }
    }
}

fn can_do_chip(
    text: gpui_kit::SharedString,
    tone: Option<StatusTone>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    div()
        .max_w_full()
        .truncate()
        .px_1p5()
        .rounded(theme.radius)
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .when_some(tone, |chip, tone| chip.text_color(tone_color(tone, cx)))
        .child(text)
        .into_any_element()
}

/// ` · group system:serviceaccounts` after the binding, when the account is reached through a
/// group.
fn group_suffix(bound: &BoundRole) -> Option<String> {
    bound
        .group
        .as_ref()
        .map(|group| format!(" · group {group}"))
}

/// A role as a link to its drawer (plain text for a kind without a screen), with the binding that
/// gives it under it.
fn bound_role_element(ix: usize, bound: &BoundRole, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let text: gpui_kit::SharedString = role_text(&bound.role).into();
    let role = match bound.role_key.clone() {
        Some(target) => link_text(ix, &text, target, cx),
        None => div()
            .truncate()
            .font_family(theme.mono_font_family.clone())
            .child(text)
            .into_any_element(),
    };
    // The role links use ids 0.., the binding links the ids after them.
    let binding = link_text(
        MAX_LISTED_BINDINGS + ix,
        &bound.binding_text.clone().into(),
        bound.binding.clone(),
        cx,
    );
    v_flex()
        .id(("bound-role", ix))
        .py_1()
        .text_sm()
        .child(role)
        .child(
            h_flex()
                .gap_1()
                .text_color(theme.muted_foreground)
                .child(div().flex_shrink_0().child("via"))
                .child(div().min_w_0().child(binding))
                .children(group_suffix(bound).map(|suffix| div().flex_shrink_0().child(suffix))),
        )
        .into_any_element()
}

/// The pods of the account namespace that run as the account, by name.
fn account_pods<'a>(
    account: &ServiceAccountSummary,
    pods: &'a [PodSummary],
) -> Vec<&'a PodSummary> {
    let mut using: Vec<&PodSummary> = pods
        .iter()
        .filter(|pod| pod.namespace == account.namespace && pod_account(pod) == account.name)
        .collect();
    using.sort_by(|a, b| a.name.cmp(&b.name));
    using
}

fn account_pods_rows(
    account: &ServiceAccountSummary,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    if live.pods.is_loading() {
        return vec![note("Loading pods…", cx)];
    }
    let Some(pods) = live.pods.ready_items() else {
        return vec![note("Pods are unavailable", cx)];
    };
    let using = account_pods(account, pods);
    if using.is_empty() {
        return vec![note("Not used by any pod", cx)];
    }
    let hidden = using.len().saturating_sub(MAX_LISTED_BINDINGS);
    using
        .iter()
        .take(MAX_LISTED_BINDINGS)
        .enumerate()
        .map(|(ix, pod)| {
            let name: gpui_kit::SharedString = pod.name.clone().into();
            h_flex()
                .id(("account-pod", ix))
                .gap_2()
                .items_center()
                .py_1()
                .text_sm()
                .child(div().flex_1().min_w_0().child(link_text(
                    ix,
                    &name,
                    ResourceKey::of_pod(pod),
                    cx,
                )))
                .child(toned_text(pod_status_label(pod), cx).flex_shrink_0())
                .into_any_element()
        })
        .chain((hidden > 0).then(|| note(&format!("+{hidden} more"), cx)))
        .collect()
}

// ---- Helm history ----

const HISTORY_STATUS_SLOT: Pixels = px(110.);

/// The revisions of a release, newest first: the revision, its status, when it was stored, and
/// the Values and Diff buttons. `shown_revision` is the one the Helm tabs show, when the History
/// chose one.
pub(crate) fn helm_history_rows(
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    shown_revision: Option<u32>,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let list = related_subject(kind, row)
        .and_then(|subject| live.related_of(&subject))
        .and_then(RelatedList::helm_history);
    match history_model(list) {
        HistoryModel::Note(text) => vec![note(&text, cx)],
        HistoryModel::Rows { rows, omitted } => rows
            .iter()
            .enumerate()
            .map(|(ix, revision)| {
                let key = ResourceKey::of_row(kind, row);
                let is_shown = shown_revision == Some(revision.revision);
                history_element(ix, revision, key, is_shown, now, cx)
            })
            .chain(
                (omitted > 0).then(|| note(&format!("{omitted} older revisions not shown."), cx)),
            )
            .collect(),
    }
}

fn history_element(
    ix: usize,
    revision: &HistoryRow,
    key: ResourceKey,
    is_shown: bool,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let age = match revision.updated_at {
        Some(_) => format!("{} ago", format_age(revision.updated_at, now)),
        None => "—".to_owned(),
    };
    let number = revision.revision;
    let values_key = key.clone();
    let title = if is_shown {
        format!("rev {number} · shown")
    } else {
        format!("rev {number}")
    };
    h_flex()
        .id(("helm-revision", ix))
        .gap_2()
        .items_center()
        .py_1()
        .text_sm()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(theme.mono_font_family.clone())
                .when(is_shown, |title| title.text_color(theme.muted_foreground))
                .child(title),
        )
        .child(
            toned_text(revision.status.clone(), cx)
                .w(HISTORY_STATUS_SLOT)
                .flex_shrink_0()
                .truncate(),
        )
        .child(
            div()
                .w(AGE_SLOT)
                .flex_shrink_0()
                .text_right()
                .text_color(theme.muted_foreground)
                .child(age),
        )
        .child(
            Button::new(("helm-values", ix))
                .label("Values")
                .ghost()
                .xsmall()
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.open_helm_values(values_key.clone(), number, ValuesLayout::Document, cx);
                })),
        )
        // The oldest revision has nothing to compare with; an empty slot keeps the columns aligned.
        .child(
            div()
                .w(DIFF_BUTTON_SLOT)
                .flex_shrink_0()
                .children(revision.can_diff.then(|| {
                    Button::new(("helm-diff", ix))
                        .label("Diff")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.open_helm_values(key.clone(), number, ValuesLayout::Diff, cx);
                        }))
                })),
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
