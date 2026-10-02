use std::borrow::Cow;

use cluster::{PodSummary, ResourceUsage};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, HighlightStyle, IntoElement, ParentElement as _, Pixels,
    SharedString, Stateful, Styled as _, StyledText, WeakEntity, Window, div, px,
};

use crate::age::format_age;
use crate::app_shell::{AppShell, Screen};
use crate::cluster_session::ClusterSession;
use crate::filter_bar::filtered_empty_state;
use crate::log_dock::LogDock;
use crate::metrics_history::PodUsageHistory;
use crate::resource_actions::pod_menu;
use crate::resource_kind::{Align, KindColumn, column};
use crate::status_tone::{StatusTone, pod_status_label, toned_text};
use crate::table_filter::FilterPreset;
use crate::table_layout::{ColumnPlan, TableLayout, clickable_row, header_cell, select_cell};
use crate::table_view::{CellValue, FilteredTable, RowCheck, TableRow, TableView, default_filter};
use crate::usage_format::Measure;

const NAME: usize = 0;
const STATUS: usize = 1;
const READY: usize = 2;
const RESTARTS: usize = 3;
const CPU: usize = 4;
const MEMORY: usize = 5;
pub(crate) const NODE: usize = 6;
const AGE: usize = 7;

const NAME_MIN_WIDTH: Pixels = px(160.);

/// The Name column takes the rest of the width: pod names are the longest values.
const POD_COLUMNS: [KindColumn; 8] = [
    column("Name", 160., Align::Left),
    column("Status", 170., Align::Left),
    column("Ready", 70., Align::Left),
    column("Restarts", 80., Align::Right),
    column("CPU", 70., Align::Right),
    column("Memory", 80., Align::Right),
    column("Node", 180., Align::Left),
    column("Age", 70., Align::Right),
];

/// Rows come straight from the session, so the table never owns a copy of the pods.
pub(crate) struct PodTableDelegate {
    session: Option<Entity<ClusterSession>>,
    log_dock: WeakEntity<LogDock>,
    /// The row menu's "View YAML" opens the drawer through the shell.
    shell: WeakEntity<AppShell>,
    layout: TableLayout,
    view: TableView,
}

impl PodTableDelegate {
    pub(crate) fn new(log_dock: WeakEntity<LogDock>, shell: WeakEntity<AppShell>) -> Self {
        Self {
            session: None,
            log_dock,
            shell,
            layout: TableLayout::new(ColumnPlan {
                specs: POD_COLUMNS.to_vec(),
                flexible: NAME,
                flexible_min: NAME_MIN_WIDTH,
            }),
            view: pods_view(),
        }
    }

    /// Resizes the Name column for a table `table_width` wide. Returns whether the columns
    /// changed, so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels) -> bool {
        self.layout.fit_width(table_width, &self.view.hidden)
    }

    pub(crate) fn set_session(&mut self, session: Option<Entity<ClusterSession>>) {
        self.session = session;
    }

    fn pods<'a>(&self, cx: &'a App) -> &'a [PodSummary] {
        let Some(session) = &self.session else {
            return &[];
        };
        session
            .read(cx)
            .live()
            .map_or(&[], |live| live.pods.items())
    }

    /// The pods with their newest usage, in session order, so item indices still index the
    /// session list.
    fn rows<'a>(&self, cx: &'a App) -> Vec<PodRow<'a>> {
        let history = self.history(cx);
        pod_rows(self.pods(cx), history)
    }

    fn history<'a>(&self, cx: &'a App) -> Option<&'a PodUsageHistory> {
        let live = self.session.as_ref()?.read(cx).live()?;
        Some(&live.metrics.pods.history)
    }

    /// The pod shown at table row `row_ix`.
    fn pod_at<'a>(&self, row_ix: usize, cx: &'a App) -> Option<&'a PodSummary> {
        self.pods(cx).get(self.view.item_index(row_ix)?)
    }

    fn scope_label(&self, cx: &App) -> String {
        self.session
            .as_ref()
            .and_then(|session| session.read(cx).live())
            .map_or_else(String::new, |live| live.scope_label())
    }
}

/// A pod with its newest usage: what the table toolkit filters, sorts, and ticks.
pub(crate) struct PodRow<'a> {
    pub(crate) pod: &'a PodSummary,
    /// `None` without a sample: a new pod, a finished one, or metrics unavailable.
    pub(crate) usage: Option<ResourceUsage>,
}

fn pod_rows<'a>(pods: &'a [PodSummary], history: Option<&PodUsageHistory>) -> Vec<PodRow<'a>> {
    pods.iter()
        .map(|pod| PodRow {
            pod,
            usage: history.and_then(|history| history.latest(&pod.namespace, &pod.name)),
        })
        .collect()
}

/// The Pods view starts with the CPU column hidden: W4 shows Memory only. Columns ▾ brings it
/// back.
fn pods_view() -> TableView {
    let mut view = TableView::new(default_filter(Screen::Pods));
    view.hidden.insert(CPU);
    view
}

fn saturating_number(value: u64) -> CellValue<'static> {
    CellValue::Number(i64::try_from(value).unwrap_or(i64::MAX))
}

impl TableRow for PodRow<'_> {
    fn namespace(&self) -> Option<&str> {
        Some(&self.pod.namespace)
    }

    fn name(&self) -> &str {
        &self.pod.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        self.pod.labels.iter().map(String::as_str)
    }

    fn tone(&self) -> StatusTone {
        pod_status_label(self.pod).tone
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        let pod = self.pod;
        match column {
            NAME => CellValue::Qualified {
                prefix: Some(&pod.namespace),
                text: &pod.name,
            },
            STATUS => {
                let label = pod_status_label(pod);
                CellValue::Status {
                    tone: label.tone,
                    text: label.text,
                }
            }
            READY => CellValue::Text(Cow::Owned(pod.ready.to_string())),
            RESTARTS => CellValue::Number(i64::from(pod.restarts)),
            CPU => self.usage.map_or(CellValue::Absent, |usage| {
                saturating_number(usage.cpu.nanocores())
            }),
            MEMORY => self.usage.map_or(CellValue::Absent, |usage| {
                saturating_number(usage.memory.bytes())
            }),
            NODE => pod.node_name.as_deref().map_or(CellValue::Absent, |node| {
                CellValue::Text(Cow::Borrowed(node))
            }),
            AGE => CellValue::Age(pod.created_at),
            _ => CellValue::Absent,
        }
    }

    fn in_preset(&self, _: &FilterPreset) -> bool {
        true
    }
}

impl FilteredTable for PodTableDelegate {
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
        let rows = self.rows(cx);
        self.view
            .rebuild(&rows, POD_COLUMNS.len(), jiff::Timestamp::now());
        self.layout.relayout(&self.view.hidden)
    }
}

impl TableDelegate for PodTableDelegate {
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
            let is_checked = self
                .pod_at(row_ix, cx)
                .is_some_and(|pod| self.view.is_checked(&PodRow { pod, usage: None }));
            return select_cell(row_ix, is_checked, &self.shell);
        }
        let (Some(pod), Some(logical)) =
            (self.pod_at(row_ix, cx), self.layout.columns.logical(col_ix))
        else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        match logical {
            NAME => name_cell(pod, mono, cx),
            STATUS => toned_text(pod_status_label(pod), cx).into_any_element(),
            READY => div()
                .font_family(mono)
                .child(pod.ready.to_string())
                .into_any_element(),
            RESTARTS => div()
                .w_full()
                .text_right()
                .font_family(mono)
                .child(pod.restarts.to_string())
                .into_any_element(),
            CPU | MEMORY => {
                let usage = self
                    .history(cx)
                    .and_then(|history| history.latest(&pod.namespace, &pod.name));
                let text = usage.map(|usage| match logical {
                    CPU => Measure::Cpu.format(usage.cpu.cores()),
                    _ => Measure::Bytes.format(usage.memory.bytes() as f64),
                });
                usage_cell(text, mono, cx)
            }
            NODE => match &pod.node_name {
                Some(node_name) => div().child(node_name.clone()).into_any_element(),
                None => dash_cell(cx),
            },
            AGE => div()
                .w_full()
                .text_right()
                .font_family(mono)
                // Read per cell: a render has no shared clock, and a second of skew is invisible.
                .child(format_age(pod.created_at, jiff::Timestamp::now()))
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
        match self.pod_at(row_ix, cx) {
            Some(pod) => pod_menu(
                menu,
                pod,
                live,
                session.read(cx).context(),
                &self.log_dock,
                &self.shell,
            ),
            None => menu,
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let empty = format!("No pods in {}", self.scope_label(cx));
        filtered_empty_state(&self.view, empty, "pods", &self.shell, cx)
    }

    fn loading(&self, cx: &App) -> bool {
        self.session
            .as_ref()
            .and_then(|session| session.read(cx).live())
            .is_some_and(|live| live.pods.is_loading())
    }
}

/// `{namespace}/` is muted so the pod name stands out; both share one text run so a long
/// name is cut with an ellipsis instead of wrapping.
fn name_cell(pod: &PodSummary, mono: SharedString, cx: &App) -> AnyElement {
    let prefix = format!("{}/", pod.namespace);
    let muted = HighlightStyle {
        color: Some(cx.theme().muted_foreground),
        ..Default::default()
    };
    let highlights = vec![(0..prefix.len(), muted)];
    let text = format!("{prefix}{}", pod.name);
    div()
        .w_full()
        .truncate()
        .font_family(mono)
        .child(StyledText::new(text).with_highlights(highlights))
        .into_any_element()
}

/// A usage value, right-aligned like the other numbers; a muted dash without a sample.
fn usage_cell(text: Option<String>, mono: SharedString, cx: &App) -> AnyElement {
    let cell = div().w_full().text_right();
    match text {
        Some(text) => cell.font_family(mono).child(text).into_any_element(),
        None => cell
            .text_color(cx.theme().muted_foreground)
            .child("—")
            .into_any_element(),
    }
}

fn dash_cell(cx: &App) -> AnyElement {
    div()
        .text_color(cx.theme().muted_foreground)
        .child("—")
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use cluster::{PodStatus, ReadyCount, StatusReason};

    use super::*;

    fn pod() -> PodSummary {
        PodSummary {
            namespace: "payments".to_owned(),
            name: "api-7".to_owned(),
            status: PodStatus::Reason(StatusReason::Running),
            ready: ReadyCount { ready: 3, total: 4 },
            restarts: 12,
            node_name: Some("wk-03".to_owned()),
            created_at: None,
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: None,
            conditions: Vec::new(),
            containers: Vec::new(),
            status_message: None,
            labels: vec!["app=api".to_owned()],
            host_network: false,
        }
    }

    fn row(pod: &PodSummary) -> PodRow<'_> {
        PodRow { pod, usage: None }
    }

    #[test]
    fn pod_row_values_follow_columns() {
        let pod = pod();
        let row = row(&pod);
        assert!(matches!(
            row.value(NAME),
            CellValue::Qualified {
                prefix: Some("payments"),
                text: "api-7"
            }
        ));
        assert!(matches!(row.value(STATUS), CellValue::Status { .. }));
        assert!(matches!(row.value(READY), CellValue::Text(text) if text == "3/4"));
        assert!(matches!(row.value(RESTARTS), CellValue::Number(12)));
        assert!(matches!(row.value(NODE), CellValue::Text(text) if text == "wk-03"));
        assert!(matches!(row.value(AGE), CellValue::Age(None)));
        assert!(matches!(row.value(POD_COLUMNS.len()), CellValue::Absent));
        let unscheduled = PodSummary {
            node_name: None,
            ..pod
        };
        assert!(matches!(
            self::row(&unscheduled).value(NODE),
            CellValue::Absent
        ));
    }

    #[test]
    fn pod_row_reads_labels_and_scope() {
        let pod = pod();
        let row = row(&pod);
        assert_eq!(row.namespace(), Some("payments"));
        assert_eq!(row.name(), "api-7");
        assert_eq!(row.labels().collect::<Vec<_>>(), ["app=api"]);
    }

    #[test]
    fn pod_row_usage_values_sort_as_numbers() {
        let pod = pod();
        let usage = ResourceUsage {
            cpu: cluster::CpuAmount::from_nanocores(310_000_000),
            memory: cluster::ByteAmount::from_bytes(498 << 20),
        };
        let with_usage = PodRow {
            pod: &pod,
            usage: Some(usage),
        };
        assert!(matches!(
            with_usage.value(CPU),
            CellValue::Number(310_000_000)
        ));
        assert!(matches!(
            with_usage.value(MEMORY),
            CellValue::Number(522_190_848)
        ));
        let without = row(&pod);
        assert!(matches!(without.value(CPU), CellValue::Absent));
        assert!(matches!(without.value(MEMORY), CellValue::Absent));
    }

    #[test]
    fn pods_view_hides_cpu_by_default() {
        let view = pods_view();
        assert_eq!(view.hidden.iter().copied().collect::<Vec<_>>(), [CPU]);
    }

    #[test]
    fn pod_rows_keep_session_order_and_attach_usage() {
        let first = pod();
        let second = PodSummary {
            name: "api-8".to_owned(),
            ..pod()
        };
        let mut history = PodUsageHistory::default();
        let usage = ResourceUsage::default();
        history.record(
            jiff::Timestamp::UNIX_EPOCH,
            &[cluster::PodMetrics {
                namespace: "payments".to_owned(),
                name: "api-8".to_owned(),
                sampled_at: None,
                containers: vec![cluster::ContainerMetrics {
                    name: "app".to_owned(),
                    usage,
                }],
            }],
            &[],
        );
        let pods = [first, second];
        let rows = pod_rows(&pods, Some(&history));
        let names: Vec<_> = rows.iter().map(|row| row.pod.name.as_str()).collect();
        assert_eq!(names, ["api-7", "api-8"]);
        assert_eq!(rows[0].usage, None);
        assert_eq!(rows[1].usage, Some(usage));
        assert!(pod_rows(&pods, None).iter().all(|row| row.usage.is_none()));
    }
}
