use std::rc::Rc;

use cluster::{
    ByteAmount, ContainerKind, ContainerResource, ContainerState, ContainerSummary, CpuAmount,
    EventSummary, PodCondition, PodSummary, ResourceUsage, ServiceSummary, VolumeSource,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::clipboard_copy::copyable_mono;
use crate::cluster_session::{
    ClusterSession, LiveCluster, LiveList, RelatedList, denied_related_check,
};
use crate::container_detail::{
    ContainerDetailInput, container_detail, last_state_text, volume_target,
};
use crate::dock::Dock;
use crate::drawer::{
    DrawerBody, DrawerChrome, DrawerHeader, DrawerSize, DrawerState, DrawerTab, absent_text, chips,
    created_text, detail_row, drawer_frame, drawer_tab_bar, drawer_tabs, first_section_title,
    link_text, menu_button, section_title, shown_tab, tab_titles, value_or_absent, yaml_body,
};
use crate::kind_join::services_selecting;
use crate::kind_row::deployment_of_pod;
use crate::monitor_tab::{MonitorView, monitor_tab};
use crate::object_events::{event_subject, recent_events};
use crate::pod_diagnosis::{PodDiagnosis, PullSecret, pod_diagnosis, pull_secrets};
use crate::port_forward_menu::{ForwardMenu, PortButtons, pod_drawer_subject};
use crate::related_objects::key_related_subject;
use crate::resource_actions::{
    LogsMenu, PodMenuItems, PodMenuLinks, ShellMenu, container_menu, pod_menu, view_logs_reason,
};
use crate::resource_kind::{POD_ICON, ResourceKind};
use crate::row_context::RowContext;
use crate::status_tone::{
    StatusLabel, StatusTone, container_state_label, pod_status_label, toned_text,
};
use crate::table_selection::ResourceKey;
use crate::usage_bar::UsageBar;
use crate::usage_format::{Measure, usage_tone};

/// Element ids of the Volumes links, clear of the overview's own link ids.
const VOLUME_LINK_ID_BASE: usize = 100;
/// Element ids of the Services links, clear of the Volumes links.
const SERVICE_LINK_ID_BASE: usize = 1_000;
/// Bounds the render cost of a pod that a namespace-wide selector puts behind many services.
const MAX_LISTED_SERVICES: usize = 20;

/// The sidebar fits the longest container name between these widths: 8 px per character plus the
/// row's padding and state marks.
const CONTAINER_LIST_MIN_WIDTH: f32 = 240.;
const CONTAINER_LIST_MAX_WIDTH: f32 = 360.;

fn container_list_width(containers: &[ContainerSummary]) -> Pixels {
    let longest = containers
        .iter()
        .map(|container| container.name.chars().count())
        .max()
        .unwrap_or(0);
    px((longest as f32 * 8. + 96.).clamp(CONTAINER_LIST_MIN_WIDTH, CONTAINER_LIST_MAX_WIDTH))
}

pub(crate) fn pod_drawer(
    pod: &PodSummary,
    state: &DrawerState,
    session: &Entity<ClusterSession>,
    row: &RowContext,
    dock: &WeakEntity<Dock>,
    chrome: DrawerChrome<'_>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = jiff::Timestamp::now();
    let DrawerChrome {
        forward,
        navigation,
    } = chrome;
    let header = DrawerHeader {
        kind_icon: POD_ICON,
        kind_name: "Pod".into(),
        name: pod.name.clone().into(),
        subtitle: subtitle(pod, now, cx),
        menu: pod_menu_button(pod, session, row, dock, cx.weak_entity()),
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
        navigation,
    };
    let events = pod_events(pod, session, cx);
    let tabs = drawer_tabs(&ResourceKey::of_pod(pod));
    let shown = shown_tab(tabs, state.tab);
    let loaded_events = events.and_then(LiveList::ready_items);
    let body = match shown {
        // A pod drawer has no Helm tabs, so `shown_tab` never yields them.
        DrawerTab::Overview | DrawerTab::Values | DrawerTab::Manifest | DrawerTab::Notes => {
            DrawerBody::Scrolling(overview(
                pod,
                loaded_events,
                session.read(cx).live(),
                now,
                cx,
            ))
        }
        DrawerTab::Containers => DrawerBody::Filling(containers_tab(
            pod,
            state,
            loaded_events,
            forward,
            ContainerMenuLinks { session, row, dock },
            now,
            cx,
        )),
        DrawerTab::Monitor => DrawerBody::Scrolling(match session.read(cx).live() {
            Some(live) => monitor_tab(&MonitorView::of_pods(state, live), cx),
            None => div().into_any_element(),
        }),
        DrawerTab::Yaml => yaml_body(state),
        DrawerTab::Events => DrawerBody::Scrolling(recent_events(events, cx)),
    };
    let titles = tab_titles(tabs, pod.containers.len(), events);
    let tab_bar = drawer_tab_bar(titles, shown, cx);
    drawer_frame(header, tab_bar, body, state.width(DrawerSize::Wide), cx).into_any_element()
}

fn subtitle(pod: &PodSummary, now: jiff::Timestamp, cx: &App) -> AnyElement {
    h_flex()
        .gap_1()
        .text_sm()
        .child(toned_text(pod_status_label(pod), cx))
        .child(
            div()
                .text_color(cx.theme().muted_foreground)
                .child(subtitle_detail(pod, now)),
        )
        .into_any_element()
}

/// `· namespace · created 2d ago`.
fn subtitle_detail(pod: &PodSummary, now: jiff::Timestamp) -> String {
    let parts = std::iter::once(pod.namespace.clone()).chain(created_text(pod.created_at, now));
    format!("· {}", parts.collect::<Vec<_>>().join(" · "))
}

/// The menu reads the session when it opens, so it shows the access state and the pod of
/// that moment.
fn pod_menu_button(
    pod: &PodSummary,
    session: &Entity<ClusterSession>,
    row: &RowContext,
    dock: &WeakEntity<Dock>,
    shell: WeakEntity<AppShell>,
) -> AnyElement {
    // Weak: a rendered menu closure must not keep a session alive after a cluster switch.
    let session = session.downgrade();
    let row = row.clone();
    let dock = dock.clone();
    let key = ResourceKey::of_pod(pod);
    menu_button()
        .dropdown_menu(move |menu, window, cx| {
            let Some(session) = session.upgrade() else {
                return menu;
            };
            // The submenus are built from the app, so they are made before the session is borrowed.
            let (logs_menu, connection, shell_menu, forward_menu) = {
                let session = session.read(cx);
                let (Some(live), Some(guard)) = (session.live(), session.guard(cx)) else {
                    return menu;
                };
                let Some(pod) = live.pods.items().iter().find(|pod| key.is_pod(pod)) else {
                    return menu;
                };
                (
                    LogsMenu::of(pod, &live.access),
                    live.connection().clone(),
                    ShellMenu::of(pod, &guard),
                    ForwardMenu::of(pod_drawer_subject(pod), &row.cluster, &guard),
                )
            };
            let shell_items = shell_menu.items(&row, &shell, window, cx);
            let items = PodMenuItems {
                view_logs: logs_menu.item(connection, &row, &dock, window, cx),
                open_shell: shell_items.open_shell,
                debug_container: shell_items.debug_container,
                port_forward: forward_menu.item(&shell, window, cx),
            };
            let session = session.read(cx);
            let (Some(live), Some(guard)) = (session.live(), session.guard(cx)) else {
                return menu;
            };
            match live.pods.items().iter().find(|pod| key.is_pod(pod)) {
                Some(pod) => {
                    let links = PodMenuLinks {
                        dock: &dock,
                        shell: &shell,
                    };
                    pod_menu(menu, pod, &guard, &row, &links, items)
                }
                None => menu,
            }
        })
        .into_any_element()
}

/// The pod's events list, or `None` while the debounce or a switch is in progress.
fn pod_events<'a>(
    pod: &PodSummary,
    session: &'a Entity<ClusterSession>,
    cx: &'a App,
) -> Option<&'a LiveList<EventSummary>> {
    let subject = event_subject(&ResourceKey::of_pod(pod))?;
    session.read(cx).live()?.events_of(&subject)
}

fn overview(
    pod: &PodSummary,
    events: Option<&[EventSummary]>,
    live: Option<&LiveCluster>,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let running = pod
        .containers
        .iter()
        .filter(|container| matches!(container.state, ContainerState::Running { .. }))
        .count();
    let node = match &pod.node_name {
        Some(name) => link_text(
            1,
            &name.clone().into(),
            ResourceKey::Node { name: name.clone() },
            cx,
        )
        .into_any_element(),
        None => absent_text(cx).into_any_element(),
    };
    let controller = pod.controller.as_ref().map(|controller| {
        let text = format!("{}/{}", controller.kind, controller.name);
        let target =
            ResourceKey::of_object(&controller.kind, Some(&pod.namespace), &controller.name);
        match target {
            Some(target) => link_text(2, &text.into(), target, cx).into_any_element(),
            None => div().truncate().child(text).into_any_element(),
        }
    });
    let service_account = pod
        .service_account
        .as_deref()
        .and_then(|name| {
            let target = ResourceKey::of_object("ServiceAccount", Some(&pod.namespace), name)?;
            Some(link_text(3, &name.to_owned().into(), target, cx))
        })
        .unwrap_or_else(|| value_or_absent(pod.service_account.as_deref(), cx));
    // Set only for a pod of a Deployment's ReplicaSet; the row is absent otherwise.
    let deployment = deployment_of_pod(pod).and_then(|name| {
        let target = ResourceKey::of_object("Deployment", Some(&pod.namespace), name)?;
        Some(detail_row(
            "Deployment",
            link_text(4, &name.to_owned().into(), target, cx),
            cx,
        ))
    });
    let labels: Vec<SharedString> = pod.labels.iter().cloned().map(SharedString::from).collect();
    let existing = secret_names_in(live, &pod.namespace);
    let pull = pull_secrets(pod, existing.as_deref());
    let diagnosis = pod_diagnosis(pod, events, now).map(|found| found.with_pull_secrets(&pull));
    let pull_row = (!pull.is_empty()).then(|| {
        detail_row(
            "Image pull secrets",
            pull_secret_links(&pull, &pod.namespace, 10, cx),
            cx,
        )
    });
    // The first heading keeps its room above only when a box comes before it.
    let pod_title = if diagnosis.is_some() {
        section_title("Pod", cx).into_any_element()
    } else {
        first_section_title("Pod", cx).into_any_element()
    };
    v_flex()
        .children(diagnosis.map(|diagnosis| {
            let links = why_pull_links(&diagnosis, &pull, &pod.namespace, cx);
            why_box(&diagnosis, links, cx)
        }))
        .child(pod_title)
        .child(detail_row("Node", node, cx))
        .child(detail_row(
            "Pod IP",
            match pod.pod_ip.as_deref() {
                Some(ip) => copyable_mono("pod-ip", ip.to_owned(), cx),
                None => absent_text(cx).into_any_element(),
            },
            cx,
        ))
        .child(detail_row(
            "QoS class",
            value_or_absent(pod.qos_class.as_deref(), cx),
            cx,
        ))
        .child(detail_row("Service account", service_account, cx))
        .children(pull_row)
        .child(detail_row(
            "Controlled by",
            controller.unwrap_or_else(|| absent_text(cx).into_any_element()),
            cx,
        ))
        .children(deployment)
        .child(section_title("Conditions", cx))
        .child(conditions(pod, cx))
        .child(section_title(
            format!("Containers · {running} of {} running", pod.containers.len()),
            cx,
        ))
        .children(
            pod.containers
                .iter()
                .enumerate()
                .map(|(index, container)| container_row(index, container, now, cx)),
        )
        .children(services_section(pod, live, cx))
        .children(volumes_section(pod, cx))
        .child(section_title("Labels", cx))
        .child(chips("pod-labels", &labels, cx))
        .into_any_element()
}

/// The names of the Secrets in `namespace` while the Secrets screen has them loaded; `None`
/// otherwise, so nothing is called missing on a guess.
fn secret_names_in<'a>(live: Option<&'a LiveCluster>, namespace: &str) -> Option<Vec<&'a str>> {
    let list = &live?.kind_list(ResourceKind::Secrets)?.list;
    list.ready_count()?;
    Some(
        list.items()
            .iter()
            .filter(|row| row.namespace.as_deref() == Some(namespace))
            .map(|row| row.name.as_str())
            .collect(),
    )
}

/// Each pull secret that exists opens its Secrets row; a missing one is plain text. `id_base`
/// keeps the link ids of two rows on one screen apart.
fn pull_secret_links(
    secrets: &[PullSecret],
    namespace: &str,
    id_base: usize,
    cx: &Context<AppShell>,
) -> AnyElement {
    h_flex()
        .gap_2()
        .flex_wrap()
        .children(secrets.iter().enumerate().map(|(index, secret)| {
            let target = ResourceKey::of_object("Secret", Some(namespace), &secret.name);
            match target {
                Some(target) if !secret.is_missing => {
                    link_text(id_base + index, &secret.name.clone().into(), target, cx)
                }
                _ if secret.is_missing => div()
                    .child(format!("{} (missing)", secret.name))
                    .into_any_element(),
                _ => div().child(secret.name.clone()).into_any_element(),
            }
        }))
        .into_any_element()
}

/// The links under the WHY box of a failed pull.
fn why_pull_links(
    diagnosis: &PodDiagnosis,
    secrets: &[PullSecret],
    namespace: &str,
    cx: &Context<AppShell>,
) -> Option<AnyElement> {
    if !diagnosis.is_pull_failure() || secrets.is_empty() {
        return None;
    }
    Some(
        h_flex()
            .gap_2()
            .flex_wrap()
            .text_sm()
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("Open pull secret:"),
            )
            .child(pull_secret_links(secrets, namespace, 20, cx))
            .into_any_element(),
    )
}

/// The WHY box. `Alert` has no children, so the link that opens the container is a sibling
/// right under it.
fn why_box(
    diagnosis: &PodDiagnosis,
    pull_links: Option<AnyElement>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let title = match &diagnosis.container {
        Some(name) => format!("WHY · CONTAINER \"{name}\""),
        None => "WHY · POD".to_owned(),
    };
    let text = diagnosis.display_text();
    let alert = match diagnosis.tone {
        StatusTone::Bad => Alert::error("why-box", text),
        _ => Alert::warning("why-box", text),
    };
    v_flex()
        .gap_1()
        .child(alert.title(title))
        .children(pull_links)
        .children(diagnosis.container.clone().map(|name| {
            let label = format!("Open container \"{name}\" →");
            div()
                .id("why-open-container")
                .cursor_pointer()
                .text_sm()
                .text_color(cx.theme().link)
                .underline()
                .on_click(
                    cx.listener(move |shell, _, _, cx| shell.open_container(name.clone(), cx)),
                )
                .child(label)
        }))
        .into_any_element()
}

/// `ClusterIP · 80/TCP, 443/TCP`: the type and ports of a service that selects the pod.
fn service_summary_text(service: &ServiceSummary) -> String {
    let ports = service
        .ports
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if ports.is_empty() {
        service.service_type.clone()
    } else {
        format!("{} · {}", service.service_type, ports.join(", "))
    }
}

/// The `Services` section: the services of the pod's namespace that select it, from the
/// drawer-scoped services watch.
fn services_section(
    pod: &PodSummary,
    live: Option<&LiveCluster>,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let note = |text: &str| {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.to_owned())
            .into_any_element()
    };
    let subject = key_related_subject(&ResourceKey::of_pod(pod));
    let body = match (live, subject) {
        (Some(live), Some(subject)) => {
            if let Some(check) = denied_related_check(&subject, &live.access) {
                vec![note(&format!("Not permitted: {check}"))]
            } else {
                match live.related_of(&subject).and_then(RelatedList::services) {
                    None | Some(LiveList::Loading) => vec![note("Loading services…")],
                    Some(LiveList::Failed { message }) => vec![
                        note("Services are unavailable"),
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(message.clone())
                            .into_any_element(),
                    ],
                    Some(LiveList::Ready { items, .. }) => service_rows(pod, items, cx)
                        .unwrap_or_else(|| vec![note("No service selects this pod")]),
                }
            }
        }
        _ => vec![note("Loading services…")],
    };
    std::iter::once(section_title("Services", cx).into_any_element())
        .chain(body)
        .collect()
}

/// One row per service that selects the pod, capped; `None` when no service does.
fn service_rows(
    pod: &PodSummary,
    services: &[ServiceSummary],
    cx: &Context<AppShell>,
) -> Option<Vec<AnyElement>> {
    let selecting = services_selecting(pod, services);
    if selecting.is_empty() {
        return None;
    }
    let hidden = selecting.len().saturating_sub(MAX_LISTED_SERVICES);
    let rows = selecting
        .iter()
        .take(MAX_LISTED_SERVICES)
        .enumerate()
        .map(|(index, service)| {
            let name = SharedString::from(service.name.clone());
            let target = ResourceKey::of_object("Service", Some(&service.namespace), &service.name);
            let link = match target {
                Some(target) => link_text(SERVICE_LINK_ID_BASE + index, &name, target, cx),
                None => div().truncate().child(name).into_any_element(),
            };
            h_flex()
                .gap_2()
                .py_1()
                .text_sm()
                .child(link)
                .child(
                    div()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(service_summary_text(service)),
                )
                .into_any_element()
        })
        .chain((hidden > 0).then(|| {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format!("+{hidden} more"))
                .into_any_element()
        }));
    Some(rows.collect())
}

/// One volume that a container of the pod mounts, as the Volumes section lists it.
#[derive(Debug, PartialEq, Eq)]
struct VolumeRow {
    name: String,
    /// `secret/y`, `pvc/z`, `hostPath /p`, ...
    source: String,
    /// The screen of the source; `None` for volume kinds without one.
    target: Option<ResourceKey>,
}

/// The volumes the containers mount, in first-seen order and once per volume name. `PodSummary`
/// has no `spec.volumes`, so a volume that no container mounts is only in the YAML tab.
fn volume_rows(namespace: &str, containers: &[ContainerSummary]) -> Vec<VolumeRow> {
    let mut rows: Vec<VolumeRow> = Vec::new();
    for mount in containers.iter().flat_map(|container| &container.mounts) {
        if rows.iter().all(|row| row.name != mount.volume) {
            rows.push(VolumeRow {
                name: mount.volume.clone(),
                source: volume_source_text(&mount.source),
                target: volume_target(&mount.source, namespace),
            });
        }
    }
    rows
}

fn volume_source_text(source: &VolumeSource) -> String {
    match source {
        VolumeSource::ConfigMap { name } => format!("configmap/{name}"),
        VolumeSource::Secret { name } => format!("secret/{name}"),
        VolumeSource::PersistentVolumeClaim { claim } => format!("pvc/{claim}"),
        VolumeSource::EmptyDir => "emptyDir".to_owned(),
        VolumeSource::HostPath { path } => format!("hostPath {path}"),
        VolumeSource::Projected {
            config_maps,
            secrets,
        } => {
            let names = config_maps
                .iter()
                .map(|name| format!("configmap/{name}"))
                .chain(secrets.iter().map(|name| format!("secret/{name}")))
                .collect::<Vec<_>>();
            if names.is_empty() {
                "projected".to_owned()
            } else {
                format!("projected · {}", names.join(", "))
            }
        }
        VolumeSource::DownwardApi => "downwardAPI".to_owned(),
        VolumeSource::Other => "volume".to_owned(),
    }
}

/// The `Volumes` section: one row per mounted volume, its source a link when it has a screen.
fn volumes_section(pod: &PodSummary, cx: &Context<AppShell>) -> Vec<AnyElement> {
    let rows = volume_rows(&pod.namespace, &pod.containers);
    let body = if rows.is_empty() {
        vec![
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No container mounts a volume; the YAML tab lists the others")
                .into_any_element(),
        ]
    } else {
        rows.into_iter()
            .enumerate()
            .map(|(index, row)| {
                let value = match row.target {
                    Some(target) => {
                        link_text(VOLUME_LINK_ID_BASE + index, &row.source.into(), target, cx)
                    }
                    None => div().truncate().child(row.source).into_any_element(),
                };
                detail_row(row.name, value, cx).into_any_element()
            })
            .collect()
    };
    std::iter::once(section_title("Volumes", cx).into_any_element())
        .chain(body)
        .collect()
}

/// `{reason}: {message}`, either part may be missing; `None` when both are.
fn condition_tooltip(condition: &PodCondition) -> Option<String> {
    match (&condition.reason, &condition.message) {
        (Some(reason), Some(message)) => Some(format!("{reason}: {message}")),
        (Some(text), None) | (None, Some(text)) => Some(text.clone()),
        (None, None) => None,
    }
}

fn conditions(pod: &PodSummary, cx: &App) -> AnyElement {
    if pod.conditions.is_empty() {
        return div().p_4().child(absent_text(cx)).into_any_element();
    }
    let theme = cx.theme();
    h_flex()
        .flex_wrap()
        .gap_2()
        .children(pod.conditions.iter().enumerate().map(|(index, condition)| {
            let dot_color = if condition.is_true {
                theme.success
            } else {
                theme.muted_foreground
            };
            let tooltip = (!condition.is_true)
                .then(|| condition_tooltip(condition))
                .flatten();
            h_flex()
                .id(("condition", index))
                .gap_1p5()
                .items_center()
                .px_2()
                .py_0p5()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .text_xs()
                .child(div().size_2().rounded_full().bg(dot_color))
                .child(condition.name.clone())
                .when_some(tooltip, |this, text| {
                    this.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                })
        }))
        .into_any_element()
}

/// "3 restarts" in the warning tone; `None` for a container that never restarted.
fn restart_label(count: u32) -> Option<StatusLabel> {
    (count > 0).then(|| StatusLabel {
        text: format!("{count} restart{}", if count == 1 { "" } else { "s" }).into(),
        tone: StatusTone::Warn,
    })
}

/// One container of the Overview tab; clicking it opens the Containers tab on it. A container that
/// ran before shows how its last run ended, so a crash loop that is quiet right now still shows.
fn container_row(
    index: usize,
    container: &ContainerSummary,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let name = container.name.clone();
    let mono = theme.mono_font_family.clone();
    let hover_bg = theme.secondary_hover;
    let last_exit = container.last_termination.as_ref().map(|termination| {
        div()
            .pl_2()
            .pb_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .truncate()
            .child(format!("Last exit: {}", last_state_text(termination, now)))
    });
    v_flex()
        .id(("container-row", index))
        .px_2()
        .rounded(theme.radius)
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, _, cx| shell.open_container(name.clone(), cx)))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .py_1()
                .text_sm()
                .child(kind_tag(container.kind, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(mono)
                        .child(container.name.clone()),
                )
                .child(toned_text(container_state_label(container), cx))
                .children(
                    restart_label(container.restart_count)
                        .map(|label| div().text_xs().child(toned_text(label, cx))),
                ),
        )
        .children(last_exit)
        .into_any_element()
}

pub(crate) fn kind_tag(kind: ContainerKind, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .px_1()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .text_color(theme.muted_foreground)
        .text_xs()
        .child(kind_tag_text(kind))
        .into_any_element()
}

pub(crate) fn kind_tag_text(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::Init => "INIT",
        ContainerKind::Sidecar => "SIDECAR",
        ContainerKind::Main => "MAIN",
    }
}

/// The first main container that is not ready, else the first main one, else the first.
pub(crate) fn default_container(containers: &[ContainerSummary]) -> Option<usize> {
    containers
        .iter()
        .position(|container| container.kind == ContainerKind::Main && !container.is_ready)
        .or_else(|| {
            containers
                .iter()
                .position(|container| container.kind == ContainerKind::Main)
        })
        .or_else(|| (!containers.is_empty()).then_some(0))
}

/// How a container's `cpu` or `memory` row reads once usage is known.
#[derive(Debug, PartialEq)]
pub(crate) struct UsageRow {
    pub(crate) value: String,
    pub(crate) tone: Option<StatusTone>,
    pub(crate) bar: Option<UsageBar>,
    pub(crate) note: Option<String>,
}

/// `None` when the row keeps its request and limit text: no usage, or not `cpu` or `memory`.
pub(crate) fn container_usage_row(
    resource: &ContainerResource,
    usage: Option<ResourceUsage>,
) -> Option<UsageRow> {
    let usage = usage?;
    let (measure, used) = match resource.name.as_str() {
        "cpu" => (Measure::Cpu, usage.cpu.cores()),
        "memory" => (Measure::Bytes, usage.memory.bytes() as f64),
        _ => return None,
    };
    let quantity = |text: &Option<String>| {
        let text = text.as_deref()?;
        match measure {
            Measure::Cpu => CpuAmount::parse(text).map(CpuAmount::cores),
            Measure::Bytes => ByteAmount::parse(text).map(|bytes| bytes.bytes() as f64),
            // A rate has no Kubernetes quantity; only cpu and memory reach here.
            Measure::Rate => None,
        }
    };
    let request = quantity(&resource.request);
    // A zero limit has no ratio, so it reads like no limit.
    let limit = quantity(&resource.limit).filter(|limit| *limit > 0.);
    let Some(limit) = limit else {
        let request = request.map_or_else(
            || "no request".to_owned(),
            |request| format!("request {}", measure.format(request)),
        );
        return Some(UsageRow {
            value: format!("{} used", measure.format(used)),
            tone: None,
            bar: None,
            note: Some(format!("{request} · no limit")),
        });
    };
    let ratio = used / limit;
    Some(UsageRow {
        value: measure.format_pair(used, limit, " of "),
        tone: usage_tone(ratio),
        bar: Some(UsageBar::of_ratio(
            ratio,
            request.map(|request| request / limit),
        )),
        note: request.map(|request| format!("request {}", measure.format(request))),
    })
}

/// The index of the container the Containers tab shows: the clicked one, else the default.
pub(crate) fn selected_container_index(pod: &PodSummary, state: &DrawerState) -> Option<usize> {
    state
        .selected_container
        .as_deref()
        .and_then(|name| {
            pod.containers
                .iter()
                .position(|container| container.name == name)
        })
        .or_else(|| default_container(&pod.containers))
}

fn containers_tab(
    pod: &PodSummary,
    state: &DrawerState,
    events: Option<&[EventSummary]>,
    forward: &PortButtons<'_>,
    links: ContainerMenuLinks<'_>,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let Some(selected) = selected_container_index(pod, state) else {
        return div().p_4().child(absent_text(cx)).into_any_element();
    };
    let list = container_list(&pod.containers, selected, cx);
    let live = links.session.read(cx).live();
    let menu = live.map(|_| container_menu_button(pod, &pod.containers[selected].name, links, cx));
    let detail = container_detail(
        &ContainerDetailInput {
            pod,
            container: &pod.containers[selected],
            tab: state.container_tab,
            events,
            logs_reason: view_logs_reason(live),
            forward,
            usage: live.and_then(|live| {
                let name = &pod.containers[selected].name;
                live.metrics
                    .pods
                    .history
                    .latest_container(&pod.namespace, &pod.name, name)
            }),
            kubelet: live.map(|live| &live.metrics.kubelet.history),
            monitor: live.map(|live| MonitorView::of_container(state, live)),
            now,
        },
        menu,
        cx,
    );
    // Each side scrolls on its own, so the sidebar's border runs the full height of the tab.
    h_flex()
        .size_full()
        .child(
            div()
                .id("container-list")
                .w(container_list_width(&pod.containers))
                .flex_shrink_0()
                .h_full()
                .overflow_y_scroll()
                .border_r_1()
                .border_color(cx.theme().border)
                .child(div().p_4().child(list)),
        )
        .child(
            div()
                .id("container-detail")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .child(div().p_4().child(detail)),
        )
        .into_any_element()
}

/// What the container ⋯ menu reads when it opens.
#[derive(Clone, Copy)]
struct ContainerMenuLinks<'a> {
    session: &'a Entity<ClusterSession>,
    row: &'a RowContext,
    dock: &'a WeakEntity<Dock>,
}

/// The ⋯ button of the container header. Like the pod menu it reads the session when it opens,
/// so it shows the access state and the container of that moment.
fn container_menu_button(
    pod: &PodSummary,
    container: &str,
    links: ContainerMenuLinks<'_>,
    cx: &Context<AppShell>,
) -> AnyElement {
    // Weak: a rendered menu closure must not keep a session alive after a cluster switch.
    let session = links.session.downgrade();
    let row = links.row.clone();
    let dock = links.dock.clone();
    let shell = cx.weak_entity();
    let key = ResourceKey::of_pod(pod);
    let container = container.to_owned();
    Button::new("container-menu")
        .ghost()
        .small()
        .icon(Icon::new(IconName::Ellipsis))
        .dropdown_menu(move |menu, _, cx| {
            let Some(session) = session.upgrade() else {
                return menu;
            };
            let session = session.read(cx);
            let (Some(live), Some(guard)) = (session.live(), session.guard(cx)) else {
                return menu;
            };
            let Some(pod) = live.pods.items().iter().find(|pod| key.is_pod(pod)) else {
                return menu;
            };
            let Some(container) = pod.containers.iter().find(|c| c.name == container) else {
                return menu;
            };
            let links = PodMenuLinks {
                dock: &dock,
                shell: &shell,
            };
            container_menu(menu, pod, container, live, &guard, &row, &links)
        })
        .into_any_element()
}

/// The groups of the Containers list, in the order they are shown.
const CONTAINER_GROUPS: [ContainerKind; 3] = [
    ContainerKind::Init,
    ContainerKind::Sidecar,
    ContainerKind::Main,
];

/// The indices of `containers` in the order the Containers list shows them: Init, Sidecar, then
/// Main, in spec order inside each group. `[` and `]` step through the same order.
pub(crate) fn container_display_order(containers: &[ContainerSummary]) -> Vec<usize> {
    CONTAINER_GROUPS
        .into_iter()
        .flat_map(|kind| {
            containers
                .iter()
                .enumerate()
                .filter(move |(_, container)| container.kind == kind)
                .map(|(index, _)| index)
        })
        .collect()
}

fn container_list(
    containers: &[ContainerSummary],
    selected: usize,
    cx: &Context<AppShell>,
) -> AnyElement {
    let order = container_display_order(containers);
    v_flex()
        .children(CONTAINER_GROUPS.into_iter().filter_map(|kind| {
            let members: Vec<(usize, &ContainerSummary)> = order
                .iter()
                .map(|&index| (index, &containers[index]))
                .filter(|(_, container)| container.kind == kind)
                .collect();
            if members.is_empty() {
                return None;
            }
            let refs: Vec<&ContainerSummary> =
                members.iter().map(|(_, container)| *container).collect();
            let title = group_title(kind, &refs);
            let theme = cx.theme();
            Some(
                v_flex()
                    .child(
                        div()
                            .py_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(title),
                    )
                    .children(members.into_iter().map(|(index, container)| {
                        list_item(index, container, index == selected, cx)
                    })),
            )
        }))
        .into_any_element()
}

fn group_title(kind: ContainerKind, members: &[&ContainerSummary]) -> String {
    let total = members.len();
    match kind {
        ContainerKind::Init => {
            let done = members
                .iter()
                .filter(|container| {
                    matches!(&container.state, ContainerState::Terminated(termination)
                        if termination.exit_code == 0)
                })
                .count();
            format!("Init · ran in order {done}/{total}")
        }
        ContainerKind::Sidecar => format!("Sidecars {total}"),
        ContainerKind::Main => {
            let ready = members
                .iter()
                .filter(|container| container.is_ready)
                .count();
            format!("Containers {ready}/{total}")
        }
    }
}

fn list_item(
    index: usize,
    container: &ContainerSummary,
    is_selected: bool,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let name = container.name.clone();
    // The same colour the tables use for their selected row.
    let selected_bg = theme.table_active;
    let hover_bg = theme.secondary_hover;
    let mono = theme.mono_font_family.clone();
    h_flex()
        .id(("container-item", index))
        .gap_2()
        .items_center()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .when(is_selected, move |this| this.bg(selected_bg))
        .when(!is_selected, move |this| {
            this.hover(move |s| s.bg(hover_bg))
        })
        .on_click(cx.listener(move |shell, _, _, cx| shell.select_container(name.clone(), cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(mono)
                .child(container.name.clone()),
        )
        .children(restart_label(container.restart_count).map(|label| {
            // Only the number: the 240 px list has no room for the word next to the name.
            let tooltip = label.text.clone();
            div()
                .id(("container-restarts", index))
                .text_xs()
                .child(toned_text(
                    StatusLabel {
                        text: container.restart_count.to_string().into(),
                        tone: label.tone,
                    },
                    cx,
                ))
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        }))
        .child(toned_text(container_state_label(container), cx))
        .into_any_element()
}

#[cfg(test)]
#[path = "pod_drawer_tests.rs"]
mod pod_drawer_tests;
