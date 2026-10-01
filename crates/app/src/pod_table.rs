use cluster::PodSummary;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, HighlightStyle, IntoElement, ParentElement as _, Pixels,
    SharedString, Styled as _, StyledText, WeakEntity, Window, div, px,
};

use crate::age::format_age;
use crate::cluster_session::ClusterSession;
use crate::log_dock::LogDock;
use crate::resource_actions::pod_menu;
use crate::status_tone::{pod_status_label, toned_text};
use crate::table_layout::{flexible_width, header_cell};

const NAME: usize = 0;
const STATUS: usize = 1;
const READY: usize = 2;
const RESTARTS: usize = 3;
const NODE: usize = 4;
const AGE: usize = 5;

const NAME_MIN_WIDTH: Pixels = px(160.);
/// The Name column takes the rest of the width: pod names are the longest values.
const FIXED_WIDTH: Pixels = px(170. + 70. + 80. + 180. + 70.);

/// Rows come straight from the session, so the table never owns a copy of the pods.
pub(crate) struct PodTableDelegate {
    session: Option<Entity<ClusterSession>>,
    log_dock: WeakEntity<LogDock>,
    columns: Vec<Column>,
}

fn columns(name_width: Pixels) -> Vec<Column> {
    vec![
        Column::new("name", "Name")
            .width(name_width)
            .min_width(NAME_MIN_WIDTH),
        Column::new("status", "Status").width(px(170.)),
        Column::new("ready", "Ready").width(px(70.)),
        Column::new("restarts", "Restarts")
            .width(px(80.))
            .text_right(),
        Column::new("node", "Node").width(px(180.)),
        Column::new("age", "Age").width(px(70.)).text_right(),
    ]
}

impl PodTableDelegate {
    pub(crate) fn new(log_dock: WeakEntity<LogDock>) -> Self {
        Self {
            session: None,
            log_dock,
            columns: columns(NAME_MIN_WIDTH),
        }
    }

    /// Resizes the Name column for a table `table_width` wide. Returns whether it changed,
    /// so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels) -> bool {
        let name_width = flexible_width(table_width, FIXED_WIDTH, NAME_MIN_WIDTH);
        if self
            .columns
            .get(NAME)
            .is_some_and(|column| column.width == name_width)
        {
            return false;
        }
        self.columns = columns(name_width);
        true
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

    fn scope_label(&self, cx: &App) -> String {
        self.session
            .as_ref()
            .and_then(|session| session.read(cx).live())
            .map_or_else(String::new, |live| live.scope_label())
    }
}

impl TableDelegate for PodTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, cx: &App) -> usize {
        self.pods(cx).len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns.get(col_ix).cloned().unwrap_or_default()
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        match self.columns.get(col_ix) {
            Some(column) => header_cell(column, cx),
            None => div().size_full(),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(pod) = self.pods(cx).get(row_ix) else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        match col_ix {
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
        match live.pods.items().get(row_ix) {
            Some(pod) => pod_menu(menu, pod, live, &self.log_dock),
            None => menu,
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        h_flex()
            .size_full()
            .justify_center()
            .items_center()
            .text_color(cx.theme().muted_foreground)
            .child(format!("No pods in {}", self.scope_label(cx)))
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
