//! The Issues table: one row per issue the engine found, newest problems last. A click reveals
//! the object on its own screen with its drawer, so this screen has no drawer.

use std::borrow::Cow;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, ClipboardItem, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Stateful, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    Window, div, px,
};

use crate::age::format_age;
use crate::app_shell::{AppShell, Screen};
use crate::cluster_session::ClusterSession;
use crate::drawer::truncated_text;
use crate::event_rows::message_line;
use crate::filter_bar::filtered_empty_state;
use crate::issue::{Issue, IssueAction};
use crate::log_dock::LogDock;
use crate::resource_actions::{disabled_menu_item, view_logs_item};
use crate::resource_kind::{Align, KindColumn, column};
use crate::status_tone::{StatusLabel, StatusTone, toned_text};
use crate::table_filter::FilterPreset;
use crate::table_layout::{ColumnPlan, TableLayout, header_cell};
use crate::table_view::{CellValue, FilteredTable, RowCheck, TableRow, TableView, default_filter};

const SEVERITY: usize = 0;
const REASON: usize = 1;
const KIND: usize = 2;
const OBJECT: usize = 3;
const NAMESPACE: usize = 4;
const CAUSE: usize = 5;
const COUNT: usize = 6;
const AGE: usize = 7;

/// Marks a value the issue does not have.
const ABSENT: &str = "—";

const CAUSE_MIN_WIDTH: Pixels = px(200.);

/// The Cause column takes the rest of the width: it holds the longest text.
const ISSUE_COLUMNS: [KindColumn; 8] = [
    column("Severity", 80., Align::Left),
    column("Reason", 170., Align::Left),
    column("Kind", 110., Align::Left),
    column("Object", 260., Align::Left),
    column("Namespace", 120., Align::Left),
    column("Cause", 300., Align::Left),
    column("Count", 64., Align::Right),
    column("Age", 60., Align::Right),
];

pub(crate) struct IssueTableDelegate {
    session: Option<Entity<ClusterSession>>,
    /// The row menu's View logs opens a tab here.
    dock: WeakEntity<LogDock>,
    /// A click reveals the object through the shell.
    shell: WeakEntity<AppShell>,
    layout: TableLayout,
    view: TableView,
}

impl IssueTableDelegate {
    pub(crate) fn new(dock: WeakEntity<LogDock>, shell: WeakEntity<AppShell>) -> Self {
        Self {
            session: None,
            dock,
            shell,
            layout: TableLayout::new(ColumnPlan {
                specs: ISSUE_COLUMNS.to_vec(),
                flexible: CAUSE,
                flexible_min: CAUSE_MIN_WIDTH,
            }),
            view: TableView::new(default_filter(Screen::Issues)),
        }
    }

    /// Resizes the Cause column for a table `table_width` wide. Returns whether the columns
    /// changed, so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels) -> bool {
        self.layout.fit_width(table_width, &self.view.hidden)
    }

    pub(crate) fn set_session(&mut self, session: Option<Entity<ClusterSession>>) {
        self.session = session;
    }

    /// The issues in board order, so item indices index the board; none without a live session.
    fn issues<'a>(&self, cx: &'a App) -> &'a [Issue] {
        let Some(session) = &self.session else {
            return &[];
        };
        let session = session.read(cx);
        if session.live().is_none() {
            return &[];
        }
        session.issues().issues()
    }

    /// The issue shown at table row `row_ix`.
    fn issue_at<'a>(&self, row_ix: usize, cx: &'a App) -> Option<&'a Issue> {
        self.issues(cx).get(self.view.item_index(row_ix)?)
    }
}

impl TableRow for Issue {
    fn namespace(&self) -> Option<&str> {
        self.shown.namespace.as_deref()
    }

    fn name(&self) -> &str {
        &self.shown.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        std::iter::empty()
    }

    fn tone(&self) -> StatusTone {
        self.severity.tone()
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        match column {
            SEVERITY => CellValue::Status {
                tone: self.severity.tone(),
                text: self.severity.label().into(),
            },
            REASON => CellValue::Text(Cow::Borrowed(&self.reason)),
            // The short label shows; the full name still matches the quick filter.
            KIND => CellValue::Qualified {
                prefix: Some(&self.shown.kind),
                text: short_kind(&self.shown.kind),
            },
            OBJECT => CellValue::Text(Cow::Borrowed(&self.shown.name)),
            NAMESPACE => self
                .shown
                .namespace
                .as_deref()
                .map_or(CellValue::Absent, |namespace| {
                    CellValue::Text(Cow::Borrowed(namespace))
                }),
            CAUSE => CellValue::Text(Cow::Borrowed(&self.cause)),
            COUNT => CellValue::Number(i64::try_from(self.count).unwrap_or(i64::MAX)),
            AGE => CellValue::Age(Some(self.since)),
            _ => CellValue::Absent,
        }
    }

    fn in_preset(&self, _: &FilterPreset) -> bool {
        true
    }
}

impl FilteredTable for IssueTableDelegate {
    fn view(&self) -> Option<&TableView> {
        Some(&self.view)
    }

    fn view_mut(&mut self) -> Option<&mut TableView> {
        Some(&mut self.view)
    }

    fn column_plan(&self) -> Option<&ColumnPlan> {
        Some(&self.layout.plan)
    }

    /// Issues have no checkbox: nothing acts on several of them.
    fn check_rows(&mut self, _: RowCheck, _: &App) {}

    fn rebuild_view(&mut self, cx: &App) -> bool {
        let issues = self.issues(cx);
        self.view
            .rebuild(issues, ISSUE_COLUMNS.len(), jiff::Timestamp::now());
        self.layout.relayout(&self.view.hidden)
    }
}

impl TableDelegate for IssueTableDelegate {
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
        // The table opens with a checkbox column; an issue has no checkbox, so its header is blank.
        if self.layout.columns.is_select(col_ix) {
            return div().size_full().into_any_element();
        }
        header_cell(&self.layout, self.view.sort, false, &self.shell, col_ix, cx)
    }

    /// A plain click opens the object. The toolkit's own click only highlights the row, which is
    /// also all the arrow keys do.
    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let shell = self.shell.clone();
        div().id(("row", row_ix)).on_click(move |event, _, cx| {
            let modifiers = event.modifiers();
            if modifiers.secondary() || modifiers.shift {
                return;
            }
            let _ = shell.update(cx, |shell, cx| shell.reveal_issue(row_ix, cx));
        })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let (Some(issue), Some(logical)) = (
            self.issue_at(row_ix, cx),
            self.layout.columns.logical(col_ix),
        ) else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        match logical {
            SEVERITY => toned_text(
                StatusLabel {
                    text: issue.severity.label().into(),
                    tone: issue.severity.tone(),
                },
                cx,
            )
            .into_any_element(),
            REASON => toned_text(
                StatusLabel {
                    text: issue.reason.clone(),
                    tone: issue.severity.tone(),
                },
                cx,
            )
            .truncate()
            .into_any_element(),
            KIND => div()
                .truncate()
                .child(short_kind(&issue.shown.kind).to_owned())
                .into_any_element(),
            OBJECT => object_cell(issue, row_ix, mono, cx),
            NAMESPACE => match &issue.shown.namespace {
                Some(namespace) => div().truncate().child(namespace.clone()).into_any_element(),
                None => absent(cx),
            },
            CAUSE => truncated_text(("issue-cause", row_ix), message_line(&issue.cause))
                .into_any_element(),
            COUNT => match count_text(issue.count) {
                None => absent_right(cx),
                Some(count) => div()
                    .w_full()
                    .text_right()
                    .font_family(mono)
                    .child(count)
                    .into_any_element(),
            },
            AGE => div()
                .w_full()
                .text_right()
                .font_family(mono)
                // Read per cell: a render has no shared clock, and a second of skew is invisible.
                .child(format_age(Some(issue.since), jiff::Timestamp::now()))
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
        let (Some(session), Some(issue)) = (&self.session, self.issue_at(row_ix, cx)) else {
            return menu;
        };
        let Some(live) = session.read(cx).live() else {
            return menu;
        };
        let menu = menu.item(open_item(issue, &self.shell));
        let menu = match logs_pod(issue, live.pods.items()) {
            Some(pod) => {
                let container = issue.container.as_deref();
                menu.item(view_logs_item(pod, container, live, &self.dock))
            }
            None => menu,
        };
        menu.separator().item(copy_name_item(issue))
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        filtered_empty_state(
            &self.view,
            "No issues".to_owned(),
            "issues",
            &self.shell,
            cx,
        )
    }

    fn loading(&self, _: &App) -> bool {
        false
    }
}

/// ` · container api`, or nothing when the issue is not about one container.
fn container_suffix(issue: &Issue) -> Option<String> {
    issue
        .container
        .as_ref()
        .map(|container| format!(" · container {container}"))
}

/// The count of a group; nothing for a single object.
fn count_text(count: usize) -> Option<String> {
    (count > 1).then(|| count.to_string())
}

/// The name in mono, then the container the issue is about, muted.
fn object_cell(
    issue: &Issue,
    row_ix: usize,
    mono: gpui_kit::SharedString,
    cx: &App,
) -> gpui_kit::AnyElement {
    let container = container_suffix(issue).map(|suffix| {
        div()
            .flex_shrink_0()
            .text_color(cx.theme().muted_foreground)
            .child(suffix)
    });
    h_flex()
        .w_full()
        .font_family(mono)
        .child(truncated_text(("issue-object", row_ix), issue.shown.name.clone()).min_w_0())
        .children(container)
        .into_any_element()
}

/// A muted dash for a value the issue does not have.
fn absent(cx: &App) -> gpui_kit::AnyElement {
    div()
        .text_color(cx.theme().muted_foreground)
        .child(ABSENT)
        .into_any_element()
}

/// The dash in a right-aligned column.
fn absent_right(cx: &App) -> gpui_kit::AnyElement {
    div()
        .w_full()
        .text_right()
        .text_color(cx.theme().muted_foreground)
        .child(ABSENT)
        .into_any_element()
}

/// The kind as the Kind column shows it: the long names of the policy kinds shortened to what
/// people say (`HPA`, `PDB`, `PVC`).
fn short_kind(kind: &str) -> &str {
    match kind {
        "HorizontalPodAutoscaler" => "HPA",
        "PodDisruptionBudget" => "PDB",
        "PersistentVolumeClaim" => "PVC",
        "PersistentVolume" => "PV",
        "ResourceQuota" => "Quota",
        other => other,
    }
}

/// `Open deployment`: reveals the object on its screen. Disabled when k8sBoard has no screen for
/// the kind.
fn open_item(issue: &Issue, shell: &WeakEntity<AppShell>) -> PopupMenuItem {
    let label = format!("Open {}", issue.shown.kind.to_lowercase());
    let Some(target) = issue.target.clone() else {
        let reason = format!("No screen for {}", issue.shown.kind);
        return disabled_menu_item(label, reason.into());
    };
    let shell = shell.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        let _ = shell.update(cx, |shell, cx| shell.reveal(target.clone(), cx));
    })
}

/// The pod View logs reads: the issue's subject, when the action asks for logs and the pod is
/// still in the list.
fn logs_pod<'a>(issue: &Issue, pods: &'a [cluster::PodSummary]) -> Option<&'a cluster::PodSummary> {
    if !matches!(issue.action, IssueAction::ViewLogs { .. }) || issue.subject.kind != "Pod" {
        return None;
    }
    let namespace = issue.subject.namespace.as_deref()?;
    pods.iter()
        .find(|pod| pod.namespace == namespace && pod.name == issue.subject.name)
}

fn copy_name_item(issue: &Issue) -> PopupMenuItem {
    let name = issue.shown.name.clone();
    PopupMenuItem::new("Copy object name").on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
    })
}

#[cfg(test)]
#[path = "issue_table_tests.rs"]
mod issue_table_tests;
