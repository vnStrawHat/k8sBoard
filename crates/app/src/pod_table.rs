use std::borrow::Cow;

use cluster::PodSummary;
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
use crate::resource_actions::pod_menu;
use crate::resource_kind::{Align, KindColumn, column};
use crate::status_tone::{StatusTone, pod_status_label, toned_text};
use crate::table_filter::FilterPreset;
use crate::table_layout::{ColumnPlan, TableLayout, clickable_row, header_cell, select_cell};
use crate::table_view::{CellValue, FilteredTable, RowCheck, TableRow, TableView, default_filter};

const NAME: usize = 0;
const STATUS: usize = 1;
const READY: usize = 2;
const RESTARTS: usize = 3;
pub(crate) const NODE: usize = 4;
const AGE: usize = 5;

const NAME_MIN_WIDTH: Pixels = px(160.);

/// The Name column takes the rest of the width: pod names are the longest values.
const POD_COLUMNS: [KindColumn; 6] = [
    column("Name", 160., Align::Left),
    column("Status", 170., Align::Left),
    column("Ready", 70., Align::Left),
    column("Restarts", 80., Align::Right),
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
            view: TableView::new(default_filter(Screen::Pods)),
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

impl TableRow for PodSummary {
    fn namespace(&self) -> Option<&str> {
        Some(&self.namespace)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        self.labels.iter().map(String::as_str)
    }

    fn tone(&self) -> StatusTone {
        pod_status_label(self).tone
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        match column {
            NAME => CellValue::Qualified {
                prefix: Some(&self.namespace),
                text: &self.name,
            },
            STATUS => {
                let label = pod_status_label(self);
                CellValue::Status {
                    tone: label.tone,
                    text: label.text,
                }
            }
            READY => CellValue::Text(Cow::Owned(self.ready.to_string())),
            RESTARTS => CellValue::Number(i64::from(self.restarts)),
            NODE => self.node_name.as_deref().map_or(CellValue::Absent, |node| {
                CellValue::Text(Cow::Borrowed(node))
            }),
            AGE => CellValue::Age(self.created_at),
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
        let pods = self.pods(cx);
        self.view.apply_check(pods, change);
    }

    fn rebuild_view(&mut self, cx: &App) -> bool {
        let pods = self.pods(cx);
        self.view
            .rebuild(pods, POD_COLUMNS.len(), jiff::Timestamp::now());
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
            self.layout.columns.is_select(col_ix) && self.view.all_checked(self.pods(cx));
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
                .is_some_and(|pod| self.view.is_checked(pod));
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
        }
    }

    #[test]
    fn pod_row_values_follow_columns() {
        let pod = pod();
        assert!(matches!(
            pod.value(NAME),
            CellValue::Qualified {
                prefix: Some("payments"),
                text: "api-7"
            }
        ));
        assert!(matches!(pod.value(STATUS), CellValue::Status { .. }));
        assert!(matches!(pod.value(READY), CellValue::Text(text) if text == "3/4"));
        assert!(matches!(pod.value(RESTARTS), CellValue::Number(12)));
        assert!(matches!(pod.value(NODE), CellValue::Text(text) if text == "wk-03"));
        assert!(matches!(pod.value(AGE), CellValue::Age(None)));
        assert!(matches!(pod.value(POD_COLUMNS.len()), CellValue::Absent));
        let unscheduled = PodSummary {
            node_name: None,
            ..pod
        };
        assert!(matches!(unscheduled.value(NODE), CellValue::Absent));
    }

    #[test]
    fn pod_row_reads_labels_and_scope() {
        let pod = pod();
        assert_eq!(pod.namespace(), Some("payments"));
        assert_eq!(pod.name(), "api-7");
        assert_eq!(pod.labels().collect::<Vec<_>>(), ["app=api"]);
    }
}
