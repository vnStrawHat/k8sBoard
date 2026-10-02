//! The overview drawer shared by the explorer kinds. The sections come from the row
//! builders; this module only renders them.

use std::rc::Rc;

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
use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::certificate_expiry::expiry_label;
use crate::cluster_session::{ClusterSession, CompanionLists, LiveCluster};
use crate::drawer::{
    DrawerBody, DrawerHeader, DrawerState, DrawerTab, absent_text, chips, created_text,
    drawer_frame, drawer_tab_bar, drawer_tabs, expand_toggle, link_text, menu_button, port_row,
    section_title, shown_tab, tab_titles, truncated_text, truncated_text_with_tooltip,
    wide_detail_row, yaml_body,
};
use crate::kind_diagnosis::{DiagnosisInputs, KindDiagnosis, kind_diagnosis};
use crate::kind_join::{matching_pods, service_health_of};
use crate::kind_row::{DetailRow, KindCell, KindObject, KindRow, LiveContent};
use crate::live_sections::{live_rows, next_run_text, owned_pods};
use crate::monitor_tab::{MonitorView, monitor_tab};
use crate::object_events::{event_subject, recent_events};
use crate::related_pods::pods_section;
use crate::resource_actions::{
    MenuExtras, OpenUrl, kind_menu, open_url_choice, open_url_menu_item, port_forward_reason,
    secret_menu,
};
use crate::resource_kind::ResourceKind;
use crate::secret_values::{SecretValuesView, ValueAccess};
use crate::status_tone::{StatusTone, tone_color, toned_text};
use crate::table_selection::ResourceKey;

pub(crate) fn kind_drawer(
    kind: ResourceKind,
    row: &KindRow,
    state: &DrawerState,
    live: &LiveCluster,
    session: &Entity<ClusterSession>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = jiff::Timestamp::now();
    let header = DrawerHeader {
        kind_badge: kind.badge(),
        name: header_name(row),
        subtitle: subtitle(row, now, cx),
        menu: kind_menu_button(kind, row, session, cx.weak_entity()),
        expand: expand_toggle(state, cx),
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
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
        DrawerTab::Overview | DrawerTab::Containers => DrawerBody::Scrolling(overview(
            kind,
            row,
            live,
            state.secret_values.as_ref(),
            now,
            cx,
        )),
    };
    let tab_bar = drawer_tab_bar(tab_titles(tabs, 0, events), shown, cx);
    drawer_frame(header, tab_bar, body, state.width(), cx).into_any_element()
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
        | KindObject::Secret(_) => None,
    }
}

/// The menu reads the session when it opens, so it shows the access state and the row of
/// that moment.
fn kind_menu_button(
    kind: ResourceKind,
    row: &KindRow,
    session: &Entity<ClusterSession>,
    shell: WeakEntity<AppShell>,
) -> AnyElement {
    let session = session.clone();
    let key = ResourceKey::of_row(kind, row);
    menu_button()
        .dropdown_menu(move |menu, window, cx| {
            let open_url = (kind == ResourceKind::Ingresses).then(|| {
                let choice = session
                    .read(cx)
                    .live()
                    .and_then(|live| live.kind_list(kind))
                    .and_then(|explorer| {
                        explorer
                            .list
                            .items()
                            .iter()
                            .find(|row| key.is_row(kind, row))
                    })
                    .map_or(OpenUrl::Unavailable, open_url_choice);
                open_url_menu_item(choice, window, cx)
            });
            let secret = (kind == ResourceKind::Secrets)
                .then(|| {
                    let row = session
                        .read(cx)
                        .live()
                        .and_then(|live| live.kind_list(kind))
                        .and_then(|explorer| {
                            explorer
                                .list
                                .items()
                                .iter()
                                .find(|row| key.is_row(kind, row))
                        })
                        .cloned()?;
                    let access = shell
                        .read_with(cx, |shell, _| shell.secret_value_access())
                        .unwrap_or(ValueAccess::Blocked);
                    secret_menu(&row, key.clone(), access, &shell, window, cx)
                })
                .flatten();
            let Some(live) = session.read(cx).live() else {
                return menu;
            };
            let current = live.kind_list(kind).and_then(|explorer| {
                explorer
                    .list
                    .items()
                    .iter()
                    .find(|row| key.is_row(kind, row))
            });
            match current {
                Some(row) => kind_menu(
                    menu,
                    kind,
                    row,
                    &live.access,
                    live.pods.items(),
                    &shell,
                    MenuExtras { open_url, secret },
                ),
                None => menu,
            }
        })
        .into_any_element()
}

/// The row's sections in order, then the related pods, then the labels.
fn overview(
    kind: ResourceKind,
    row: &KindRow,
    live: &LiveCluster,
    secret_values: Option<&Entity<SecretValuesView>>,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let paint = DrawerPaint::new(kind, row, live, now).with_secret_values(secret_values);
    // Gives every element that needs an id one that is unique inside the drawer.
    let mut next_id = 0_usize;
    let mut column = v_flex();
    if let Some(diagnosis) = row_diagnosis(row, live, now) {
        column = column.child(why_box(&diagnosis, cx));
    }
    for section in &row.sections {
        column = column.child(section_title(section.title, cx));
        if section.rows.is_empty() {
            column = column.child(absent_text(cx));
        }
        for detail in &section.rows {
            next_id += 1;
            column = column.child(detail_element(detail, next_id, &paint, cx));
        }
    }
    if let Some(owner) = &row.related_pods {
        column = column.child(pods_section(owner, &row.object, live, cx));
    }
    if kind.has_labels() {
        column = column
            .child(section_title("Labels", cx))
            .child(chips(&row.labels, cx));
    }
    column.into_any_element()
}

/// The WHY box of the row, read from its object, its owned pods (a Service's matching pods), and
/// the nodes. Rules that need pods wait until the pods list has loaded.
fn row_diagnosis(row: &KindRow, live: &LiveCluster, now: jiff::Timestamp) -> Option<KindDiagnosis> {
    let (pods, service) = match &row.object {
        KindObject::Service(service) => {
            let slices = live.companion().and_then(CompanionLists::endpoint_slices);
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
            ready_binding_lists(live.companion())
        }
        KindObject::ServiceAccount(_) => ready_binding_lists(live.companion()),
        _ => None,
    };
    let bindings = lists.as_ref().map(BindingIndex::build);
    kind_diagnosis(
        &row.object,
        &DiagnosisInputs {
            pods: pods.as_deref(),
            nodes: live.nodes.items(),
            service,
            bindings: bindings.as_ref(),
            tls_secrets: live
                .companion()
                .and_then(CompanionLists::tls_secrets)
                .and_then(|list| list.ready_items()),
            now,
        },
    )
}

/// The box: tone, title, text, and under it a link to the pod the text is about. `Alert` has no
/// children, so the link is a sibling, like the pod drawer's WHY box.
fn why_box(diagnosis: &KindDiagnosis, cx: &Context<AppShell>) -> AnyElement {
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
        ResourceKey::Node { .. } | ResourceKey::Kind { .. } => None,
    });
    v_flex()
        .gap_1()
        .child(alert.title(title))
        .children(link.map(|(label, key)| {
            div()
                .id("why-open-object")
                .cursor_pointer()
                .text_sm()
                .text_color(cx.theme().link)
                .underline()
                .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(key.clone(), cx)))
                .child(label)
        }))
        .into_any_element()
}

/// What painting a drawer row may read besides the row itself.
pub(crate) struct DrawerPaint<'a> {
    kind: ResourceKind,
    row: &'a KindRow,
    live: &'a LiveCluster,
    forward_reason: SharedString,
    now: jiff::Timestamp,
    /// The values view of the open Secret drawer, which draws the Data section.
    secret_values: Option<&'a Entity<SecretValuesView>>,
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
            forward_reason: port_forward_reason(&live.access),
            now,
            secret_values: None,
        }
    }
}

impl<'a> DrawerPaint<'a> {
    fn with_secret_values(mut self, view: Option<&'a Entity<SecretValuesView>>) -> Self {
        self.secret_values = view;
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
                view.read(cx)
                    .is_for(&ResourceKey::of_row(paint.kind, paint.row))
            }) =>
        {
            paint.secret_values.map_or_else(
                || div().into_any_element(),
                |view| view.clone().into_any_element(),
            )
        }
        DetailRow::Live(content) => v_flex()
            .children(live_rows(
                *content, paint.kind, paint.row, paint.live, now, cx,
            ))
            .into_any_element(),
        DetailRow::Field { label, value } => {
            wide_detail_row(label.clone(), field_value(value, id, now, cx), cx).into_any_element()
        }
        DetailRow::Chips(terms) => chips(terms, cx),
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
        DetailRow::Port { text } => port_row(text, id, &paint.forward_reason, cx),
        DetailRow::Stacked { label, value } => {
            stacked_row(label, field_value(value, id, now, cx), id, cx)
        }
    }
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
        KindCell::Hinted { text, tooltip } => {
            truncated_text_with_tooltip(("detail", id), text.clone(), tooltip.clone())
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
        KindCell::Expiry { not_after } => toned_text(expiry_label(*not_after, now), cx)
            .truncate()
            .into_any_element(),
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
            let text = div()
                .truncate()
                .child(format!("{at} ({} ago)", format_age(Some(*at), now)));
            match tone {
                Some(tone) => text.text_color(tone_color(*tone, cx)),
                None => text,
            }
            .into_any_element()
        }
    }
}
