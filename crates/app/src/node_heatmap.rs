//! The Nodes panel of Overview: one cell per node, shaded by CPU, with NotReady nodes outlined.

use cluster::{NodeReadiness, NodeScheduling, NodeSummary};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::{h_flex, tooltip::Tooltip};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app_shell::AppShell;
use crate::metrics_history::NodeUsageHistory;
use crate::node_usage::{NodeUsage, node_usage};
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ResourceKey;
use crate::usage_format::format_percent;

const CELL_SIZE: f32 = 24.;
const CELL_GAP: f32 = 3.;
/// A sampled cell is never fully faint: the lowest share still tints it, so it reads as measured.
const MIN_FILL_ALPHA: f32 = 0.15;

/// One node of the heatmap.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HeatCell {
    pub(crate) node: String,
    pub(crate) usage: NodeUsage,
    pub(crate) readiness: NodeReadiness,
    pub(crate) is_cordoned: bool,
}

impl HeatCell {
    /// The CPU share clamped to 0..=1; `None` without a sample, or when the node is not Ready.
    pub(crate) fn intensity(&self) -> Option<f32> {
        if self.is_not_ready() {
            return None;
        }
        let cpu = self.usage.cpu?;
        Some(if cpu.is_nan() {
            0.
        } else {
            cpu.clamp(0., 1.) as f32
        })
    }

    /// NotReady and Unknown nodes are outlined instead of shaded.
    pub(crate) fn is_not_ready(&self) -> bool {
        self.readiness != NodeReadiness::Ready
    }

    /// `ip-10-0-3-17 · CPU 62% · Memory 48% · Ready`, plus ` · SchedulingDisabled` when cordoned.
    pub(crate) fn tooltip(&self) -> String {
        let share = |ratio: Option<f64>| ratio.map_or_else(|| "—".to_owned(), format_percent);
        let status = match self.readiness {
            NodeReadiness::Ready => "Ready",
            NodeReadiness::NotReady => "NotReady",
            NodeReadiness::Unknown => "Unknown",
        };
        let mut text = format!(
            "{} · CPU {} · Memory {} · {status}",
            self.node,
            share(self.usage.cpu),
            share(self.usage.memory)
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

/// The fill opacity of a cell at CPU share `intensity`: `0.15 + 0.85 * intensity`.
fn fill_alpha(intensity: f32) -> f32 {
    MIN_FILL_ALPHA + (1. - MIN_FILL_ALPHA) * intensity
}

/// The wrapped grid. A click reveals the node with its drawer.
pub(crate) fn node_heatmap(cells: &[HeatCell], cx: &Context<AppShell>) -> impl IntoElement {
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
            let base = div()
                .id(SharedString::from(format!("node-{}", cell.node)))
                .relative()
                .flex_none()
                .size(px(CELL_SIZE))
                .rounded(theme.radius)
                .overflow_hidden()
                .cursor_pointer()
                .bg(theme.muted)
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .on_click(
                    cx.listener(move |shell, _, _, cx| shell.reveal_in_primary(key.clone(), cx)),
                );
            if cell.is_not_ready() {
                return base
                    .border_2()
                    .border_color(tone_color(StatusTone::Bad, cx));
            }
            base.children(cell.intensity().map(|intensity| {
                div()
                    .absolute()
                    .size_full()
                    .bg(theme.foreground.opacity(fill_alpha(intensity)))
            }))
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
    fn intensity_is_clamped_cpu_ratio() {
        let nodes = [ready("a")];
        let cell =
            |millicores| heat_cells(&nodes, Some(&history("a", millicores, 1)))[0].intensity();
        assert_eq!(cell(2_000), Some(0.5));
        assert_eq!(cell(9_000), Some(1.));
    }

    #[test]
    fn fill_alpha_starts_at_a_visible_tint() {
        assert_eq!(fill_alpha(0.), MIN_FILL_ALPHA);
        assert_eq!(fill_alpha(1.), 1.);
        assert!((fill_alpha(0.5) - 0.575).abs() < 1e-6);
    }

    #[test]
    fn not_ready_and_unknown_have_no_intensity() {
        for readiness in [NodeReadiness::NotReady, NodeReadiness::Unknown] {
            let nodes = [node("a", readiness, NodeScheduling::Enabled)];
            let cells = heat_cells(&nodes, Some(&history("a", 2_000, 1)));
            assert_eq!(cells[0].intensity(), None);
            assert!(cells[0].is_not_ready());
        }
    }

    #[test]
    fn missing_sample_has_no_intensity() {
        let nodes = [ready("a")];
        assert_eq!(heat_cells(&nodes, None)[0].intensity(), None);
        assert_eq!(
            heat_cells(&nodes, Some(&history("other", 1_000, 1)))[0].intensity(),
            None
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
