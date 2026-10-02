use std::borrow::Cow;

use cluster::{NodeSummary, NodeTaint};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, IntoElement, ParentElement as _, Pixels, Stateful,
    Styled as _, WeakEntity, Window, div, px,
};

use crate::age::format_age;
use crate::app_shell::{AppShell, Screen};
use crate::cluster_session::ClusterSession;
use crate::drawer::truncated_text;
use crate::filter_bar::filtered_empty_state;
use crate::metrics_history::NodeUsageHistory;
use crate::node_summary::{NodeCounts, node_counts, node_in_group};
use crate::node_usage::{NodeUsage, node_usage};
use crate::resource_actions::node_menu;
use crate::resource_kind::{Align, KindColumn, column};
use crate::status_tone::{StatusTone, node_status_label, tone_color, toned_text};
use crate::table_filter::FilterPreset;
use crate::table_layout::{ColumnPlan, TableLayout, clickable_row, header_cell, select_cell};
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
const AGE: usize = 8;

/// Marks a value the node does not have.
const ABSENT: &str = "—";

const TAINTS_MIN_WIDTH: Pixels = px(160.);
const USAGE_BAR_WIDTH: f32 = 46.;

/// The Taints column takes the rest of the width: it holds the longest values.
const NODE_COLUMNS: [KindColumn; 9] = [
    column("Name", 112., Align::Left),
    column("Status", 200., Align::Left),
    column("Roles", 110., Align::Left),
    column("Taints", 160., Align::Left),
    column("Version", 90., Align::Left),
    column("Internal IP", 120., Align::Left),
    column("CPU", 92., Align::Left),
    column("Memory", 92., Align::Left),
    column("Age", 60., Align::Right),
];

pub(crate) struct NodeTableDelegate {
    session: Option<Entity<ClusterSession>>,
    /// The row menu's "View YAML" opens the drawer through the shell.
    shell: WeakEntity<AppShell>,
    layout: TableLayout,
    view: TableView,
    /// Counts of all nodes, taken once per rebuild: the summary chips and the version skew read
    /// them.
    counts: Option<NodeCounts>,
}

impl NodeTableDelegate {
    pub(crate) fn new(shell: WeakEntity<AppShell>) -> Self {
        Self {
            session: None,
            shell,
            layout: TableLayout::new(ColumnPlan {
                specs: NODE_COLUMNS.to_vec(),
                flexible: TAINTS,
                flexible_min: TAINTS_MIN_WIDTH,
            }),
            view: TableView::new(default_filter(Screen::Nodes)),
            counts: None,
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

    pub(crate) fn set_session(&mut self, session: Option<Entity<ClusterSession>>) {
        self.session = session;
    }

    fn nodes<'a>(&self, cx: &'a App) -> &'a [NodeSummary] {
        let Some(session) = &self.session else {
            return &[];
        };
        session
            .read(cx)
            .live()
            .map_or(&[], |live| live.nodes.items())
    }

    /// The nodes with their usage as a share of allocatable, in session order, so item indices
    /// still index the session list.
    fn rows<'a>(&self, cx: &'a App) -> Vec<NodeRow<'a>> {
        let history = self.session.as_ref().and_then(|session| {
            let live = session.read(cx).live()?;
            Some(&live.metrics.nodes.history)
        });
        node_rows(self.nodes(cx), history)
    }

    fn usage_of(&self, node: &NodeSummary, cx: &App) -> NodeUsage {
        let latest = self.session.as_ref().and_then(|session| {
            let live = session.read(cx).live()?;
            live.metrics.nodes.history.latest(&node.name)
        });
        node_usage(node, latest)
    }

    /// The node shown at table row `row_ix`.
    fn node_at<'a>(&self, row_ix: usize, cx: &'a App) -> Option<&'a NodeSummary> {
        self.nodes(cx).get(self.view.item_index(row_ix)?)
    }
}

/// A node with its usage as a share of its allocatable resources.
pub(crate) struct NodeRow<'a> {
    pub(crate) node: &'a NodeSummary,
    pub(crate) usage: NodeUsage,
}

fn node_rows<'a>(nodes: &'a [NodeSummary], history: Option<&NodeUsageHistory>) -> Vec<NodeRow<'a>> {
    nodes
        .iter()
        .map(|node| NodeRow {
            node,
            usage: node_usage(node, history.and_then(|history| history.latest(&node.name))),
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
        node_status_label(self.node.status).tone
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        let node = self.node;
        match column {
            NAME => CellValue::Text(Cow::Borrowed(&node.name)),
            STATUS => {
                let label = node_status_label(node.status);
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
            AGE => CellValue::Age(node.created_at),
            _ => CellValue::Absent,
        }
    }

    fn in_preset(&self, preset: &FilterPreset) -> bool {
        match preset {
            FilterPreset::Nodes(group) => node_in_group(self.node, group),
            FilterPreset::HideInactive => true,
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
    }

    fn rebuild_view(&mut self, cx: &App) -> bool {
        let nodes = self.nodes(cx);
        self.counts = Some(node_counts(nodes));
        let rows = self.rows(cx);
        self.view
            .rebuild(&rows, NODE_COLUMNS.len(), jiff::Timestamp::now());
        self.layout.relayout(&self.view.hidden)
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
        let all_checked =
            self.layout.columns.is_select(col_ix) && self.view.all_checked(&self.rows(cx));
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
        if self.layout.columns.is_select(col_ix) {
            let is_checked = self.node_at(row_ix, cx).is_some_and(|node| {
                self.view.is_checked(&NodeRow {
                    node,
                    usage: NodeUsage::default(),
                })
            });
            return select_cell(row_ix, is_checked, &self.shell);
        }
        let (Some(node), Some(logical)) = (
            self.node_at(row_ix, cx),
            self.layout.columns.logical(col_ix),
        ) else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        match logical {
            NAME => truncated_text("name", node.name.clone()).into_any_element(),
            STATUS => toned_text(node_status_label(node.status), cx).into_any_element(),
            ROLES => cell_text(&roles_cell(&node.roles), cx),
            TAINTS => taints_cell(&node.taints, mono, cx),
            VERSION => {
                let cell = div().font_family(mono).child(node.kubelet_version.clone());
                let common = self
                    .counts
                    .as_ref()
                    .and_then(|counts| counts.common_version.as_ref());
                match common {
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
                let usage = self.usage_of(node, cx);
                let ratio = if logical == CPU {
                    usage.cpu
                } else {
                    usage.memory
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
            _ => div().into_any_element(),
        }
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let Some(session) = &self.session else {
            return menu;
        };
        let Some(live) = session.read(cx).live() else {
            return menu;
        };
        match self.node_at(row_ix, cx) {
            Some(node) => node_menu(menu, node, live, &self.shell),
            None => menu,
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        filtered_empty_state(&self.view, "No nodes".to_owned(), "nodes", &self.shell, cx)
    }

    fn loading(&self, cx: &App) -> bool {
        self.session
            .as_ref()
            .and_then(|session| session.read(cx).live())
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

/// The first taint, plus how many more there are.
struct TaintsSummary {
    first: String,
    more: usize,
}

fn taints_summary(taints: &[NodeTaint]) -> Option<TaintsSummary> {
    let (first, rest) = taints.split_first()?;
    Some(TaintsSummary {
        first: first.to_string(),
        more: rest.len(),
    })
}

fn taints_cell(taints: &[NodeTaint], mono: gpui_kit::SharedString, cx: &App) -> AnyElement {
    let Some(summary) = taints_summary(taints) else {
        return cell_text(ABSENT, cx);
    };
    let more = (summary.more > 0).then(|| {
        div()
            .flex_shrink_0()
            .text_color(cx.theme().muted_foreground)
            .child(format!(" +{}", summary.more))
    });
    // The taint is cut with an ellipsis; the "+N" stays visible.
    h_flex()
        .w_full()
        .font_family(mono)
        .child(truncated_text("taints", summary.first).min_w_0())
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

    fn taint(key: &str, effect: &str) -> NodeTaint {
        NodeTaint {
            key: key.to_owned(),
            value: None,
            effect: effect.to_owned(),
        }
    }

    #[test]
    fn taints_cell_shows_first_and_plus_count() {
        let taints = [
            taint("a", "NoSchedule"),
            taint("b", "NoExecute"),
            taint("c", "NoSchedule"),
        ];
        let summary = taints_summary(&taints).expect("has taints");
        assert_eq!(summary.first, "a:NoSchedule");
        assert_eq!(summary.more, 2);
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
        let rows = node_rows(&nodes, Some(&history));
        assert_eq!(rows[0].usage.cpu, Some(0.5));
        assert_eq!(rows[0].usage.memory, None);
        assert_eq!(node_rows(&nodes, None)[0].usage, NodeUsage::default());
    }

    #[test]
    fn roles_cell_dash_when_empty() {
        assert_eq!(roles_cell(&[]), ABSENT);
        let roles = ["control-plane".to_owned(), "etcd".to_owned()];
        assert_eq!(roles_cell(&roles), "control-plane, etcd");
    }
}
