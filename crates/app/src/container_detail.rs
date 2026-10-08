//! The container detail of the pod drawer's Containers tab: a header, the Info, Env, and
//! Mounts sub-tabs, and the text builders behind them.
//!
//! Secret safety: the cluster crate keeps only names and sources of env vars, so nothing here
//! can show a value; a literal env var points to the YAML tab instead.

use std::borrow::Cow;

use cluster::{
    ContainerResource, ContainerState, ContainerSummary, EnvFromSource, EnvSource, EventSummary,
    PodSummary, ProbeAction, ProbeSummary, PvcUsage, ResourceUsage, Termination, VolumeSource,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, relative,
};

use crate::age::{format_age, format_countdown};
use crate::app_shell::AppShell;
use crate::clipboard_copy::copyable_mono;
use crate::drawer::{
    ContainerTab, LABEL_WIDTH, absent_text, chips, detail_row, link_text, section_title,
    truncated_text, value_or_absent,
};
use crate::kubelet_history::KubeletHistory;
use crate::monitor_tab::{MonitorView, monitor_tab};
use crate::pod_diagnosis::{ProbeKind, ProbeResult, next_retry, probe_of, probe_result};
use crate::pod_drawer::{UsageRow, container_usage_row, kind_tag};
use crate::port_forward_menu::{PortButtons, PortChoice, pod_drawer_subject};
use crate::status_tone::{StatusLabel, StatusTone, container_state_label, tone_color, toned_text};
use crate::table_selection::ResourceKey;
use crate::usage_bar::usage_bar;
use crate::usage_format::{Measure, format_percent, usage_tone};

/// How many distinct env sources `env_summary` names before `+N more`.
const MAX_SUMMARY_SOURCES: usize = 3;
const PROBE_KINDS: [ProbeKind; 3] = [
    ProbeKind::Liveness,
    ProbeKind::Readiness,
    ProbeKind::Startup,
];
/// The share of an Env or Mounts row that the name column takes; names are longer than sources
/// are informative, so they get 3 parts to the source's 2.
const SOURCE_NAME_SHARE: f32 = 0.6;
const CONTAINER_TABS: [ContainerTab; 5] = [
    ContainerTab::Info,
    ContainerTab::Env,
    ContainerTab::Mounts,
    ContainerTab::Logs,
    ContainerTab::Monitor,
];
/// Element ids of the links inside the container detail; the drawer's kind links use small ids.
const LINK_ID_BASE: usize = 1_000;

/// One Env or Mounts row: what it is, where it comes from, and the screen of the source when
/// k8sBoard has one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceRow {
    name: String,
    source: String,
    target: Option<ResourceKey>,
    /// A second, muted line under the source: the usage of a PVC mount.
    usage: Option<UsageNote>,
}

/// `83 of 100Gi used (83%)` and its tone (Warn from 80 %, Bad from 90 %).
#[derive(Clone, Debug, PartialEq, Eq)]
struct UsageNote {
    text: String,
    tone: Option<StatusTone>,
}

/// What the container detail needs from the open pod.
pub(crate) struct ContainerDetailInput<'a> {
    pub(crate) pod: &'a PodSummary,
    pub(crate) container: &'a ContainerSummary,
    pub(crate) tab: ContainerTab,
    /// The pod's object events when they are loaded.
    pub(crate) events: Option<&'a [EventSummary]>,
    /// What the Forward buttons of the ports read: the pod's own cluster.
    pub(crate) forward: &'a PortButtons<'a>,
    /// Why the Logs sub-tab cannot open the dock, or `None` when it can.
    pub(crate) logs_reason: Option<SharedString>,
    /// The container's newest usage; `None` without a sample.
    pub(crate) usage: Option<ResourceUsage>,
    /// The kubelet history, for the usage of PVC mounts; `None` while the session is not live.
    pub(crate) kubelet: Option<&'a KubeletHistory>,
    /// The Monitor sub-tab; `None` while the session is not live.
    pub(crate) monitor: Option<MonitorView<'a>>,
    pub(crate) now: jiff::Timestamp,
}

/// `menu` is the ⋯ button at the right of the header; `None` while the session is not live.
pub(crate) fn container_detail(
    input: &ContainerDetailInput<'_>,
    menu: Option<AnyElement>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let container = input.container;
    let label = container_state_label(container);
    let tone = label.tone;
    let retry = next_retry(container, input.now)
        .map(|remaining| format!("next retry in {}", format_countdown(remaining.as_secs())));
    let header = h_flex()
        .gap_2()
        .items_center()
        .pb_2()
        .child(
            div()
                .font_semibold()
                .font_family(cx.theme().mono_font_family.clone())
                .child(container.name.clone()),
        )
        .child(div().text_color(tone_color(tone, cx)).child(label.text))
        .child(kind_tag(container.kind, cx))
        .children(retry.map(|text| {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text)
        }))
        .children(menu.map(|menu| div().ml_auto().child(menu)));
    let body = match input.tab {
        ContainerTab::Info => info_body(input, cx),
        ContainerTab::Env => source_body(
            &env_rows(container, &input.pod.namespace),
            "No environment variables",
            cx,
        ),
        ContainerTab::Mounts => source_body(
            &mount_rows(container, &input.pod.namespace, input.kubelet),
            "No mounts",
            cx,
        ),
        ContainerTab::Logs => logs_body(input, cx),
        ContainerTab::Monitor => match &input.monitor {
            Some(view) => monitor_tab(view, cx),
            None => div().into_any_element(),
        },
    };
    v_flex()
        .child(header)
        .child(sub_tab_bar(container, input.tab, cx))
        .child(body)
        .into_any_element()
}

fn sub_tab_bar(
    container: &ContainerSummary,
    shown: ContainerTab,
    cx: &Context<AppShell>,
) -> AnyElement {
    let selected_index = CONTAINER_TABS
        .iter()
        .position(|tab| *tab == shown)
        .unwrap_or(0);
    let title = |tab: ContainerTab| match tab {
        ContainerTab::Info => "Info".to_owned(),
        ContainerTab::Env => format!("Env {}", container.env.len() + container.env_from.len()),
        ContainerTab::Mounts => format!("Mounts {}", container.mounts.len()),
        ContainerTab::Logs => "Logs".to_owned(),
        ContainerTab::Monitor => "Monitor".to_owned(),
    };
    div()
        .pb_2()
        .child(
            TabBar::new("container-tabs")
                .segmented()
                .xsmall()
                .selected_index(selected_index)
                .on_click(cx.listener(move |shell, index: &usize, window, cx| {
                    if let Some(&tab) = CONTAINER_TABS.get(*index) {
                        shell.set_container_tab(tab, cx);
                        // The dock sits right below the drawer, so this click is the explicit
                        // action that opens the stream; navigation alone never does.
                        if tab == ContainerTab::Logs {
                            shell.open_container_logs(window, cx);
                        }
                    }
                }))
                .children(
                    CONTAINER_TABS
                        .into_iter()
                        .map(|tab| Tab::new().label(title(tab))),
                ),
        )
        .into_any_element()
}

fn info_body(input: &ContainerDetailInput<'_>, cx: &Context<AppShell>) -> AnyElement {
    let container = input.container;
    let now = input.now;
    let mono = cx.theme().mono_font_family.clone();
    let label = container_state_label(container);
    let state = StatusLabel {
        text: state_text(container, &label, now).into(),
        tone: label.tone,
    };
    let last_state = container
        .last_termination
        .as_ref()
        .map(|termination| last_state_text(termination, now));
    let last_run = container
        .last_termination
        .as_ref()
        .and_then(|termination| last_run_text(termination, now));

    let mut column = v_flex()
        .child(section_title("State", cx))
        .child(detail_row("State", toned_text(state, cx), cx))
        .child(detail_row(
            "Last state",
            value_or_absent(last_state.as_deref(), cx),
            cx,
        ))
        .children(last_run.map(|text| detail_row("Last run", div().truncate().child(text), cx)))
        .child(detail_row(
            "Restarts",
            div()
                .font_family(mono.clone())
                .child(container.restart_count.to_string()),
            cx,
        ))
        .child(section_title("Image", cx))
        .child(detail_row(
            "Image",
            copyable_mono("container-image", container.image.clone(), cx),
            cx,
        ))
        .child(detail_row(
            "Digest",
            match &container.image_digest {
                Some(digest) => copyable_mono("container-digest", digest.clone(), cx),
                None => absent_text(cx).into_any_element(),
            },
            cx,
        ))
        .child(detail_row(
            "Pull policy",
            value_or_absent(container.pull_policy.as_deref(), cx),
            cx,
        ))
        .child(section_title("Ports", cx));
    if container.ports.is_empty() {
        column = column.child(absent_text(cx));
    }
    let forward_subject = pod_drawer_subject(input.pod);
    for (index, port) in container.ports.iter().enumerate() {
        let mut text = format!("{}/{}", port.port, port.protocol);
        if let Some(name) = &port.name {
            text.push_str(&format!(" · {name}"));
        }
        let choice = PortChoice {
            label: text.clone(),
            remote_port: port.port,
            is_tcp: port.protocol.eq_ignore_ascii_case("TCP"),
        };
        column = column.child(input.forward.row(
            &text.into(),
            LINK_ID_BASE + index,
            &forward_subject,
            &choice,
            cx,
        ));
    }

    column = column.child(section_title("Resources", cx));
    let resources = resource_rows(container, input.usage);
    if resources.is_empty() {
        column = column.child(muted_note("No requests or limits", cx));
    }
    for resource in resources.iter() {
        let label = resource_label(&resource.name);
        column = match container_usage_row(resource, input.usage) {
            Some(row) => column.child(detail_row(label, usage_value(row, cx), cx)),
            None => column.child(detail_row(
                label,
                truncated_text(
                    SharedString::from(format!("resource-{}", resource.name)),
                    resource_text(resource),
                ),
                cx,
            )),
        };
    }

    column = column.child(section_title("Probes", cx));
    for kind in PROBE_KINDS {
        column = column.child(probe_row(input, kind, cx));
    }

    let env = env_summary(container);
    let mounts = mount_summary(container);
    if env.is_some() || mounts.is_some() {
        column = column.child(section_title("Env & mounts", cx));
    }
    if let Some(text) = env {
        column = column.child(summary_row(
            "env-summary",
            text,
            "Env →",
            ContainerTab::Env,
            cx,
        ));
    }
    if let Some(text) = mounts {
        column = column.child(summary_row(
            "mount-summary",
            text,
            "Mounts →",
            ContainerTab::Mounts,
            cx,
        ));
    }
    column.into_any_element()
}

/// The Logs sub-tab never streams by itself: it names the container and offers the button that
/// opens or focuses the dock tab.
fn logs_body(input: &ContainerDetailInput<'_>, cx: &Context<AppShell>) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    if let Some(reason) = &input.logs_reason {
        return div()
            .text_sm()
            .text_color(muted)
            .child(reason.clone())
            .into_any_element();
    }
    v_flex()
        .gap_2()
        .items_start()
        .child(div().text_sm().text_color(muted).child(format!(
            "Logs of {} open in the dock below.",
            input.container.name
        )))
        .child(
            Button::new("container-logs-show")
                .ghost()
                .small()
                .icon(Icon::new(IconName::FileText))
                .label("Show in dock")
                .on_click(
                    cx.listener(|shell, _, window, cx| shell.open_container_logs(window, cx)),
                ),
        )
        .into_any_element()
}

fn muted_note(text: &'static str, cx: &App) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn probe_row(
    input: &ContainerDetailInput<'_>,
    kind: ProbeKind,
    cx: &Context<AppShell>,
) -> AnyElement {
    let terms = probe_of(input.container, kind)
        .map(probe_chips)
        .unwrap_or_default();
    let result = probe_result(input.pod, input.container, kind, input.events);
    let result_label = probe_result_label(result);
    h_flex()
        .w_full()
        .gap_3()
        .py_1()
        .items_start()
        .text_sm()
        .child(
            div()
                .w(LABEL_WIDTH)
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(kind.name()),
        )
        .child(div().flex_1().min_w_0().overflow_hidden().child(chips(
            SharedString::from(format!("probe-{}", kind.name())),
            &terms,
            cx,
        )))
        .child(
            toned_text(result_label, cx)
                .flex_shrink_0()
                .whitespace_nowrap(),
        )
        .into_any_element()
}

/// A summary line with the link that opens the sub-tab behind it.
fn summary_row(
    id: &'static str,
    text: String,
    link: &'static str,
    tab: ContainerTab,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .gap_3()
        .py_1()
        .items_start()
        .text_sm()
        .child(
            truncated_text(id, text)
                .flex_1()
                .min_w_0()
                .text_color(theme.muted_foreground),
        )
        .child(
            div()
                .id(link)
                .flex_shrink_0()
                .cursor_pointer()
                .text_color(theme.link)
                .underline()
                .on_click(cx.listener(move |shell, _, _, cx| shell.set_container_tab(tab, cx)))
                .child(link),
        )
        .into_any_element()
}

fn source_body(rows: &[SourceRow], empty_text: &'static str, cx: &Context<AppShell>) -> AnyElement {
    if rows.is_empty() {
        return muted_note(empty_text, cx);
    }
    let theme = cx.theme();
    v_flex()
        .children(rows.iter().enumerate().map(|(index, row)| {
            let source = match &row.target {
                Some(target) => link_text(
                    LINK_ID_BASE + index,
                    &SharedString::from(row.source.clone()),
                    target.clone(),
                    cx,
                )
                .into_any_element(),
                None => truncated_text(
                    ("source-text", index),
                    SharedString::from(row.source.clone()),
                )
                .text_color(theme.muted_foreground)
                .into_any_element(),
            };
            let usage = row.usage.as_ref().map(|usage| {
                let color = usage
                    .tone
                    .map_or(theme.muted_foreground, |tone| tone_color(tone, cx));
                // Not truncated: it is short, and the percentage is its point.
                div()
                    .text_color(color)
                    .text_xs()
                    .child(SharedString::from(usage.text.clone()))
            });
            h_flex()
                .gap_3()
                .py_1()
                .items_start()
                .text_sm()
                .child(
                    truncated_text(("source-name", index), SharedString::from(row.name.clone()))
                        .w(gpui_kit::relative(SOURCE_NAME_SHARE))
                        .flex_shrink_0()
                        .font_family(theme.mono_font_family.clone()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(source)
                        .children(usage),
                )
        }))
        .into_any_element()
}

/// The state label, plus how long a running container has been up.
fn state_text(container: &ContainerSummary, label: &StatusLabel, now: jiff::Timestamp) -> String {
    match &container.state {
        ContainerState::Running {
            started_at: Some(started_at),
        } => format!(
            "{} · started {} ago",
            label.text,
            format_age(Some(*started_at), now)
        ),
        _ => label.text.to_string(),
    }
}

pub(crate) fn last_state_text(termination: &Termination, now: jiff::Timestamp) -> String {
    let reason = termination
        .reason
        .as_ref()
        .map_or_else(|| "Terminated".to_owned(), ToString::to_string);
    let mut text = format!("{reason} · exit {}", termination.exit_code);
    if let Some(signal) = termination.signal {
        text.push_str(&format!(" · signal {signal}"));
    }
    if let Some(finished_at) = termination.finished_at {
        text.push_str(&format!(
            " · ended {} ago",
            format_age(Some(finished_at), now)
        ));
    }
    text
}

/// `ran 4m, ended 2m ago`; needs both timestamps.
fn last_run_text(termination: &Termination, now: jiff::Timestamp) -> Option<String> {
    let (started_at, finished_at) = (termination.started_at?, termination.finished_at?);
    Some(format!(
        "ran {}, ended {} ago",
        format_age(Some(started_at), finished_at),
        format_age(Some(finished_at), now)
    ))
}

pub(crate) fn resource_label(name: &str) -> String {
    match name {
        "cpu" => "CPU".to_owned(),
        "memory" => "Memory".to_owned(),
        "ephemeral-storage" => "Ephemeral storage".to_owned(),
        "pods" => "Pods".to_owned(),
        other => other
            .strip_prefix("hugepages-")
            .map_or_else(|| other.to_owned(), |size| format!("Hugepages {size}")),
    }
}

/// The container's resources. With usage, a `cpu` or `memory` the container has no request or limit
/// for (a BestEffort container) is added as an empty row, so its usage still shows (`no request ·
/// no limit`); only then are the rows copied and ordered `cpu`, `memory`, the rest.
fn resource_rows(
    container: &ContainerSummary,
    usage: Option<ResourceUsage>,
) -> Cow<'_, [ContainerResource]> {
    let missing: Vec<&str> = ["cpu", "memory"]
        .into_iter()
        .filter(|name| container.resources.iter().all(|row| row.name != *name))
        .collect();
    if usage.is_none() || missing.is_empty() {
        return Cow::Borrowed(&container.resources);
    }
    let mut rows = container.resources.clone();
    rows.extend(missing.into_iter().map(|name| ContainerResource {
        name: name.to_owned(),
        request: None,
        limit: None,
    }));
    rows.sort_by_key(|row| match row.name.as_str() {
        "cpu" => 0,
        "memory" => 1,
        _ => 2,
    });
    Cow::Owned(rows)
}

/// The usage of one resource: the text in its tone, the bar when there is a limit, and the
/// request below.
fn usage_value(row: UsageRow, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let color = row
        .tone
        .map_or(theme.foreground, |tone| tone_color(tone, cx));
    v_flex()
        .w_full()
        .gap_1()
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_color(color)
                .child(row.value),
        )
        .children(row.bar.map(|bar| usage_bar(bar, relative(1.), cx)))
        .children(row.note.map(|note| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(note)
        }))
        .into_any_element()
}

/// `request 250m · limit 1`, `request 250m · no limit`, `no request · limit 512Mi`.
fn resource_text(resource: &ContainerResource) -> String {
    let request = resource.request.as_ref().map_or_else(
        || "no request".to_owned(),
        |value| format!("request {value}"),
    );
    let limit = resource
        .limit
        .as_ref()
        .map_or_else(|| "no limit".to_owned(), |value| format!("limit {value}"));
    format!("{request} · {limit}")
}

/// `HTTP GET :8080/ready · every 5s`.
pub(crate) fn probe_summary_text(probe: &ProbeSummary) -> String {
    let action = match &probe.action {
        ProbeAction::HttpGet { scheme, port, path } => format!("{scheme} GET :{port}{path}"),
        ProbeAction::TcpSocket { port } => format!("TCP :{port}"),
        ProbeAction::Grpc { port } => format!("gRPC :{port}"),
        ProbeAction::Exec { command } => exec_text(command),
        ProbeAction::Unknown => "unknown action".to_owned(),
    };
    format!("{action} · every {}s", probe.period_seconds)
}

/// The longest exec argv the probe text quotes.
const EXEC_TEXT_CHARS: usize = 100;

/// ``exec `test -f /tmp/ready` ``: the argv joined by spaces and cut, or `exec command` for none.
fn exec_text(command: &[String]) -> String {
    if command.is_empty() {
        return "exec command".to_owned();
    }
    format!("exec `{}`", exec_command(command))
}

/// The argv joined by spaces and cut at `EXEC_TEXT_CHARS`; `command` for none.
fn exec_command(command: &[String]) -> String {
    if command.is_empty() {
        return "command".to_owned();
    }
    let joined = command.join(" ");
    let mut cut: String = joined.chars().take(EXEC_TEXT_CHARS).collect();
    if cut.len() < joined.len() {
        cut.push('…');
    }
    cut
}

/// The probe's facts, one chip each: protocol, method, port, path or command, then the timings.
pub(crate) fn probe_chips(probe: &ProbeSummary) -> Vec<SharedString> {
    let mut terms: Vec<String> = match &probe.action {
        ProbeAction::HttpGet { scheme, port, path } => vec![
            scheme.clone(),
            "GET".to_owned(),
            format!("port {port}"),
            format!("path {path}"),
        ],
        ProbeAction::TcpSocket { port } => vec!["TCP".to_owned(), format!("port {port}")],
        ProbeAction::Grpc { port } => vec!["gRPC".to_owned(), format!("port {port}")],
        ProbeAction::Exec { command } => vec!["exec".to_owned(), exec_command(command)],
        ProbeAction::Unknown => vec!["unknown action".to_owned()],
    };
    terms.extend([
        format!("delay {}s", probe.initial_delay_seconds),
        format!("timeout {}s", probe.timeout_seconds),
        format!("period {}s", probe.period_seconds),
        format!("failures {}", probe.failure_threshold),
    ]);
    terms.into_iter().map(SharedString::from).collect()
}

/// The right-hand text of a probe row; muted results use the `Done` tone, which is the muted
/// foreground.
fn probe_result_label(result: ProbeResult) -> StatusLabel {
    let (text, tone) = match result {
        ProbeResult::NotSet => ("not set".to_owned(), StatusTone::Done),
        ProbeResult::Inactive => ("—".to_owned(), StatusTone::Done),
        ProbeResult::WaitingForStartup => ("Waiting for startup".to_owned(), StatusTone::Done),
        ProbeResult::Pending => ("Not passed yet".to_owned(), StatusTone::Warn),
        ProbeResult::Passed => ("Passed".to_owned(), StatusTone::Ok),
        ProbeResult::Passing => ("Passing".to_owned(), StatusTone::Ok),
        ProbeResult::Failing { failures: 0 } => ("Failing".to_owned(), StatusTone::Bad),
        ProbeResult::Failing { failures } => (format!("Failing · ×{failures}"), StatusTone::Bad),
        ProbeResult::NoData => ("No data".to_owned(), StatusTone::Done),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

/// `14 env vars · 6 from configmap/api-config · all of secret/extra`; at most three sources,
/// then `+N more`. `None` when the container has no env at all.
fn env_summary(container: &ContainerSummary) -> Option<String> {
    if container.env.is_empty() && container.env_from.is_empty() {
        return None;
    }
    // First-seen order, with the number of variables each source supplies.
    let mut sources: Vec<(String, usize)> = Vec::new();
    for entry in &container.env {
        let name = match &entry.source {
            EnvSource::ConfigMapKey { name, .. } => format!("configmap/{name}"),
            EnvSource::SecretKey { name, .. } => format!("secret/{name}"),
            _ => continue,
        };
        match sources.iter_mut().find(|(known, _)| *known == name) {
            Some((_, count)) => *count += 1,
            None => sources.push((name, 1)),
        }
    }
    let mut parts: Vec<String> = Vec::new();
    match container.env.len() {
        0 => {}
        1 => parts.push("1 env var".to_owned()),
        count => parts.push(format!("{count} env vars")),
    }
    let mut named: Vec<String> = sources
        .into_iter()
        .map(|(name, count)| format!("{count} from {name}"))
        .collect();
    named.extend(
        container
            .env_from
            .iter()
            .filter_map(|entry| match &entry.source {
                EnvFromSource::ConfigMap { name } => Some(format!("all of configmap/{name}")),
                EnvFromSource::Secret { name } => Some(format!("all of secret/{name}")),
                EnvFromSource::Unknown => None,
            }),
    );
    let hidden = named.len().saturating_sub(MAX_SUMMARY_SOURCES);
    named.truncate(MAX_SUMMARY_SOURCES);
    parts.extend(named);
    if hidden > 0 {
        parts.push(format!("+{hidden} more"));
    }
    Some(parts.join(" · "))
}

/// `/etc/api ← configmap/api-config (read-only) · +3 more`; the first mount in spec order.
fn mount_summary(container: &ContainerSummary) -> Option<String> {
    let first = container.mounts.first()?;
    let mut text = format!(
        "{} ← {}",
        first.path,
        source_text(&first.source, &first.volume)
    );
    if first.is_read_only {
        text.push_str(" (read-only)");
    }
    let more = container.mounts.len() - 1;
    if more > 0 {
        text.push_str(&format!(" · +{more} more"));
    }
    Some(text)
}

/// `configmap/{n}`, `secret/{n}`, `pvc/{claim}`, then the volume kinds that have no screen.
fn source_text(source: &VolumeSource, volume: &str) -> String {
    match source {
        VolumeSource::ConfigMap { name } => format!("configmap/{name}"),
        VolumeSource::Secret { name } => format!("secret/{name}"),
        VolumeSource::PersistentVolumeClaim { claim } => format!("pvc/{claim}"),
        VolumeSource::EmptyDir => format!("emptyDir {volume}"),
        VolumeSource::HostPath { path } => format!("hostPath {path}"),
        VolumeSource::Projected { .. } => format!("projected {volume}"),
        VolumeSource::DownwardApi => format!("downwardAPI {volume}"),
        VolumeSource::Other => format!("volume {volume}"),
    }
}

/// The screen entry of a ConfigMap, Secret, or PersistentVolumeClaim source in the pod's
/// namespace. An empty name never reaches a link.
pub(crate) fn source_target(kind: &str, namespace: &str, name: &str) -> Option<ResourceKey> {
    if name.is_empty() {
        return None;
    }
    ResourceKey::of_object(kind, Some(namespace), name)
}

/// The screen of a volume's source; the other volume kinds have none.
pub(crate) fn volume_target(source: &VolumeSource, namespace: &str) -> Option<ResourceKey> {
    match source {
        VolumeSource::ConfigMap { name } => source_target("ConfigMap", namespace, name),
        VolumeSource::Secret { name } => source_target("Secret", namespace, name),
        VolumeSource::PersistentVolumeClaim { claim } => {
            source_target("PersistentVolumeClaim", namespace, claim)
        }
        VolumeSource::EmptyDir
        | VolumeSource::HostPath { .. }
        | VolumeSource::Projected { .. }
        | VolumeSource::DownwardApi
        | VolumeSource::Other => None,
    }
}

/// envFrom rows first, then the variables in spec order. Names and sources only.
fn env_rows(container: &ContainerSummary, namespace: &str) -> Vec<SourceRow> {
    let from_rows = container.env_from.iter().map(|entry| {
        let name = format!("{}*", entry.prefix.as_deref().unwrap_or_default());
        match &entry.source {
            EnvFromSource::ConfigMap { name: source } => SourceRow {
                name,
                source: format!("all keys of configmap/{source}"),
                target: source_target("ConfigMap", namespace, source),
                usage: None,
            },
            EnvFromSource::Secret { name: source } => SourceRow {
                name,
                source: format!("all keys of secret/{source}"),
                target: source_target("Secret", namespace, source),
                usage: None,
            },
            EnvFromSource::Unknown => SourceRow {
                name,
                source: "unknown source".to_owned(),
                target: None,
                usage: None,
            },
        }
    });
    let variable_rows = container.env.iter().map(|entry| {
        let (source, target) = match &entry.source {
            EnvSource::Literal => ("literal · value in the YAML tab".to_owned(), None),
            EnvSource::ConfigMapKey { name, key } => (
                format!("configmap/{name} · {key}"),
                source_target("ConfigMap", namespace, name),
            ),
            EnvSource::SecretKey { name, key } => (
                format!("secret/{name} · {key}"),
                source_target("Secret", namespace, name),
            ),
            EnvSource::Field { path } => (format!("field {path}"), None),
            EnvSource::ResourceField { resource } => (format!("resource {resource}"), None),
            EnvSource::Unknown => ("unknown source".to_owned(), None),
        };
        SourceRow {
            name: entry.name.clone(),
            source,
            target,
            usage: None,
        }
    });
    from_rows.chain(variable_rows).collect()
}

fn mount_rows(
    container: &ContainerSummary,
    namespace: &str,
    kubelet: Option<&KubeletHistory>,
) -> Vec<SourceRow> {
    container
        .mounts
        .iter()
        .map(|mount| {
            let mut source = source_text(&mount.source, &mount.volume);
            if mount.is_read_only {
                source.push_str(" · read-only");
            }
            if let Some(sub_path) = &mount.sub_path {
                source.push_str(&format!(" · subPath {sub_path}"));
            }
            let target = volume_target(&mount.source, namespace);
            let usage = match &mount.source {
                VolumeSource::PersistentVolumeClaim { claim } => kubelet
                    .and_then(|kubelet| kubelet.pvc_usage(namespace, claim))
                    .and_then(pvc_usage_note),
                _ => None,
            };
            SourceRow {
                name: mount.path.clone(),
                source,
                target,
                usage,
            }
        })
        .collect()
}

/// `None` without a used amount or a capacity.
fn pvc_usage_note(usage: &PvcUsage) -> Option<UsageNote> {
    let used = usage.used?.bytes() as f64;
    let capacity = usage.capacity?.bytes() as f64;
    if capacity <= 0. {
        return None;
    }
    let ratio = used / capacity;
    Some(UsageNote {
        text: format!(
            "{} used ({})",
            Measure::Bytes.format_pair(used, capacity, " of "),
            format_percent(ratio)
        ),
        tone: usage_tone(ratio),
    })
}

#[cfg(test)]
#[path = "container_detail_tests.rs"]
mod container_detail_tests;
