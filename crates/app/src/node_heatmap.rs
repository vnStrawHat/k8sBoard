//! The Nodes panel of Overview: one cell per node with its CPU and memory shares, tinted when
//! either is high, and NotReady nodes outlined.

use cluster::{NodeReadiness, NodeScheduling, NodeSummary};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{StyledExt as _, h_flex, tooltip::Tooltip, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app_shell::AppShell;
use crate::cluster_metrics::FeedStatus;
use crate::metrics_history::NodeUsageHistory;
use crate::node_usage::{NodeUsage, node_usage};
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ResourceKey;
use crate::usage_format::{format_percent, usage_tone};

const CELL_WIDTH: f32 = 168.;
const CELL_GAP: f32 = 4.;
/// The tint of a cell whose usage is high, over the muted base.
const TINT_ALPHA: f32 = 0.16;

/// Where the node usage numbers stand, so a cell can say why it has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UsageState {
    /// The feed is still being checked or has no sample yet.
    Loading,
    Live,
    /// Denied or failing: no sample will come.
    Absent,
}

impl UsageState {
    pub(crate) fn of(status: &FeedStatus) -> Self {
        match status {
            FeedStatus::Checking | FeedStatus::Waiting => Self::Loading,
            FeedStatus::Live | FeedStatus::Interrupted(_) => Self::Live,
            FeedStatus::Failed(_) | FeedStatus::Unavailable(_) => Self::Absent,
        }
    }
}

/// One node of the heatmap.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HeatCell {
    pub(crate) node: String,
    pub(crate) usage: NodeUsage,
    pub(crate) readiness: NodeReadiness,
    pub(crate) is_cordoned: bool,
}

impl HeatCell {
    /// The warn or bad tone of the higher of the CPU and memory shares, like the usage bars;
    /// `None` below 80 %, without a sample, or when the node is not Ready.
    pub(crate) fn tone(&self) -> Option<StatusTone> {
        if self.is_not_ready() {
            return None;
        }
        let highest = [self.usage.cpu, self.usage.memory]
            .into_iter()
            .flatten()
            .filter(|ratio| !ratio.is_nan())
            .fold(f64::NEG_INFINITY, f64::max);
        usage_tone(highest)
    }

    /// NotReady and Unknown nodes are outlined instead of tinted.
    pub(crate) fn is_not_ready(&self) -> bool {
        self.readiness != NodeReadiness::Ready
    }

    /// The line under the node name: `CPU 31% · MEM 56%`, or why there is none.
    pub(crate) fn usage_line(&self, state: UsageState) -> String {
        if self.is_not_ready() {
            return self.status_text().to_owned();
        }
        match state {
            UsageState::Loading => "Loading usage…".to_owned(),
            UsageState::Absent => self.status_text().to_owned(),
            UsageState::Live => {
                let share =
                    |ratio: Option<f64>| ratio.map_or_else(|| "—".to_owned(), format_percent);
                format!(
                    "CPU {} · MEM {}",
                    share(self.usage.cpu),
                    share(self.usage.memory)
                )
            }
        }
    }

    fn status_text(&self) -> &'static str {
        match self.readiness {
            NodeReadiness::Ready => "Ready",
            NodeReadiness::NotReady => "NotReady",
            NodeReadiness::Unknown => "Unknown",
        }
    }

    /// `ip-10-0-3-17 · CPU 62% · Memory 48% · Ready`, plus ` · SchedulingDisabled` when cordoned.
    pub(crate) fn tooltip(&self) -> String {
        let share = |ratio: Option<f64>| ratio.map_or_else(|| "—".to_owned(), format_percent);
        let mut text = format!(
            "{} · CPU {} · Memory {} · {}",
            self.node,
            share(self.usage.cpu),
            share(self.usage.memory),
            self.status_text()
        );
        if self.is_cordoned {
            text.push_str(" · SchedulingDisabled");
        }
        text
    }
}

/// The cells in node list order. `usage` is `None` while the node feed is not live.
pub(crate) fn heat_cells(nodes: &[NodeSummary], usage: Option<&NodeUsageHistory>) -> Vec<HeatCell> {
    nodes
        .iter()
        .map(|node| HeatCell {
            node: node.name.clone(),
            usage: node_usage(node, usage.and_then(|usage| usage.latest(&node.name))),
            readiness: node.status.readiness,
            is_cordoned: node.status.scheduling == NodeScheduling::Disabled,
        })
        .collect()
}

/// The wrapped grid. A click reveals the node with its drawer.
pub(crate) fn node_heatmap(
    cells: &[HeatCell],
    state: UsageState,
    cx: &Context<AppShell>,
) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .flex_wrap()
        .gap(px(CELL_GAP))
        .p_3()
        .children(cells.iter().map(|cell| {
            let key = ResourceKey::Node {
                name: cell.node.clone(),
            };
            let tooltip = cell.tooltip();
            let base = v_flex()
                .id(SharedString::from(format!("node-{}", cell.node)))
                .flex_none()
                .w(px(CELL_WIDTH))
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .overflow_hidden()
                .cursor_pointer()
                .bg(theme.muted)
                .border_1()
                .border_color(theme.border)
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(key.clone(), cx)))
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .truncate()
                        .child(cell.node.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(cell.usage_line(state)),
                );
            if cell.is_not_ready() {
                return base.border_color(tone_color(StatusTone::Bad, cx));
            }
            match cell.tone() {
                Some(tone) => {
                    let color = tone_color(tone, cx);
                    base.border_color(color).bg(color.opacity(TINT_ALPHA))
                }
                None => base,
            }
        }))
}

#[cfg(test)]
mod tests {
    use cluster::{NodeResource, NodeStatus, NodeSystemInfo};

    use super::*;

    fn node(name: &str, readiness: NodeReadiness, scheduling: NodeScheduling) -> NodeSummary {
        NodeSummary {
            name: name.to_owned(),
            status: NodeStatus {
                readiness,
                scheduling,
            },
            roles: Vec::new(),
            taints: Vec::new(),
            kubelet_version: "v1.29.5".to_owned(),
            internal_ip: None,
            created_at: None,
            conditions: Vec::new(),
            addresses: Vec::new(),
            system: NodeSystemInfo::default(),
            resources: ["cpu", "memory"]
                .map(|name| NodeResource {
                    name: name.to_owned(),
                    capacity: None,
                    allocatable: Some(if name == "cpu" { "4" } else { "8Gi" }.to_owned()),
                })
                .to_vec(),
            labels: Vec::new(),
        }
    }

    fn history(name: &str, millicores: u64, gibibytes: u64) -> NodeUsageHistory {
        let mut history = NodeUsageHistory::default();
        history.record(
            jiff::Timestamp::UNIX_EPOCH,
            &[cluster::NodeMetrics {
                name: name.to_owned(),
                sampled_at: None,
                usage: cluster::ResourceUsage {
                    cpu: cluster::CpuAmount::from_nanocores(millicores * 1_000_000),
                    memory: cluster::ByteAmount::from_bytes(gibibytes << 30),
                },
            }],
        );
        history
    }

    fn ready(name: &str) -> NodeSummary {
        node(name, NodeReadiness::Ready, NodeScheduling::Enabled)
    }

    #[test]
    fn cells_keep_node_list_order() {
        let nodes = [ready("b"), ready("a"), ready("c")];
        let names: Vec<String> = heat_cells(&nodes, None)
            .into_iter()
            .map(|cell| cell.node)
            .collect();
        assert_eq!(names, ["b", "a", "c"]);
    }

    #[test]
    fn tone_follows_the_higher_of_cpu_and_memory() {
        let nodes = [ready("a")];
        let tone = |millicores, gibibytes| {
            heat_cells(&nodes, Some(&history("a", millicores, gibibytes)))[0].tone()
        };
        // The node has 4 cores and 8Gi.
        assert_eq!(tone(2_000, 1), None);
        assert_eq!(tone(3_300, 1), Some(StatusTone::Warn));
        assert_eq!(tone(1_000, 7), Some(StatusTone::Warn));
        assert_eq!(tone(1_000, 8), Some(StatusTone::Bad));
    }

    #[test]
    fn not_ready_and_unknown_have_no_tone() {
        for readiness in [NodeReadiness::NotReady, NodeReadiness::Unknown] {
            let nodes = [node("a", readiness, NodeScheduling::Enabled)];
            let cells = heat_cells(&nodes, Some(&history("a", 4_000, 8)));
            assert_eq!(cells[0].tone(), None);
            assert!(cells[0].is_not_ready());
        }
    }

    #[test]
    fn missing_sample_has_no_tone() {
        let nodes = [ready("a")];
        assert_eq!(heat_cells(&nodes, None)[0].tone(), None);
        assert_eq!(
            heat_cells(&nodes, Some(&history("other", 4_000, 8)))[0].tone(),
            None
        );
    }

    #[test]
    fn the_usage_line_shows_shares_or_the_reason_there_are_none() {
        let nodes = [ready("a")];
        let live = heat_cells(&nodes, Some(&history("a", 1_240, 4)));
        assert_eq!(live[0].usage_line(UsageState::Live), "CPU 31% · MEM 50%");
        assert_eq!(live[0].usage_line(UsageState::Loading), "Loading usage…");
        assert_eq!(live[0].usage_line(UsageState::Absent), "Ready");
        let unsampled = heat_cells(&nodes, None);
        assert_eq!(unsampled[0].usage_line(UsageState::Live), "CPU — · MEM —");
        let down = [node("a", NodeReadiness::NotReady, NodeScheduling::Enabled)];
        assert_eq!(
            heat_cells(&down, None)[0].usage_line(UsageState::Loading),
            "NotReady"
        );
    }

    #[test]
    fn feed_status_maps_to_a_usage_state() {
        assert_eq!(UsageState::of(&FeedStatus::Checking), UsageState::Loading);
        assert_eq!(UsageState::of(&FeedStatus::Waiting), UsageState::Loading);
        assert_eq!(UsageState::of(&FeedStatus::Live), UsageState::Live);
        assert_eq!(
            UsageState::of(&FeedStatus::Interrupted("x".to_owned())),
            UsageState::Live
        );
        assert_eq!(
            UsageState::of(&FeedStatus::Failed("x".to_owned())),
            UsageState::Absent
        );
        assert_eq!(
            UsageState::of(&FeedStatus::Unavailable("x".to_owned())),
            UsageState::Absent
        );
    }

    #[test]
    fn tooltip_names_cpu_memory_status() {
        let nodes = [ready("ip-10-0-3-17")];
        let cells = heat_cells(&nodes, Some(&history("ip-10-0-3-17", 2_480, 4)));
        assert_eq!(
            cells[0].tooltip(),
            "ip-10-0-3-17 · CPU 62% · Memory 50% · Ready"
        );
        assert_eq!(
            heat_cells(&nodes, None)[0].tooltip(),
            "ip-10-0-3-17 · CPU — · Memory — · Ready"
        );
    }

    #[test]
    fn tooltip_marks_cordoned_node() {
        let nodes = [node("a", NodeReadiness::Ready, NodeScheduling::Disabled)];
        let cells = heat_cells(&nodes, None);
        assert!(cells[0].is_cordoned);
        assert!(cells[0].tooltip().ends_with(" · SchedulingDisabled"));
    }
}
