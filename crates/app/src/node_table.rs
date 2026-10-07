use std::borrow::Cow;
use std::collections::BTreeSet;

use cluster::{NodeSummary, NodeTaint, PodSummary};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Div, IntoElement, ParentElement as _, Pixels, Stateful, Styled as _,
    WeakEntity, Window, div, px,
};

use crate::age::format_age;
use crate::app_shell::{AppShell, Screen};
use crate::cell_truncation::{middle_truncate, mono_capacity, plain_text};
use crate::drawer::{truncated_text, truncated_text_with_tooltip};
use crate::filter_bar::filtered_empty_state;
use crate::metrics_history::NodeUsageHistory;
use crate::node_summary::{NodeCounts, node_counts, node_in_group};
use crate::node_usage::{NodeUsage, node_request_share, node_usage};
use crate::resource_actions::node_menu;
use crate::resource_kind::{Align, KindColumn, column};
use crate::row_context::TableSession;
use crate::settings::TablePrefs;
use crate::status_tone::{StatusTone, node_status_label, tone_color};
use crate::table_filter::FilterPreset;
use crate::table_layout::{
    ColumnPlan, TableLayout, centered_cell, clickable_row, header_cell, select_cell,
};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::table_view::{CellValue, FilteredTable, RowCheck, TableRow, TableView, default_filter};
use crate::usage_bar::{UsageBar, usage_bar};
use crate::usage_format::{format_percent, usage_tone};

const NAME: usize = 0;
const STATUS: usize = 1;
const ROLES: usize = 2;
const TAINTS: usize = 3;
const VERSION: usize = 4;
const INTERNAL_IP: usize = 5;
const CPU: usize = 6;
const MEMORY: usize = 7;
const CPU_REQUESTED: usize = 8;
const MEMORY_REQUESTED: usize = 9;
const AGE: usize = 10;
const LABELS: usize = 11;

/// Marks a value the node does not have.
const ABSENT: &str = "—";

const USAGE_BAR_WIDTH: f32 = 46.;

/// What the pods request, not what they use, and the labels: shown only when the user asks.
const HIDDEN_BY_DEFAULT: [usize; 3] = [CPU_REQUESTED, MEMORY_REQUESTED, LABELS];

/// The base widths of the default columns add up to what a 1100 px window leaves for the table, so
/// Memory and Age stay inside it. Name takes most of the spare width, up to 300 px (28 mono characters),
/// so node names stay whole at 1320 px; Taints is the column that gives way first. Internal IP is fixed
/// at the width of `255.255.255.255`, and Version at that of `v1.29.5`.
const NODE_COLUMNS: [KindColumn; 12] = [
    column("Name", 110., Align::Left).grows(8).up_to(300.),
    column("Status", 84., Align::Left),
    column("Roles", 106., Align::Left),
    column("Taints", 52., Align::Left).grows(2).up_to(420.),
    column("Version", 90., Align::Left),
    column("Internal IP", 140., Align::Left),
    column("CPU", 90., Align::Left),
    column("Memory", 90., Align::Left),
    column("CPU req", 92., Align::Left),
    column("Mem req", 92., Align::Left),
    column("Age", 56., Align::Right),
    column("Labels", 200., Align::Left).grows(2).up_to(420.),
];

pub(crate) struct NodeTableDelegate {
    /// The open cluster's session; `None` before the first one.
    session: Option<TableSession>,
    /// Every shown row is ticked: read by the header checkbox, computed with the rows (a rebuild or
    /// a tick) so a frame never reads the rows again.
    all_checked: bool,
    /// The row menu's "View YAML" opens the drawer through the shell.
    shell: WeakEntity<AppShell>,
    layout: TableLayout,
    view: TableView,
    /// Counts of the nodes, taken once per rebuild: the summary chips read them.
    counts: Option<NodeCounts>,
    /// The version most nodes run, so a node is skewed only against it.
    common_version: Option<String>,
}

fn node_plan() -> ColumnPlan {
    ColumnPlan {
        specs: NODE_COLUMNS.to_vec(),
        flexible: TAINTS,
    }
}

impl NodeTableDelegate {
    pub(crate) fn new(shell: WeakEntity<AppShell>, saved: Option<&TablePrefs>) -> Self {
        let plan = node_plan();
        let mut view = TableView::new(default_filter(Screen::Nodes));
        // The request columns are opt-in from the Columns menu; a saved choice replaces this.
        view.hidden = BTreeSet::from(HIDDEN_BY_DEFAULT);
        if let Some(saved) = saved {
            view.apply_prefs(saved, &plan);
        }
        Self {
            session: None,
            all_checked: false,
            shell,
            layout: TableLayout::new(plan),
            view,
            counts: None,
            common_version: None,
        }
    }

    /// Resizes the Taints column for a table `table_width` wide. Returns whether the columns
    /// changed, so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels) -> bool {
        self.layout.fit_width(table_width, &self.view.hidden)
    }

    /// The counts of the last rebuild; `None` before the first.
    pub(crate) fn counts(&self) -> Option<&NodeCounts> {
        self.counts.as_ref()
    }

    /// The session the rows come from. A tick belongs to the session it was made on: any change of
    /// the session entity, also to and from none, clears the ticks and the range anchor, so a
    /// same-named row of another cluster never inherits one. The caller refreshes the table.
    pub(crate) fn set_session(&mut self, session: Option<TableSession>) {
        let id = |session: &Option<TableSession>| {
            session.as_ref().map(|session| session.session.entity_id())
        };
        if id(&self.session) != id(&session) {
            self.view.clear_checked();
            self.all_checked = false;
        }
        self.session = session;
    }

    /// The ticked nodes in display order, in the open cluster: what a bulk Delete covers.
    pub(crate) fn checked_objects(&self, cx: &App) -> Vec<ClusterObject> {
        let Some(session) = &self.session else {
            return Vec::new();
        };
        if self.view.checked_count() == 0 {
            return Vec::new();
        }
        let rows = self.rows(cx);
        self.view
            .checked_rows(&rows)
            .into_iter()
            .filter_map(|index| rows.get(index))
            .map(|row| ClusterObject::new(session.cluster.clone(), ResourceKey::of_node(row.node)))
            .collect()
    }

    /// The nodes with their usage as a share of allocatable, so item indices still index the
    /// session list.
    fn rows<'a>(&self, cx: &'a App) -> Vec<NodeRow<'a>> {
        let history = self
            .session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .map(|live| &live.metrics.nodes.history);
        node_rows(self.nodes(cx), history, self.pods_of_all_namespaces(cx))
    }

    /// The pods list when it holds every namespace: a namespace scope would understate what a
    /// node's pods request, so the request columns read "—" then.
    fn pods_of_all_namespaces<'a>(&self, cx: &'a App) -> Option<&'a [PodSummary]> {
        let live = self.session.as_ref()?.session.read(cx).live()?;
        if live.scope != cluster::NamespaceScope::All {
            return None;
        }
        live.pods.ready_items()
    }

    /// The nodes of the open cluster; none while its session is not live.
    fn nodes<'a>(&self, cx: &'a App) -> &'a [NodeSummary] {
        self.session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .map_or(&[], |live| live.nodes.items())
    }

    fn usage_of(&self, session: &TableSession, node: &NodeSummary, cx: &App) -> NodeUsage {
        let latest = session
            .session
            .read(cx)
            .live()
            .and_then(|live| live.metrics.nodes.history.latest(&node.name));
        node_usage(node, latest)
    }

    fn requests_of(&self, node: &NodeSummary, cx: &App) -> NodeUsage {
        self.pods_of_all_namespaces(cx)
            .map_or_else(NodeUsage::default, |pods| node_request_share(node, pods))
    }

    /// The nodes in table order: the rows a name is told apart from.
    fn shown_nodes<'a>(&'a self, cx: &'a App) -> impl Iterator<Item = &'a NodeSummary> {
        let nodes = self.nodes(cx);
        self.view.rows().iter().filter_map(|&item| nodes.get(item))
    }

    /// The node shown at table row `row_ix`, and the session it belongs to.
    fn node_at<'a>(&self, row_ix: usize, cx: &'a App) -> Option<(&TableSession, &'a NodeSummary)> {
        let session = self.session.as_ref()?;
        let live = session.session.read(cx).live()?;
        let node = live.nodes.items().get(self.view.item_index(row_ix)?)?;
        Some((session, node))
    }
}

/// A node with its usage, and what its pods request, as shares of its allocatable resources.
pub(crate) struct NodeRow<'a> {
    pub(crate) node: &'a NodeSummary,
    pub(crate) usage: NodeUsage,
    /// All `None` while the pods of every namespace are not known.
    pub(crate) requests: NodeUsage,
}

fn node_rows<'a>(
    nodes: &'a [NodeSummary],
    history: Option<&NodeUsageHistory>,
    pods: Option<&[PodSummary]>,
) -> Vec<NodeRow<'a>> {
    nodes
        .iter()
        .map(|node| NodeRow {
            node,
            usage: node_usage(node, history.and_then(|history| history.latest(&node.name))),
            requests: pods.map_or_else(NodeUsage::default, |pods| node_request_share(node, pods)),
        })
        .collect()
}

/// The ratio in per mille, so the sort keeps one decimal of a percent.
fn per_mille(ratio: Option<f64>) -> CellValue<'static> {
    ratio.map_or(CellValue::Absent, |ratio| {
        CellValue::Number((ratio * 1000.).round() as i64)
    })
}

impl TableRow for NodeRow<'_> {
    fn namespace(&self) -> Option<&str> {
        None
    }

    fn name(&self) -> &str {
        &self.node.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        self.node.labels.iter().map(String::as_str)
    }

    fn tone(&self) -> StatusTone {
        node_status_label(self.node.status, &self.node.conditions).tone
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        let node = self.node;
        match column {
            NAME => CellValue::Text(Cow::Borrowed(&node.name)),
            STATUS => {
                let label = node_status_label(node.status, &node.conditions);
                CellValue::Status {
                    tone: label.tone,
                    text: label.text,
                }
            }
            ROLES if node.roles.is_empty() => CellValue::Absent,
            ROLES => CellValue::Text(Cow::Owned(node.roles.join(", "))),
            TAINTS => node.taints.first().map_or(CellValue::Absent, |taint| {
                CellValue::Text(Cow::Owned(taint.to_string()))
            }),
            VERSION => CellValue::Text(Cow::Borrowed(&node.kubelet_version)),
            INTERNAL_IP => node
                .internal_ip
                .as_deref()
                .map_or(CellValue::Absent, |ip| CellValue::Text(Cow::Borrowed(ip))),
            CPU => per_mille(self.usage.cpu),
            MEMORY => per_mille(self.usage.memory),
            CPU_REQUESTED => per_mille(self.requests.cpu),
            MEMORY_REQUESTED => per_mille(self.requests.memory),
            AGE => CellValue::Age(node.created_at),
            LABELS => {
                let summary = labels_summary(&node.labels);
                if summary.first.is_empty() {
                    CellValue::Absent
                } else {
                    CellValue::Text(Cow::Owned(summary.first.join(", ")))
                }
            }
            _ => CellValue::Absent,
        }
    }

    fn in_preset(&self, preset: &FilterPreset) -> bool {
        match preset {
            FilterPreset::Nodes(group) => node_in_group(self.node, group),
            FilterPreset::HideInactive
            | FilterPreset::HideSystem
            | FilterPreset::Changes
            | FilterPreset::Unmounted => true,
        }
    }
}

impl FilteredTable for NodeTableDelegate {
    fn view(&self) -> Option<&TableView> {
        Some(&self.view)
    }

    fn view_mut(&mut self) -> Option<&mut TableView> {
        Some(&mut self.view)
    }

    fn column_plan(&self) -> Option<&ColumnPlan> {
        Some(&self.layout.plan)
    }

    fn check_rows(&mut self, change: RowCheck, cx: &App) {
        let rows = self.rows(cx);
        self.view.apply_check(&rows, change);
        self.all_checked = self.view.all_checked(&rows);
    }

    fn rebuild_view(&mut self, cx: &App) -> bool {
        let counts = node_counts(self.nodes(cx));
        self.common_version.clone_from(&counts.common_version);
        self.counts = Some(counts);
        let rows = self.rows(cx);
        self.view
            .rebuild(&rows, self.layout.plan.specs.len(), jiff::Timestamp::now());
        self.all_checked = self.view.all_checked(&rows);
        self.layout.relayout(&self.view.hidden)
    }
}

impl NodeTableDelegate {
    /// The content of one body cell; the trait method centres it.
    fn cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        cx: &mut Context<TableState<Self>>,
    ) -> AnyElement {
        if self.layout.columns.is_select(col_ix) {
            let is_checked = self.node_at(row_ix, cx).is_some_and(|(_, node)| {
                self.view.is_checked(&NodeRow {
                    node,
                    usage: NodeUsage::default(),
                    requests: NodeUsage::default(),
                })
            });
            return select_cell(row_ix, is_checked, &self.shell);
        }
        let capacity = mono_capacity(self.layout.columns.columns.get(col_ix), cx);
        let (Some((session, node)), Some(logical)) = (
            self.node_at(row_ix, cx),
            self.layout.columns.logical(col_ix),
        ) else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        match logical {
            NAME => plain_text(
                ("node-name", row_ix),
                &node.name,
                &node.name,
                capacity,
                self.shown_nodes(cx).map(|node| (None, node.name.as_str())),
                cx,
            ),
            STATUS => {
                // Pressure names can outgrow the column, so the full label is the tooltip.
                let label = node_status_label(node.status, &node.conditions);
                div()
                    .text_color(tone_color(label.tone, cx))
                    .child(truncated_text_with_tooltip(
                        ("node-status", row_ix),
                        label.text.clone(),
                        label.text,
                    ))
                    .into_any_element()
            }
            ROLES => match roles_cell(&node.roles) {
                roles if roles == ABSENT => cell_text(ABSENT, cx),
                roles => truncated_text(("node-roles", row_ix), roles).into_any_element(),
            },
            TAINTS => taints_cell(&node.taints, mono, capacity, cx),
            VERSION => {
                let cell = div().font_family(mono).child(node.kubelet_version.clone());
                match self.common_version.as_ref() {
                    Some(common) if *common != node.kubelet_version => cell
                        .text_color(tone_color(StatusTone::Warn, cx))
                        .into_any_element(),
                    _ => cell.into_any_element(),
                }
            }
            INTERNAL_IP => match &node.internal_ip {
                Some(ip) => div().font_family(mono).child(ip.clone()).into_any_element(),
                None => cell_text(ABSENT, cx),
            },
            CPU | MEMORY => {
                let usage = self.usage_of(session, node, cx);
                let ratio = if logical == CPU {
                    usage.cpu
                } else {
                    usage.memory
                };
                usage_cell(ratio, mono, cx)
            }
            CPU_REQUESTED | MEMORY_REQUESTED => {
                let requests = self.requests_of(node, cx);
                let ratio = if logical == CPU_REQUESTED {
                    requests.cpu
                } else {
                    requests.memory
                };
                usage_cell(ratio, mono, cx)
            }
            AGE => div()
                .w_full()
                .text_right()
                .font_family(mono)
                // Read per cell: a render has no shared clock, and a second of skew is invisible.
                .child(format_age(node.created_at, jiff::Timestamp::now()))
                .into_any_element(),
            LABELS => labels_cell(&node.labels, mono, cx),
            _ => div().into_any_element(),
        }
    }
}

impl TableDelegate for NodeTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.layout.columns.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.view.rows().len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.layout
            .columns
            .columns
            .get(col_ix)
            .cloned()
            .unwrap_or_default()
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let all_checked = self.layout.columns.is_select(col_ix) && self.all_checked;
        header_cell(
            &self.layout,
            self.view.sort,
            all_checked,
            &self.shell,
            col_ix,
            cx,
        )
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        clickable_row(row_ix, &self.shell)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        centered_cell(self.cell(row_ix, col_ix, cx))
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let Some((open, node)) = self.node_at(row_ix, cx) else {
            return menu;
        };
        let session = open.session.read(cx);
        let (Some(live), Some(guard)) = (session.live(), session.guard(cx)) else {
            return menu;
        };
        let row = open.row_context(cx);
        node_menu(menu, node, live, &guard, &row, &self.shell)
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        filtered_empty_state(&self.view, "No nodes".to_owned(), "nodes", &self.shell, cx)
    }

    /// Loading while the session is live and still waits for its first list.
    fn loading(&self, cx: &App) -> bool {
        self.session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .is_some_and(|live| live.nodes.is_loading())
    }
}

/// The bar and the percent, both in the usage tone; a muted dash without a sample.
fn usage_cell(ratio: Option<f64>, mono: gpui_kit::SharedString, cx: &App) -> AnyElement {
    let Some(ratio) = ratio else {
        return cell_text(ABSENT, cx);
    };
    let percent = div().font_family(mono).child(format_percent(ratio));
    let percent = match usage_tone(ratio) {
        Some(tone) => percent.text_color(tone_color(tone, cx)),
        None => percent,
    };
    h_flex()
        .gap_1p5()
        .items_center()
        .child(usage_bar(
            UsageBar::of_ratio(ratio, None),
            px(USAGE_BAR_WIDTH),
            cx,
        ))
        .child(percent)
        .into_any_element()
}

/// Roles joined with commas; "—" when the node has none. No `worker` is inferred.
fn roles_cell(roles: &[String]) -> String {
    if roles.is_empty() {
        return ABSENT.to_owned();
    }
    roles.join(", ")
}

/// The first taint as the cell spells it, how many more there are, and every taint for the tooltip.
struct TaintsSummary {
    first: String,
    more: usize,
    all: String,
}

fn taints_summary(taints: &[NodeTaint]) -> Option<TaintsSummary> {
    let (first, rest) = taints.split_first()?;
    Some(TaintsSummary {
        first: short_taint(first),
        more: rest.len(),
        all: taints
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

/// A taint as `key:effect` in the cell: no key domain (`control-plane` for
/// `node-role.kubernetes.io/control-plane`), no value, and the effect cut short (`NoSched`). The
/// tooltip keeps the whole taints.
fn short_taint(taint: &NodeTaint) -> String {
    let key = taint.key.rsplit('/').next().unwrap_or(&taint.key);
    let effect = match taint.effect.as_str() {
        "NoSchedule" => "NoSched",
        "PreferNoSchedule" => "PreferNoSched",
        "NoExecute" => "NoExec",
        other => other,
    };
    format!("{key}:{effect}")
}

fn taints_cell(
    taints: &[NodeTaint],
    mono: gpui_kit::SharedString,
    capacity: usize,
    cx: &App,
) -> AnyElement {
    let Some(summary) = taints_summary(taints) else {
        return cell_text(ABSENT, cx);
    };
    let more_width = if summary.more > 0 {
        format!(" +{}", summary.more).chars().count()
    } else {
        0
    };
    let more = (summary.more > 0).then(|| {
        div()
            .flex_shrink_0()
            .text_color(cx.theme().muted_foreground)
            .child(format!(" +{}", summary.more))
    });
    // The taint is cut in the middle, so its effect stays visible; the "+N" is never cut.
    h_flex()
        .w_full()
        .font_family(mono)
        .child(
            truncated_text_with_tooltip(
                "taints",
                middle_truncate(&summary.first, capacity.saturating_sub(more_width)).into_owned(),
                summary.all,
            )
            .min_w_0(),
        )
        .children(more)
        .into_any_element()
}

/// How many labels the Labels column spells out before `+N`.
const SHOWN_LABELS: usize = 2;

/// Keys every cluster sets on its nodes: they say nothing about this node, so the column leaves
/// them out and only the tooltip lists them. Roles has its own column.
const SYSTEM_LABEL_PREFIXES: [&str; 4] = [
    "kubernetes.io/",
    "node.kubernetes.io/",
    "beta.kubernetes.io/",
    "node-role.kubernetes.io/",
];

/// The first labels of a node worth reading, and how many more there are.
struct LabelsSummary<'a> {
    first: Vec<&'a str>,
    more: usize,
}

fn labels_summary(labels: &[String]) -> LabelsSummary<'_> {
    let mut own = labels.iter().map(String::as_str).filter(|label| {
        !SYSTEM_LABEL_PREFIXES
            .iter()
            .any(|prefix| label.starts_with(prefix))
    });
    let first: Vec<&str> = own.by_ref().take(SHOWN_LABELS).collect();
    LabelsSummary {
        first,
        more: own.count(),
    }
}

/// The first two own labels and a muted `+N`; the tooltip lists every label.
fn labels_cell(labels: &[String], mono: gpui_kit::SharedString, cx: &App) -> AnyElement {
    let summary = labels_summary(labels);
    if summary.first.is_empty() {
        return cell_text(ABSENT, cx);
    }
    let more = (summary.more > 0).then(|| {
        div()
            .flex_shrink_0()
            .text_color(cx.theme().muted_foreground)
            .child(format!(" +{}", summary.more))
    });
    h_flex()
        .w_full()
        .font_family(mono)
        .child(
            truncated_text_with_tooltip("labels", summary.first.join(", "), labels.join("\n"))
                .min_w_0(),
        )
        .children(more)
        .into_any_element()
}

/// Shows `text`, or a muted dash when the value is absent.
fn cell_text(text: &str, cx: &App) -> AnyElement {
    if text == ABSENT {
        return div()
            .text_color(cx.theme().muted_foreground)
            .child("—")
            .into_any_element();
    }
    div().truncate().child(text.to_owned()).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_truncation::cut_name;
    use crate::table_layout::layout_columns;

    fn taint(key: &str, effect: &str) -> NodeTaint {
        NodeTaint {
            key: key.to_owned(),
            value: None,
            effect: effect.to_owned(),
            time_added: None,
        }
    }

    #[test]
    fn short_taint_drops_the_key_domain_and_the_value_and_cuts_the_effect() {
        let spelled = |key: &str, value: Option<&str>, effect: &str| {
            short_taint(&NodeTaint {
                key: key.to_owned(),
                value: value.map(str::to_owned),
                effect: effect.to_owned(),
                time_added: None,
            })
        };
        assert_eq!(
            spelled("node-role.kubernetes.io/control-plane", None, "NoSchedule"),
            "control-plane:NoSched"
        );
        assert_eq!(
            spelled("workload", Some("ingress"), "NoSchedule"),
            "workload:NoSched"
        );
        assert_eq!(spelled("a", None, "NoExecute"), "a:NoExec");
        assert_eq!(spelled("a", None, "PreferNoSchedule"), "a:PreferNoSched");
        assert_eq!(spelled("a", None, "Odd"), "a:Odd");
    }

    #[test]
    fn taints_cell_shows_first_and_plus_count() {
        let taints = [
            taint("a", "NoSchedule"),
            taint("b", "NoExecute"),
            taint("c", "NoSchedule"),
        ];
        let summary = taints_summary(&taints).expect("has taints");
        assert_eq!(summary.first, "a:NoSched");
        assert_eq!(summary.more, 2);
        // The tooltip lists every taint whole, one a line.
        assert_eq!(summary.all, "a:NoSchedule\nb:NoExecute\nc:NoSchedule");
        assert!(taints_summary(&[]).is_none());
        assert_eq!(taints_summary(&taints[..1]).expect("one taint").more, 0);
    }

    fn node() -> NodeSummary {
        NodeSummary {
            name: "wk-03".to_owned(),
            status: cluster::NodeStatus {
                readiness: cluster::NodeReadiness::NotReady,
                scheduling: cluster::NodeScheduling::Enabled,
            },
            roles: vec!["control-plane".to_owned(), "etcd".to_owned()],
            taints: vec![taint("a", "NoSchedule"), taint("b", "NoExecute")],
            kubelet_version: "v1.29.5".to_owned(),
            internal_ip: Some("10.0.0.3".to_owned()),
            created_at: None,
            conditions: Vec::new(),
            addresses: Vec::new(),
            system: cluster::NodeSystemInfo::default(),
            resources: Vec::new(),
            labels: vec!["role=db".to_owned()],
        }
    }

    fn row(node: &NodeSummary) -> NodeRow<'_> {
        NodeRow {
            node,
            usage: NodeUsage::default(),
            requests: NodeUsage::default(),
        }
    }

    #[test]
    fn node_row_values_follow_columns() {
        let node = node();
        let row = row(&node);
        assert!(matches!(row.value(NAME), CellValue::Text(text) if text == "wk-03"));
        assert!(matches!(
            row.value(STATUS),
            CellValue::Status {
                tone: StatusTone::Bad,
                ..
            }
        ));
        assert!(matches!(row.value(ROLES), CellValue::Text(text) if text == "control-plane, etcd"));
        assert!(matches!(row.value(TAINTS), CellValue::Text(text) if text == "a:NoSchedule"));
        assert!(matches!(row.value(VERSION), CellValue::Text(text) if text == "v1.29.5"));
        assert!(matches!(row.value(INTERNAL_IP), CellValue::Text(text) if text == "10.0.0.3"));
        assert!(matches!(row.value(AGE), CellValue::Age(None)));
        assert!(matches!(row.value(NODE_COLUMNS.len()), CellValue::Absent));
    }

    #[test]
    fn node_row_reads_scope_and_labels() {
        let node = node();
        let row = row(&node);
        assert_eq!(row.namespace(), None);
        assert_eq!(row.labels().collect::<Vec<_>>(), ["role=db"]);
    }

    #[test]
    fn node_row_values_are_absent_without_roles_taints_or_ip() {
        let bare = NodeSummary {
            roles: Vec::new(),
            taints: Vec::new(),
            internal_ip: None,
            ..node()
        };
        let row = row(&bare);
        assert!(matches!(row.value(ROLES), CellValue::Absent));
        assert!(matches!(row.value(TAINTS), CellValue::Absent));
        assert!(matches!(row.value(INTERNAL_IP), CellValue::Absent));
    }

    #[test]
    fn node_row_usage_is_per_mille() {
        let node = node();
        let with_usage = NodeRow {
            node: &node,
            usage: NodeUsage {
                cpu: Some(0.314),
                memory: None,
            },
            requests: NodeUsage::default(),
        };
        assert!(matches!(with_usage.value(CPU), CellValue::Number(314)));
        assert!(matches!(with_usage.value(MEMORY), CellValue::Absent));
        assert!(matches!(row(&node).value(CPU), CellValue::Absent));
    }

    #[test]
    fn node_rows_attach_usage_from_the_history() {
        let mut node = node();
        node.resources = vec![cluster::NodeResource {
            name: "cpu".to_owned(),
            capacity: None,
            allocatable: Some("4".to_owned()),
        }];
        let mut history = NodeUsageHistory::default();
        history.record(
            jiff::Timestamp::UNIX_EPOCH,
            &[cluster::NodeMetrics {
                name: "wk-03".to_owned(),
                sampled_at: None,
                usage: cluster::ResourceUsage {
                    cpu: cluster::CpuAmount::from_nanocores(2_000_000_000),
                    memory: cluster::ByteAmount::from_bytes(1),
                },
            }],
        );
        let nodes = [node];
        let rows = node_rows(&nodes, Some(&history), None);
        assert_eq!(rows[0].usage.cpu, Some(0.5));
        assert_eq!(rows[0].usage.memory, None);
        assert_eq!(node_rows(&nodes, None, None)[0].usage, NodeUsage::default());
    }

    #[test]
    fn the_base_widths_fit_a_1100_px_window_and_name_grows_most() {
        // The window less the 220 px sidebar, the table gutter, and the checkbox column.
        let room = 1100. - 220. - 28. - 32.;
        // The request columns are hidden by default, so they take no room until asked for.
        let base: f32 = NODE_COLUMNS
            .iter()
            .enumerate()
            .filter(|(index, _)| !HIDDEN_BY_DEFAULT.contains(index))
            .map(|(_, column)| column.width)
            .sum();
        assert!(base <= room, "{base} px of columns for {room} px");
        let ip = &NODE_COLUMNS[INTERNAL_IP];
        assert!(
            ip.width >= 130. && ip.weight == 0,
            "an IP never shrinks or grows"
        );
        assert!(NODE_COLUMNS[VERSION].width >= 90., "v1.29.5 shows whole");
        assert!(NODE_COLUMNS[STATUS].width >= 84., "Cordoned shows whole");
        let heaviest = NODE_COLUMNS.iter().map(|column| column.weight).max();
        assert_eq!(heaviest, Some(NODE_COLUMNS[NAME].weight));
        assert!(NODE_COLUMNS[TAINTS].width < NODE_COLUMNS[NAME].width);
    }

    /// The mono characters that fit in the Name column of a window `window` px wide: the table
    /// is the window less the 220 px sidebar, and a mono glyph is about 9.6 px.
    fn name_capacity(window: f32) -> usize {
        let plan = node_plan();
        let hidden = BTreeSet::from(HIDDEN_BY_DEFAULT);
        let layout = layout_columns(&plan.specs, plan.flexible, px(window - 220.), &hidden);
        let name = layout.columns.get(1).expect("a Name column").width;
        (f32::from(name - px(24.)) / 9.6) as usize
    }

    #[test]
    fn the_lab_node_names_stay_whole_at_1320_px() {
        assert!(name_capacity(1320.) >= 26, "{}", name_capacity(1320.));
    }

    #[test]
    fn the_lab_node_names_elide_to_different_strings_at_any_width() {
        let names = [
            "k8sboard-lab-control-plane",
            "k8sboard-lab-worker",
            "k8sboard-lab-worker2",
        ];
        for window in [900., 1100., 1320.] {
            let capacity = name_capacity(window);
            let shown: BTreeSet<_> = names
                .iter()
                .map(|name| cut_name(name, capacity, names.map(|name| (None, name))).into_owned())
                .collect();
            assert_eq!(
                shown.len(),
                3,
                "{window} px, capacity {capacity}: {shown:?}"
            );
        }
    }

    #[test]
    fn node_names_that_share_both_ends_keep_their_start_apart() {
        let names = [
            "ip-10-0-1-11.eu-west-1.compute.internal",
            "ip-10-0-1-12.eu-west-1.compute.internal",
        ];
        let shown: BTreeSet<_> = names
            .iter()
            .map(|name| cut_name(name, 24, names.map(|name| (None, name))).into_owned())
            .collect();
        assert_eq!(shown.len(), 2, "{shown:?}");
    }

    fn labels(terms: &[&str]) -> Vec<String> {
        terms.iter().map(|term| (*term).to_owned()).collect()
    }

    #[test]
    fn the_labels_column_skips_system_keys_and_counts_the_rest() {
        let labels = labels(&[
            "beta.kubernetes.io/arch=amd64",
            "k8sboard.io/pool=web",
            "kubernetes.io/hostname=k8sboard-lab-worker",
            "maintenance-window=sun-02",
            "node-role.kubernetes.io/control-plane=",
            "node.kubernetes.io/instance-type=kind",
            "topology.kubernetes.io/zone=lab-a",
        ]);
        let summary = labels_summary(&labels);
        assert_eq!(
            summary.first,
            ["k8sboard.io/pool=web", "maintenance-window=sun-02"]
        );
        // The zone label is not a system key: only kubernetes.io/ and node.kubernetes.io/ are.
        assert_eq!(summary.more, 1);
    }

    #[test]
    fn a_node_with_only_system_labels_has_no_labels_cell() {
        let mut node = node();
        node.labels = labels(&["kubernetes.io/os=linux"]);
        assert!(matches!(row(&node).value(LABELS), CellValue::Absent));
        assert_eq!(labels_summary(&node.labels).more, 0);
        node.labels = labels(&["role=db"]);
        assert!(matches!(row(&node).value(LABELS), CellValue::Text(text) if text == "role=db"));
    }

    #[test]
    fn labels_are_hidden_until_asked_for() {
        assert_eq!(NODE_COLUMNS[LABELS].name, "Labels");
        assert!(HIDDEN_BY_DEFAULT.contains(&LABELS));
    }

    #[test]
    fn roles_cell_dash_when_empty() {
        assert_eq!(roles_cell(&[]), ABSENT);
        let roles = ["control-plane".to_owned(), "etcd".to_owned()];
        assert_eq!(roles_cell(&roles), "control-plane, etcd");
    }

    #[test]
    fn request_columns_are_hidden_until_asked_for() {
        assert_eq!(NODE_COLUMNS[CPU_REQUESTED].name, "CPU req");
        assert_eq!(NODE_COLUMNS[MEMORY_REQUESTED].name, "Mem req");
        assert_eq!(HIDDEN_BY_DEFAULT[..2], [CPU_REQUESTED, MEMORY_REQUESTED]);
    }

    #[test]
    fn node_row_requests_are_per_mille_and_absent_when_unknown() {
        let node = node();
        let with_requests = NodeRow {
            node: &node,
            usage: NodeUsage::default(),
            requests: NodeUsage {
                cpu: Some(0.6),
                memory: Some(0.0725),
            },
        };
        assert!(matches!(
            with_requests.value(CPU_REQUESTED),
            CellValue::Number(600)
        ));
        assert!(matches!(
            with_requests.value(MEMORY_REQUESTED),
            CellValue::Number(73)
        ));
        assert!(matches!(row(&node).value(CPU_REQUESTED), CellValue::Absent));
    }
}
