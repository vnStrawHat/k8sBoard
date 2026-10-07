use std::borrow::Cow;
use std::collections::BTreeSet;

use cluster::{ContainerKind, PodSummary, ResourceUsage};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::{
    AnyElement, App, Context, Div, IntoElement, ParentElement as _, Pixels, SharedString, Stateful,
    Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _,
};

use crate::age::format_age;
use crate::app_shell::table_export::table_csv;
use crate::app_shell::{AppShell, Screen};
use crate::cell_truncation::{NameScope, mono_capacity, plain_text, scoped_name_text};
use crate::dock::Dock;
use crate::drawer::truncated_text_with_tooltip;
use crate::filter_bar::filtered_empty_state;
use crate::kind_row::KindCell;
use crate::metrics_history::PodUsageHistory;
use crate::port_forward_menu::{ForwardMenu, pod_subject};
use crate::resource_actions::{LogsMenu, PodMenuItems, PodMenuLinks, ShellMenu, pod_menu};
use crate::resource_kind::{Align, IMAGE_COLUMN_NAME, KindColumn, column};
use crate::row_context::TableSession;
use crate::settings::TablePrefs;
use crate::status_tone::{StatusTone, pod_status_label, tone_color, toned_text};
use crate::table_filter::FilterPreset;
use crate::table_layout::{
    ColumnPlan, TableLayout, centered_cell, clickable_row, header_cell, select_cell,
};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::table_view::{
    CellValue, FilteredTable, RowCheck, TableRow, TableView, cell_text, default_filter,
};
use crate::usage_format::Measure;

const NAME: usize = 0;
pub(crate) const STATUS: usize = 1;
const READY: usize = 2;
const RESTARTS: usize = 3;
const CPU: usize = 4;
const MEMORY: usize = 5;
pub(crate) const NODE: usize = 6;
const IMAGE: usize = 7;
const AGE: usize = 8;

/// Name takes most of the spare width, up to a cap: pod names are the longest values. Status is
/// sized to its longest label (`CrashLoopBackOff`) and Node to a short host name, so the width
/// they would keep unused goes to Name. At 1024 px Age, then Memory, then CPU are shed (`sheds`) so
/// the table needs no horizontal scroll and Name keeps about 28 characters.
const POD_COLUMNS: [KindColumn; 9] = [
    column("Name", 280., Align::Left).grows(4).up_to(640.),
    column("Status", 160., Align::Left),
    column("Ready", 70., Align::Left),
    column("Restarts", 80., Align::Right),
    column("CPU", 70., Align::Right).sheds(3),
    column("Memory", 80., Align::Right).sheds(2),
    column("Node", 110., Align::Left).grows(2).up_to(260.),
    column(IMAGE_COLUMN_NAME, 200., Align::Left).grows(1),
    column("Age", 70., Align::Right).sheds(1),
];

/// Rows come straight from the sessions, so the table never owns a copy of the pods.
pub(crate) struct PodTableDelegate {
    /// The open cluster's session; `None` before the first one.
    session: Option<TableSession>,
    /// Every shown row is ticked: read by the header checkbox, computed with the rows (a rebuild or
    /// a tick) so a frame never reads the rows again.
    all_checked: bool,
    dock: WeakEntity<Dock>,
    /// The row menu's "View YAML" opens the drawer through the shell.
    shell: WeakEntity<AppShell>,
    layout: TableLayout,
    view: TableView,
}

fn pod_plan() -> ColumnPlan {
    ColumnPlan {
        specs: POD_COLUMNS.to_vec(),
        flexible: NAME,
    }
}

impl PodTableDelegate {
    pub(crate) fn new(
        dock: WeakEntity<Dock>,
        shell: WeakEntity<AppShell>,
        saved: Option<&TablePrefs>,
    ) -> Self {
        let plan = pod_plan();
        let mut view = TableView::new(default_filter(Screen::Pods));
        // The Image column is opt-in from the Columns menu; a saved choice replaces this.
        view.hidden = BTreeSet::from([IMAGE]);
        if let Some(saved) = saved {
            view.apply_prefs(saved, &plan);
        }
        Self {
            session: None,
            all_checked: false,
            dock,
            shell,
            layout: TableLayout::new(plan),
            view,
        }
    }

    /// Resizes the Name column for a table `table_width` wide. Returns whether the columns
    /// changed, so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels) -> bool {
        self.layout.fit_width(table_width, &self.view.hidden)
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

    /// Whether every shown row is ticked, as of the last rebuild or tick.
    #[cfg(test)]
    pub(crate) fn all_checked(&self) -> bool {
        self.all_checked
    }

    /// The ticked pods in display order, in the open cluster: what a bulk Delete covers.
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
            .map(|row| ClusterObject::new(session.cluster.clone(), ResourceKey::of_pod(row.pod)))
            .collect()
    }

    /// The pods of the open cluster with their newest usage, so item indices still index the
    /// session list.
    fn rows<'a>(&self, cx: &'a App) -> Vec<PodRow<'a>> {
        let live = self
            .session
            .as_ref()
            .and_then(|session| session.session.read(cx).live());
        let pods = live.map_or(&[][..], |live| live.pods.items());
        pod_rows(pods, live.map(|live| &live.metrics.pods.history))
    }

    /// The pod shown at table row `row_ix`, and the session it belongs to.
    fn pod_at<'a>(&self, row_ix: usize, cx: &'a App) -> Option<(&TableSession, &'a PodSummary)> {
        let session = self.session.as_ref()?;
        let live = session.session.read(cx).live()?;
        let item = self.view.item_index(row_ix)?;
        Some((session, live.pods.items().get(item)?))
    }

    /// The pods the table shows, in row order.
    fn shown_pods<'a>(&'a self, cx: &'a App) -> impl Iterator<Item = &'a PodSummary> {
        let pods = self
            .session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .map_or(&[][..], |live| live.pods.items());
        self.view.rows().iter().filter_map(|&item| pods.get(item))
    }

    fn scope_label(&self, cx: &App) -> String {
        self.session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .map_or_else(String::new, |live| live.scope_label())
    }

    /// Whether the title-bar scope is one namespace, so the Name column drops `namespace/`.
    fn name_scope(&self, cx: &App) -> NameScope {
        self.session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .map_or(NameScope::SeveralNamespaces, |live| {
                NameScope::of(&live.scope)
            })
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

/// The main containers' images; init and sidecar images are not what the pod runs.
fn image_cell(pod: &PodSummary) -> KindCell {
    KindCell::images(
        pod.containers
            .iter()
            .filter(|container| container.kind == ContainerKind::Main)
            .map(|container| container.image.as_str()),
    )
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

    fn images(&self) -> impl Iterator<Item = &str> {
        self.pod
            .containers
            .iter()
            .map(|container| container.image.as_str())
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
            IMAGE => match image_cell(pod) {
                KindCell::Images { text, .. } => CellValue::Text(Cow::Owned(text.to_string())),
                _ => CellValue::Absent,
            },
            AGE => CellValue::Age(pod.created_at),
            _ => CellValue::Absent,
        }
    }

    fn in_preset(&self, _: &FilterPreset) -> bool {
        true
    }

    /// The usage as the table writes it (`120m`, `64 MiB`), not as the raw number it sorts by.
    fn export_text(&self, column: usize, now: jiff::Timestamp) -> String {
        match (column, self.usage) {
            (CPU, Some(usage)) => Measure::Cpu.format(usage.cpu.cores()),
            (MEMORY, Some(usage)) => Measure::Bytes.format(usage.memory.bytes() as f64),
            _ => cell_text(self.value(column), now),
        }
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
        self.all_checked = self.view.all_checked(&rows);
    }

    fn export_csv(&self, now: jiff::Timestamp, cx: &App) -> Option<String> {
        Some(table_csv(
            &self.view,
            &self.layout.plan,
            &self.rows(cx),
            now,
        ))
    }

    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "PodTableDelegate")
    )]
    fn rebuild_view(&mut self, cx: &App) -> bool {
        let rows = self.rows(cx);
        self.view
            .rebuild(&rows, self.layout.plan.specs.len(), jiff::Timestamp::now());
        self.all_checked = self.view.all_checked(&rows);
        self.layout.relayout(&self.view.hidden)
    }
}

impl PodTableDelegate {
    /// The content of one body cell; the trait method centres it.
    fn cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        cx: &mut Context<TableState<Self>>,
    ) -> AnyElement {
        if self.layout.columns.is_select(col_ix) {
            let is_checked = self
                .pod_at(row_ix, cx)
                .is_some_and(|(_, pod)| self.view.is_checked(&PodRow { pod, usage: None }));
            return select_cell(row_ix, is_checked, &self.shell);
        }
        let capacity = mono_capacity(self.layout.columns.columns.get(col_ix), cx);
        let (Some((session, pod)), Some(logical)) =
            (self.pod_at(row_ix, cx), self.layout.columns.logical(col_ix))
        else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        match logical {
            NAME => scoped_name_text(
                ("pod-name", row_ix),
                Some(&pod.namespace),
                &pod.name,
                self.name_scope(cx),
                capacity,
                self.shown_pods(cx)
                    .map(|pod| (Some(pod.namespace.as_str()), pod.name.as_str())),
                cx,
            ),
            STATUS => toned_text(pod_status_label(pod), cx).into_any_element(),
            READY => div()
                .font_family(mono)
                .child(pod.ready.to_string())
                .into_any_element(),
            RESTARTS => div()
                .w_full()
                .text_right()
                .font_family(mono)
                .when(pod.restarts > 0, |this| {
                    this.text_color(tone_color(StatusTone::Warn, cx))
                })
                .child(pod.restarts.to_string())
                .into_any_element(),
            CPU | MEMORY => {
                let usage =
                    session.session.read(cx).live().and_then(|live| {
                        live.metrics.pods.history.latest(&pod.namespace, &pod.name)
                    });
                let text = usage.map(|usage| match logical {
                    CPU => Measure::Cpu.format(usage.cpu.cores()),
                    _ => Measure::Bytes.format(usage.memory.bytes() as f64),
                });
                usage_cell(text, mono, cx)
            }
            NODE => match &pod.node_name {
                Some(node_name) => plain_text(
                    ("pod-node", row_ix),
                    node_name,
                    node_name,
                    capacity,
                    self.shown_pods(cx)
                        .filter_map(|pod| pod.node_name.as_deref())
                        .map(|node_name| (None, node_name)),
                    cx,
                ),
                None => dash_cell(cx),
            },
            IMAGE => match image_cell(pod) {
                KindCell::Images { text, all } => {
                    truncated_text_with_tooltip(("pod-image", row_ix), text, all)
                        .w_full()
                        .font_family(mono)
                        .into_any_element()
                }
                _ => dash_cell(cx),
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
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        // The submenus are built from the app, so they are made before the session is borrowed.
        let prepared = {
            let Some((open, pod)) = self.pod_at(row_ix, cx) else {
                return menu;
            };
            let session = open.session.read(cx);
            let (Some(live), Some(guard)) = (session.live(), session.guard(cx)) else {
                return menu;
            };
            (
                open.row_context(cx),
                LogsMenu::of(pod, &live.access),
                live.connection().clone(),
                ShellMenu::of(pod, &guard),
                ForwardMenu::of(pod_subject(pod), &open.cluster, &guard),
            )
        };
        let (row, logs_menu, connection, shell_menu, forward_menu) = prepared;
        let shell_items = shell_menu.items(&row, &self.shell, window, cx);
        let items = PodMenuItems {
            view_logs: logs_menu.item(connection, &row, &self.dock, window, cx),
            open_shell: shell_items.open_shell,
            debug_container: shell_items.debug_container,
            port_forward: forward_menu.item(&self.shell, window, cx),
        };
        let Some((open, pod)) = self.pod_at(row_ix, cx) else {
            return menu;
        };
        let session = open.session.read(cx);
        let (Some(_), Some(guard)) = (session.live(), session.guard(cx)) else {
            return menu;
        };
        pod_menu(
            menu,
            pod,
            &guard,
            &row,
            &PodMenuLinks {
                dock: &self.dock,
                shell: &self.shell,
            },
            items,
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let empty = format!("No pods in {}", self.scope_label(cx));
        filtered_empty_state(&self.view, empty, "pods", &self.shell, cx)
    }

    /// Loading while the session is live and still waits for its first list.
    fn loading(&self, cx: &App) -> bool {
        self.session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .is_some_and(|live| live.pods.is_loading())
    }
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

    /// The names of the columns shown by default in a window `window` px wide (less the sidebar).
    fn default_columns_at(window: f32) -> Vec<String> {
        let plan = pod_plan();
        let hidden = BTreeSet::from([IMAGE]);
        crate::table_layout::layout_columns(
            &plan.specs,
            plan.flexible,
            gpui_kit::px(window) - crate::navigation::SIDEBAR_WIDTH,
            &hidden,
        )
        .columns
        .iter()
        .map(|column| column.name.to_string())
        .collect()
    }

    #[test]
    fn at_1320_px_the_pod_table_keeps_every_default_column() {
        assert_eq!(default_columns_at(1320.).len(), 1 + 8);
    }

    #[test]
    fn at_1024_px_the_pod_table_sheds_age_memory_and_cpu_to_avoid_a_horizontal_scroll() {
        let names = default_columns_at(1024.);
        assert!(!names.contains(&"Age".to_owned()), "{names:?}");
        assert!(!names.contains(&"Memory".to_owned()), "{names:?}");
        assert!(!names.contains(&"CPU".to_owned()), "{names:?}");
        assert!(names.contains(&"Name".to_owned()));
        assert!(names.contains(&"Status".to_owned()));
        assert!(names.contains(&"Node".to_owned()));
    }

    fn pod() -> PodSummary {
        PodSummary {
            annotations: cluster::AnnotationTerms::default(),
            is_finished: false,
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
            image_pull_secrets: Vec::new(),
            node_selector: Vec::new(),
            node_affinity: Vec::new(),
        }
    }

    fn container(kind: ContainerKind, image: &str) -> cluster::ContainerSummary {
        cluster::ContainerSummary {
            terminal: cluster::ContainerTerminal::None,
            name: "c".to_owned(),
            image: image.to_owned(),
            kind,
            state: cluster::ContainerState::Running { started_at: None },
            is_ready: true,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: Vec::new(),
            probes: cluster::ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: Vec::new(),
        }
    }

    fn pod_with_images() -> PodSummary {
        PodSummary {
            containers: vec![
                container(ContainerKind::Init, "busybox:1"),
                container(ContainerKind::Main, "reg.io/team/nginx:1.27"),
                container(ContainerKind::Main, "envoyproxy/envoy:v1"),
            ],
            ..pod()
        }
    }

    #[test]
    fn the_image_column_shows_the_main_containers_and_the_filter_searches_all() {
        let pod = pod_with_images();
        let row = row(&pod);
        assert!(matches!(row.value(IMAGE), CellValue::Text(text) if text == "nginx:1.27 +1"));
        let images: Vec<_> = row.images().collect();
        assert_eq!(
            images,
            ["busybox:1", "reg.io/team/nginx:1.27", "envoyproxy/envoy:v1"]
        );
        assert!(matches!(
            self::row(&self::pod()).value(IMAGE),
            CellValue::Absent
        ));
    }

    fn row(pod: &PodSummary) -> PodRow<'_> {
        PodRow { pod, usage: None }
    }

    #[test]
    fn hide_system_ignores_pods_and_nodes() {
        let mut pod = pod();
        pod.name = "system:odd".to_owned();
        assert!(row(&pod).in_preset(&FilterPreset::HideSystem));
        let node = cluster::NodeSummary {
            name: "system:odd".to_owned(),
            status: cluster::NodeStatus {
                readiness: cluster::NodeReadiness::Ready,
                scheduling: cluster::NodeScheduling::Enabled,
            },
            roles: Vec::new(),
            taints: Vec::new(),
            kubelet_version: String::new(),
            internal_ip: None,
            created_at: None,
            conditions: Vec::new(),
            addresses: Vec::new(),
            system: cluster::NodeSystemInfo::default(),
            resources: Vec::new(),
            labels: Vec::new(),
        };
        let node_row = crate::node_table::NodeRow {
            node: &node,
            usage: crate::node_usage::NodeUsage::default(),
            requests: crate::node_usage::NodeUsage::default(),
        };
        assert!(node_row.in_preset(&FilterPreset::HideSystem));
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
            is_finished: false,
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
    fn new_pods_table_hides_only_the_image_column() {
        let table =
            PodTableDelegate::new(WeakEntity::new_invalid(), WeakEntity::new_invalid(), None);
        assert_eq!(table.view.hidden, BTreeSet::from([IMAGE]));
    }

    #[test]
    fn pod_rows_keep_session_order_and_attach_usage() {
        let first = pod();
        let second = PodSummary {
            is_finished: false,
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
