//! The overview drawer shared by the explorer kinds. The sections come from the row
//! builders; this module only renders them.

use std::rc::Rc;

use cluster::{EventSummary, HelmRevisionRef, ObjectKind, StorageClassSummary};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    div,
};

use crate::access_bindings::{BindingIndex, ready_binding_lists};
use crate::age::{format_age, format_local_time};
use crate::app_shell::AppShell;
use crate::batch_rows::cron_state_at;
use crate::certificate_expiry::expiry_detail_label;
use crate::clipboard_copy::copyable_mono;
use crate::cluster_registry::ClusterRef;
use crate::cluster_session::{CompanionLists, CompanionSource, LiveCluster, RelatedList};
use crate::custom_rows::{date_text, date_tone};
use crate::drawer::{
    DrawerBody, DrawerChrome, DrawerHeader, DrawerSize, DrawerState, DrawerTab, TabCounts,
    absent_text, annotations_section, chips, created_text, drawer_frame, drawer_tab_bar,
    drawer_tabs, first_section_title, helm_body, link_text, menu_button, open_link, port_row,
    section_title, shown_tab, tab_titles, truncated_text, truncated_text_with_tooltip,
    wide_detail_row, yaml_body,
};
use crate::helm_release_view::HelmReleaseView;
use crate::helm_rows::VALUES_CHANGE_TITLE;
use crate::ingress_backends::IngressBackends;
use crate::kind_diagnosis::{
    DiagnosisInputs, KindDiagnosis, kind_diagnosis, missing_storage_class, rollout_progress,
};
use crate::kind_join::{matching_pods, service_health_of};
use crate::kind_row::{DetailRow, KindCell, KindObject, KindRow, LiveContent};
use crate::live_sections::{
    DrawerWriteGate, all_pods_ready, helm_history_rows, live_rows, loaded_replica_sets,
    next_run_text, owned_pods,
};
use crate::monitor_tab::{MonitorView, monitor_tab};
use crate::object_events::{event_subject, recent_events};
use crate::port_forward_menu::{
    ForwardMenu, ForwardSubject, PortButton, PortButtons, PortChoice, row_subject,
};
use crate::related_objects::RelatedSubject;
use crate::related_pods::pods_section;
use crate::resource_actions::{
    MenuCluster, MenuExtras, OpenUrl, ResourceAction, action_availability, browse_instances_item,
    kind_menu, open_url_choice, open_url_menu_item, secret_menu,
};
use crate::resource_kind::ResourceKind;
use crate::row_context::RowContext;
use crate::secret_values::{SecretValuesView, ValueAccess};
use crate::status_tone::{StatusLabel, StatusTone, tone_color, toned_text};
use crate::table_selection::{ClusterObject, ResourceKey};

pub(crate) fn kind_drawer(
    kind: ResourceKind,
    row: &KindRow,
    state: &DrawerState,
    live: &LiveCluster,
    chrome: DrawerChrome<'_>,
    context: &RowContext,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = jiff::Timestamp::now();
    let DrawerChrome {
        forward,
        navigation,
    } = chrome;
    let header = DrawerHeader {
        kind_icon: kind.icon(),
        kind_name: kind.display_name().into(),
        name: header_name(row),
        subtitle: subtitle(row, now, cx),
        menu: kind_menu_button(kind, row, context, cx.weak_entity()),
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
        navigation,
    };
    let key = ResourceKey::of_row(kind, row);
    let events = event_subject(&key).and_then(|subject| live.events_of(&subject));
    let tabs = drawer_tabs(&key);
    let shown = shown_tab(tabs, state.tab);
    let body = match shown {
        DrawerTab::Events => DrawerBody::Scrolling(recent_events(events, cx)),
        DrawerTab::Monitor => DrawerBody::Scrolling(match row.related_pods {
            Some(_) => monitor_tab(&MonitorView::of_pods(state, live), cx),
            None => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No pods to monitor")
                .into_any_element(),
        }),
        DrawerTab::Yaml => yaml_body(state),
        DrawerTab::Values | DrawerTab::Manifest | DrawerTab::Notes => helm_body(state),
        DrawerTab::Overview | DrawerTab::Containers | DrawerTab::Pods => {
            // The Roll back buttons are gated by the cluster of the drawer's own subject.
            let roll_back = context.session.upgrade().and_then(|session| {
                let session = session.read(cx);
                session.guard(cx).map(|guard| DrawerWriteGate {
                    subject: ClusterObject::new(context.cluster.clone(), key.clone()),
                    availability: action_availability(ResourceAction::RollBack, &guard),
                    hpa_range: action_availability(ResourceAction::EditHpaRange, &guard),
                    restart: [
                        ObjectKind::Deployment,
                        ObjectKind::StatefulSet,
                        ObjectKind::DaemonSet,
                    ]
                    .map(|kind| {
                        let action = ResourceAction::RestartRollout(kind);
                        (kind, action_availability(action, &guard))
                    })
                    .to_vec(),
                })
            });
            let paint = DrawerPaint::new(kind, row, live, now)
                .in_cluster(&context.cluster)
                .with_roll_back(roll_back)
                .with_annotations_open(state.are_annotations_open)
                .with_secret_values(state.secret_values.as_ref())
                .with_helm(state.helm.as_ref(), state.helm_revision)
                .with_ports(forward);
            let Overview {
                sections,
                section_starts,
            } = overview(&paint, cx);
            // A menu item asked to see a section, once: the scroll handle moves the box on the
            // paint this frame ends with. A section the row lacks leaves the scroll as it is.
            let wanted = state.reveal_section.take();
            let start = wanted.and_then(|title| {
                section_starts
                    .iter()
                    .find_map(|&(found, at)| (found == title).then_some(at))
            });
            if let Some(at) = start {
                state.scroll.scroll_to_top_of_item(at);
            }
            DrawerBody::Sections {
                sections,
                scroll: state.scroll.clone(),
            }
        }
    };
    let tab_bar = drawer_tab_bar(tab_titles(tabs, TabCounts::default(), events), shown, cx);
    drawer_frame(
        header,
        tab_bar,
        body,
        state.width(DrawerSize::Standard),
        &state.scroll,
        cx,
    )
    .into_any_element()
}

/// The event title for events, else the object name.
fn header_name(row: &KindRow) -> SharedString {
    match &row.event {
        Some(event) => event.title.clone(),
        None => row.name.clone().into(),
    }
}

/// The status, then `· namespace · created 2d ago`; events show their source instead of an age.
fn subtitle(row: &KindRow, now: jiff::Timestamp, cx: &App) -> AnyElement {
    let detail: Vec<String> = match &row.event {
        Some(event) => row
            .namespace
            .iter()
            .cloned()
            .chain(event.source.iter().map(ToString::to_string))
            .collect(),
        None => row
            .namespace
            .iter()
            .cloned()
            .chain(revision_text(row))
            .chain(created_text(row.created_at, now))
            .collect(),
    };
    h_flex()
        .gap_1()
        .text_sm()
        .child(toned_text(row.status.clone(), cx))
        .children((!detail.is_empty()).then(|| {
            div()
                .text_color(cx.theme().muted_foreground)
                .child(format!("· {}", detail.join(" · ")))
        }))
        .into_any_element()
}

/// `rev 7` for a Deployment that has a revision, shown after its namespace in the subtitle.
fn revision_text(row: &KindRow) -> Option<String> {
    match &row.object {
        KindObject::Deployment(deployment) => {
            Some(format!("rev {}", deployment.revision.as_deref()?))
        }
        KindObject::Plain
        | KindObject::CronJob(_)
        | KindObject::StatefulSet(_)
        | KindObject::DaemonSet(_)
        | KindObject::ReplicaSet(_)
        | KindObject::Job(_)
        | KindObject::Service(_)
        | KindObject::Ingress(_)
        | KindObject::ConfigMap(_)
        | KindObject::NetworkPolicy(_)
        | KindObject::PodDisruptionBudget(_)
        | KindObject::HorizontalPodAutoscaler(_)
        | KindObject::ResourceQuota(_)
        | KindObject::PersistentVolumeClaim(_)
        | KindObject::PersistentVolume(_)
        | KindObject::Role(_)
        | KindObject::Binding(_)
        | KindObject::ServiceAccount(_)
        | KindObject::Secret(_)
        | KindObject::HelmRelease(_)
        | KindObject::Crd(_)
        | KindObject::Custom(_)
        | KindObject::StorageClass(_)
        | KindObject::Namespace(_) => None,
    }
}

/// The menu reads the session when it opens, so it shows the access state and the row of
/// that moment.
fn kind_menu_button(
    kind: ResourceKind,
    row: &KindRow,
    context: &RowContext,
    shell: WeakEntity<AppShell>,
) -> AnyElement {
    // Weak: a rendered menu closure must not keep a session alive after a cluster switch.
    let session = context.session.clone();
    let context = context.clone();
    let key = ResourceKey::of_row(kind, row);
    menu_button()
        .dropdown_menu(move |menu, window, cx| {
            let Some(session) = session.upgrade() else {
                return menu;
            };
            let open_url = (kind == ResourceKind::Ingresses).then(|| {
                let choice = session
                    .read(cx)
                    .live()
                    .and_then(|live| live.row_of(&key))
                    .map_or(OpenUrl::Unavailable, open_url_choice);
                open_url_menu_item(choice, &context, window, cx)
            });
            let secret = (kind == ResourceKind::Secrets)
                .then(|| {
                    let row = session
                        .read(cx)
                        .live()
                        .and_then(|live| live.row_of(&key))
                        .cloned()?;
                    let access = shell
                        .read_with(cx, |shell, _| shell.secret_value_access())
                        .unwrap_or(ValueAccess::Blocked);
                    secret_menu(
                        &row,
                        &context,
                        context.object(key.clone()),
                        access,
                        &shell,
                        window,
                        cx,
                    )
                })
                .flatten();
            // The submenu is built from the app, so it is made before the session is borrowed.
            let forward_menu = {
                let session = session.read(cx);
                session.guard(cx).and_then(|guard| {
                    let subject = row_subject(session.live()?.row_of(&key)?)?;
                    Some(ForwardMenu::of(subject, &context.cluster, &guard))
                })
            };
            let port_forward = forward_menu.map(|menu| menu.item(&shell, window, cx));
            let default_namespace = shell
                .read_with(cx, |shell, cx| {
                    shell.default_namespace(&context.cluster, cx)
                })
                .ok()
                .flatten();
            let session = session.read(cx);
            let (Some(live), Some(guard)) = (session.live(), session.guard(cx)) else {
                return menu;
            };
            let current = live.row_of(&key);
            match current {
                Some(row) => kind_menu(
                    menu,
                    kind,
                    row,
                    &MenuCluster {
                        guard: &guard,
                        pods: live.pods.items(),
                        context: &context,
                        replica_sets: loaded_replica_sets(kind, row, live),
                    },
                    &shell,
                    MenuExtras {
                        open_url,
                        port_forward,
                        secret,
                        browse: browse_instances_item(row, live.crd_kinds(), &shell),
                        default_namespace,
                        scope: Some(live.scope.clone()),
                    },
                ),
                None => menu,
            }
        })
        .into_any_element()
}

/// The row's sections in order, then the related pods, then the labels.
fn overview(paint: &DrawerPaint, cx: &Context<AppShell>) -> Overview {
    let (kind, row, live, now) = (paint.kind, paint.row, paint.live, paint.now);
    // Gives every element that needs an id one that is unique inside the drawer.
    let mut next_id = 0_usize;
    let mut sections: Vec<AnyElement> = Vec::new();
    let mut section_starts = Vec::with_capacity(row.sections.len());
    if let Some(diagnosis) = row_diagnosis(kind, row, live, now) {
        sections.push(why_box(&diagnosis, kind, cx));
    }
    let missing_class = missing_claim_class(kind, row, live);
    for section in &row.sections {
        // An all-ready DaemonSet has nothing to list under "Not ready"; the bars already say so.
        if section.rows == [DetailRow::Live(LiveContent::NotReadyPods)] && all_pods_ready(row, live)
        {
            continue;
        }
        section_starts.push((section.title, sections.len()));
        // The values view draws its own heading, which names the revision.
        if section.title != VALUES_CHANGE_TITLE {
            sections.push(if sections.is_empty() {
                first_section_title(section.title, cx).into_any_element()
            } else {
                section_title(section.title, cx).into_any_element()
            });
        }
        if section.rows.is_empty() {
            sections.push(absent_text(cx).into_any_element());
        }
        for detail in &section.rows {
            next_id += 1;
            let plain_class;
            let detail = match (detail, missing_class) {
                (DetailRow::Link { label, .. }, Some(class)) if label.as_ref() == "Class" => {
                    // A link to a StorageClass that does not exist would open nothing.
                    plain_class = DetailRow::field(
                        "Class",
                        KindCell::Text(format!("{class} (not found)").into()),
                    );
                    &plain_class
                }
                _ => detail,
            };
            sections.push(detail_element(detail, next_id, paint, cx));
        }
    }
    if let Some(owner) = &row.related_pods {
        sections.push(pods_section(owner, &row.object, live, cx));
    }
    if kind.has_labels() {
        sections.push(section_title("Labels", cx).into_any_element());
        sections.push(chips("labels", &row.labels, cx));
    }
    if let Some(annotations) = row.object.annotations() {
        sections.extend(annotations_section(
            annotations,
            paint.are_annotations_open,
            cx,
        ));
    }
    Overview {
        sections,
        section_starts,
    }
}

/// The title of the Deployment section that lists the revisions with their Roll back buttons.
pub(crate) const REVISIONS_TITLE: &str = "Revisions";

/// The overview of a row as the drawer's sections, and the index among them where each titled
/// section starts.
struct Overview {
    sections: Vec<AnyElement>,
    section_starts: Vec<(&'static str, usize)>,
}

/// The events of the open drawer's object, once they have loaded.
fn loaded_events<'a>(
    kind: ResourceKind,
    row: &KindRow,
    live: &'a LiveCluster,
) -> Option<&'a [EventSummary]> {
    let subject = event_subject(&ResourceKey::of_row(kind, row))?;
    live.events_of(&subject)?.ready_items()
}

/// The StorageClass of a claim row that the provisioner's events say does not exist.
fn missing_claim_class<'a>(
    kind: ResourceKind,
    row: &'a KindRow,
    live: &LiveCluster,
) -> Option<&'a str> {
    let KindObject::PersistentVolumeClaim(claim) = &row.object else {
        return None;
    };
    missing_storage_class(claim, loaded_events(kind, row, live)?)
}

/// The StorageClasses a Pending claim row is explained with, once they have loaded.
fn claim_classes_of(kind: ResourceKind, live: &LiveCluster) -> Option<&[StorageClassSummary]> {
    if kind != ResourceKind::PersistentVolumeClaims {
        return None;
    }
    live.related_of(&RelatedSubject::ClaimClasses)
        .and_then(RelatedList::storage_classes)?
        .ready_items()
}

/// The Services and pods an Ingress row's backends are checked against, once both have loaded.
fn ingress_backends_of<'a>(row: &KindRow, live: &'a LiveCluster) -> Option<IngressBackends<'a>> {
    let namespace = row.namespace.clone()?;
    if !matches!(row.object, KindObject::Ingress(_)) {
        return None;
    }
    let subject = RelatedSubject::PodServices { namespace };
    let services = live
        .related_of(&subject)
        .and_then(RelatedList::services)?
        .ready_items()?;
    Some(IngressBackends {
        services,
        pods: live.pods.ready_items()?,
    })
}

/// The WHY box of the row, read from its object, its owned pods (a Service's matching pods), and
/// the nodes. Rules that need pods wait until the pods list has loaded.
fn row_diagnosis(
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    now: jiff::Timestamp,
) -> Option<KindDiagnosis> {
    let source = live.companion_source(kind);
    let companion = source.as_ref().map(CompanionSource::lists);
    let (pods, service) = match &row.object {
        KindObject::Service(service) => {
            let slices = companion.and_then(CompanionLists::endpoint_slices);
            let health = service_health_of(service, &live.pods, slices);
            // Only V2 reads the pods, and only when no endpoint is ready, so the pass over the pods
            // that finds the matching ones runs only then; `service_health_of` made the other.
            let pods = live.pods.ready_items().map(|pods| {
                if health.is_unserved() {
                    matching_pods(service, pods)
                } else {
                    Vec::new()
                }
            });
            (pods, Some(health))
        }
        _ => (owned_pods(row, live), None),
    };
    // Only a ClusterRole that grants everything and a service account read the bindings; the index
    // is built for them alone.
    let lists = match &row.object {
        KindObject::Role(role) if role.namespace.is_none() && role.grants_everything() => {
            ready_binding_lists(companion)
        }
        KindObject::ServiceAccount(_) => ready_binding_lists(companion),
        _ => None,
    };
    let bindings = lists.as_ref().map(BindingIndex::build);
    let inputs = DiagnosisInputs {
        pods: pods.as_deref(),
        nodes: live.nodes.items(),
        service,
        bindings: bindings.as_ref(),
        tls_secrets: companion
            .and_then(CompanionLists::tls_secrets)
            .and_then(|list| list.ready_items()),
        events: loaded_events(kind, row, live),
        backends: ingress_backends_of(row, live),
        storage_classes: claim_classes_of(kind, live),
        now,
    };
    let problem = kind_diagnosis(&row.object, &inputs);
    // A rollout that is only going on is no problem, so it shows when nothing else does.
    match &row.object {
        KindObject::Deployment(deployment) => {
            problem.or_else(|| rollout_progress(deployment, &inputs))
        }
        _ => problem,
    }
}

/// The box: tone, title, text, and under it a link to the pod the text is about (a failed Job's
/// also offers its logs). `Alert` has no children, so the links are siblings, like the pod
/// drawer's WHY box.
fn why_box(diagnosis: &KindDiagnosis, kind: ResourceKind, cx: &Context<AppShell>) -> AnyElement {
    let title = format!("WHY · {}", diagnosis.title);
    let text = diagnosis.text.clone();
    let alert = match diagnosis.tone {
        StatusTone::Bad => Alert::error("why-box", text),
        StatusTone::Warn | StatusTone::Ok | StatusTone::Info | StatusTone::Done => {
            Alert::warning("why-box", text)
        }
    };
    let link = diagnosis.link.clone().and_then(|key| match &key {
        ResourceKey::Pod { name, .. } => Some((format!("Open pod {name} →"), key.clone())),
        ResourceKey::Kind {
            kind: ResourceKind::Secrets,
            name,
            ..
        } => Some((format!("Open secret {name} →"), key.clone())),
        ResourceKey::Kind {
            kind: ResourceKind::ResourceQuotas,
            name,
            ..
        } => Some((format!("Open quota {name} →"), key.clone())),
        ResourceKey::Kind {
            kind: ResourceKind::Services,
            name,
            ..
        } => Some((format!("Open service {name} →"), key.clone())),
        ResourceKey::Node { .. } | ResourceKey::Kind { .. } => None,
    });
    let logs_key = diagnosis
        .link
        .clone()
        .filter(|key| kind == ResourceKind::Jobs && matches!(key, ResourceKey::Pod { .. }));
    let why_link = |id: &'static str, label: String| {
        div()
            .id(id)
            .cursor_pointer()
            .text_sm()
            .text_color(cx.theme().link)
            .underline()
            .child(label)
    };
    let has_links = link.is_some();
    let links = h_flex()
        .gap_3()
        .children(link.map(|(label, key)| {
            why_link("why-open-object", label).on_click(
                cx.listener(move |shell, _, window, cx| open_link(shell, key.clone(), window, cx)),
            )
        }))
        .children(logs_key.map(|key| {
            why_link("why-view-logs", "View logs →".to_owned()).on_click(
                cx.listener(move |shell, _, window, cx| shell.open_pod_logs(&key, window, cx)),
            )
        }));
    v_flex()
        .gap_1()
        .child(alert.title(title))
        .children(has_links.then_some(links))
        .into_any_element()
}

/// The Forward buttons of a drawer's Port rows and the object they forward to.
struct PortRows<'a> {
    buttons: &'a PortButtons<'a>,
    subject: ForwardSubject,
}

/// What painting a drawer row may read besides the row itself.
pub(crate) struct DrawerPaint<'a> {
    kind: ResourceKind,
    row: &'a KindRow,
    live: &'a LiveCluster,
    /// The Forward buttons of the ports: the drawer subject's own cluster.
    ports: Option<PortRows<'a>>,
    now: jiff::Timestamp,
    /// The values view of the open Secret drawer, which draws the Data section.
    secret_values: Option<&'a Entity<SecretValuesView>>,
    /// The Helm view of the open release drawer, which draws the values diff.
    helm: Option<&'a Entity<HelmReleaseView>>,
    /// The revision a History button chose, for the `shown` mark.
    helm_revision: Option<u32>,
    /// The cluster of the drawer: the views above belong to one cluster's object.
    cluster: Option<&'a ClusterRef>,
    /// The gate of the Roll back buttons of a Deployment's revisions.
    roll_back: Option<DrawerWriteGate>,
    /// Whether the Annotations section shows its list.
    are_annotations_open: bool,
}

impl<'a> DrawerPaint<'a> {
    pub(crate) fn new(
        kind: ResourceKind,
        row: &'a KindRow,
        live: &'a LiveCluster,
        now: jiff::Timestamp,
    ) -> Self {
        Self {
            kind,
            row,
            live,
            ports: None,
            now,
            secret_values: None,
            helm: None,
            helm_revision: None,
            cluster: None,
            roll_back: None,
            are_annotations_open: false,
        }
    }
}

impl<'a> DrawerPaint<'a> {
    fn in_cluster(mut self, cluster: &'a ClusterRef) -> Self {
        self.cluster = Some(cluster);
        self
    }

    fn with_roll_back(mut self, gate: Option<DrawerWriteGate>) -> Self {
        self.roll_back = gate;
        self
    }

    fn with_annotations_open(mut self, is_open: bool) -> Self {
        self.are_annotations_open = is_open;
        self
    }

    /// The Forward buttons of the Port rows, for the object of the row.
    fn with_ports(mut self, buttons: &'a PortButtons<'a>) -> Self {
        self.ports = row_subject(self.row).map(|subject| PortRows { buttons, subject });
        self
    }

    fn with_secret_values(mut self, view: Option<&'a Entity<SecretValuesView>>) -> Self {
        self.secret_values = view;
        self
    }

    fn with_helm(
        mut self,
        view: Option<&'a Entity<HelmReleaseView>>,
        revision: Option<u32>,
    ) -> Self {
        self.helm = view;
        self.helm_revision = revision;
        self
    }
}

/// The elements of rows that a live section builds at paint time, drawn like the sections of the
/// drawer. `id_base` keeps their element ids apart from the others in the drawer.
pub(crate) fn live_detail_rows(
    rows: &[DetailRow],
    id_base: usize,
    paint: &DrawerPaint,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    rows.iter()
        .enumerate()
        .map(|(offset, detail)| detail_element(detail, id_base + offset, paint, cx))
        .collect()
}

fn detail_element(
    detail: &DetailRow,
    id: usize,
    paint: &DrawerPaint,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = paint.now;
    match detail {
        DetailRow::Bar {
            label,
            percent,
            text,
            tone,
        } => bar_row(label, *percent, text, *tone, id, cx),
        // The values view draws the Data section; one frame before it exists, or for another Secret,
        // the masked rows without buttons stand in.
        DetailRow::Live(LiveContent::SecretData)
            if paint.secret_values.is_some_and(|view| {
                paint.cluster.is_some_and(|cluster| {
                    let key = ResourceKey::of_row(paint.kind, paint.row);
                    view.read(cx)
                        .is_for(&ClusterObject::new(cluster.clone(), key))
                })
            }) =>
        {
            paint.secret_values.map_or_else(
                || div().into_any_element(),
                |view| view.clone().into_any_element(),
            )
        }
        DetailRow::Live(LiveContent::HelmValuesChange) => helm_values_change(paint, cx),
        DetailRow::Live(LiveContent::HelmHistory) => v_flex()
            .children(helm_history_rows(
                paint.kind,
                paint.row,
                paint.live,
                effective_helm_revision(paint),
                now,
                cx,
            ))
            .into_any_element(),
        DetailRow::Live(content) => v_flex()
            .children(live_rows(
                *content,
                paint.kind,
                paint.row,
                paint.live,
                now,
                paint.roll_back.as_ref(),
                cx,
            ))
            .into_any_element(),
        DetailRow::Field { label, value } => {
            wide_detail_row(label.clone(), field_value(value, id, now, cx), cx).into_any_element()
        }
        DetailRow::CopyField { label, text } => wide_detail_row(
            label.clone(),
            copyable_mono(("detail", id), text.clone(), cx),
            cx,
        )
        .into_any_element(),
        DetailRow::Chips(terms) => chips(("detail-chips", id), terms, cx),
        DetailRow::Note(text) => div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.clone())
            .into_any_element(),
        DetailRow::Code(text) => code_block(text, cx),
        DetailRow::Table(text) => table_block(text, id, cx),
        DetailRow::Link {
            label,
            text,
            target,
        } => {
            let link = link_text(id, text, target.clone(), cx);
            wide_detail_row(label.clone(), link, cx).into_any_element()
        }
        DetailRow::StackedLink {
            label,
            text,
            target,
        } => {
            let link = link_text(id, text, target.clone(), cx);
            stacked_row(label, link, id, cx)
        }
        DetailRow::Condition {
            name,
            status,
            since,
            message,
        } => condition_element(name, status, *since, message.as_ref(), now, cx),
        DetailRow::Port { text, port, is_tcp } => match &paint.ports {
            Some(ports) => {
                let choice = PortChoice {
                    label: text.to_string(),
                    remote_port: *port,
                    is_tcp: *is_tcp,
                };
                ports.buttons.row(text, id, &ports.subject, &choice, cx)
            }
            None => {
                let button = PortButton::Disabled("Not connected".into());
                port_row(text, id, &button, None, cx)
            }
        },
        DetailRow::Stacked { label, value } => {
            stacked_row(label, field_value(value, id, now, cx), id, cx)
        }
    }
}

/// The revision the Helm tabs show: the one a History button chose, else the latest.
fn effective_helm_revision(paint: &DrawerPaint) -> Option<u32> {
    let latest = match &paint.row.object {
        KindObject::HelmRelease(release) => Some(release.revision),
        _ => None,
    };
    paint.helm_revision.or(latest)
}

/// The values-change section of a release: the Helm view when it is for the latest revision, else
/// a heading and a note (one frame before the shell's sync creates the view).
fn helm_values_change(paint: &DrawerPaint, cx: &Context<AppShell>) -> AnyElement {
    let KindObject::HelmRelease(release) = &paint.row.object else {
        return div().into_any_element();
    };
    let revision = HelmRevisionRef {
        namespace: release.namespace.clone(),
        release: release.name.clone(),
        revision: release.revision,
    };
    let helm = paint.helm.filter(|view| {
        paint
            .cluster
            .is_some_and(|cluster| view.read(cx).is_for(cluster, &revision))
    });
    if let Some(view) = helm {
        return view.clone().into_any_element();
    }
    v_flex()
        .child(section_title(VALUES_CHANGE_TITLE, cx))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Loading…"),
        )
        .into_any_element()
}

/// A label, a bar toned by `tone` (the kit color without one), then the text in mono.
pub(crate) fn bar_row(
    label: &SharedString,
    percent: u8,
    text: &SharedString,
    tone: Option<StatusTone>,
    id: usize,
    cx: &App,
) -> AnyElement {
    let mut bar = Progress::new(("bar", id)).value(f32::from(percent));
    if let Some(tone) = tone {
        bar = bar.color(tone_color(tone, cx));
    }
    let value = h_flex()
        .gap_2()
        .items_center()
        .child(div().flex_1().min_w_0().child(bar))
        .child(
            div()
                .flex_shrink_0()
                .font_family(cx.theme().mono_font_family.clone())
                .child(text.clone()),
        );
    wide_detail_row(label.clone(), value, cx).into_any_element()
}

/// Preformatted text that wraps, such as an event message.
fn code_block(text: &SharedString, cx: &App) -> AnyElement {
    code_base(text, cx).into_any_element()
}

/// A text table whose long lines scroll sideways instead of wrapping, which would break the
/// column alignment.
fn table_block(text: &SharedString, id: usize, cx: &App) -> AnyElement {
    code_base(text, cx)
        .id(("table", id))
        .overflow_x_scroll()
        .whitespace_nowrap()
        .into_any_element()
}

fn code_base(text: &SharedString, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .w_full()
        .min_w_0()
        .px_2()
        .py_1p5()
        .rounded(theme.radius)
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .child(text.clone())
}

/// The label above its value, for labels that do not fit the label column.
/// The status line of a condition with its age, then the message in muted wrapped text under the
/// status, so a long controller message never widens the drawer.
fn condition_element(
    name: &SharedString,
    status: &StatusLabel,
    since: Option<jiff::Timestamp>,
    message: Option<&SharedString>,
    now: jiff::Timestamp,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let status_line = h_flex()
        .gap_1()
        .child(toned_text(status.clone(), cx).truncate())
        .children(since.map(|at| {
            div()
                .flex_shrink_0()
                .text_color(muted)
                .child(format!("· {} ago", format_age(Some(at), now)))
        }));
    v_flex()
        .child(wide_detail_row(name.clone(), status_line, cx))
        .children(
            message.map(|message| {
                wide_detail_row("", div().text_color(muted).child(message.clone()), cx)
            }),
        )
        .into_any_element()
}

fn stacked_row(label: &SharedString, value: AnyElement, id: usize, cx: &App) -> AnyElement {
    v_flex()
        .py_1()
        .text_sm()
        .child(
            truncated_text(("stacked", id), label.clone()).text_color(cx.theme().muted_foreground),
        )
        .child(div().min_w_0().overflow_hidden().child(value))
        .into_any_element()
}

fn field_value(value: &KindCell, id: usize, now: jiff::Timestamp, cx: &App) -> AnyElement {
    let mono = cx.theme().mono_font_family.clone();
    match value {
        KindCell::Text(text) => truncated_text(("detail", id), text.clone()).into_any_element(),
        KindCell::Hinted { text, tooltip: all } | KindCell::Images { text, all } => {
            truncated_text_with_tooltip(("detail", id), text.clone(), all.clone())
                .into_any_element()
        }
        KindCell::Mono(text) => truncated_text(("detail", id), text.clone())
            .font_family(mono)
            .into_any_element(),
        KindCell::Qualified { prefix, text } => {
            let text = match prefix {
                Some(prefix) => SharedString::from(format!("{prefix}/{text}")),
                None => text.clone(),
            };
            truncated_text(("detail", id), text)
                .font_family(mono)
                .into_any_element()
        }
        KindCell::Toned(label) => toned_text(label.clone(), cx).truncate().into_any_element(),
        KindCell::MonoWithMore { text, more } => truncated_text(
            ("detail", id),
            SharedString::from(format!("{text} +{more}")),
        )
        .font_family(mono)
        .into_any_element(),
        KindCell::Quantity { text, tone, .. } => {
            let quantity = truncated_text(("detail", id), text.clone()).font_family(mono);
            match tone {
                Some(tone) => quantity.text_color(tone_color(*tone, cx)),
                None => quantity,
            }
            .into_any_element()
        }
        KindCell::CronStatus(cron_job) => {
            toned_text(cron_state_at(cron_job, now).label(), cx).into_any_element()
        }
        KindCell::Expiry { not_after } => toned_text(expiry_detail_label(*not_after, now), cx)
            .truncate()
            .into_any_element(),
        KindCell::Date { at, rule } => {
            let text = div()
                .truncate()
                .child(format!("{at} ({})", date_text(*at, now)));
            match date_tone(*rule, *at, now) {
                Some(tone) => text.text_color(tone_color(tone, cx)),
                None => text,
            }
            .into_any_element()
        }
        KindCell::Absent => absent_text(cx).into_any_element(),
        KindCell::NextRun(schedule) => match next_run_text(schedule, now) {
            Some(text) => div().truncate().child(text).into_any_element(),
            None => absent_text(cx).into_any_element(),
        },
        KindCell::Duration {
            started_at: None, ..
        } => absent_text(cx).into_any_element(),
        KindCell::Duration {
            started_at,
            finished_at,
        } => div()
            .truncate()
            .child(format_age(*started_at, finished_at.unwrap_or(now)))
            .into_any_element(),
        KindCell::Age { at: None, .. } => absent_text(cx).into_any_element(),
        KindCell::Age { at: Some(at), tone } => {
            let text = div().truncate().child(format!(
                "{} ({} ago)",
                format_local_time(*at, &jiff::tz::TimeZone::system()),
                format_age(Some(*at), now)
            ));
            match tone {
                Some(tone) => text.text_color(tone_color(*tone, cx)),
                None => text,
            }
            .into_any_element()
        }
    }
}
