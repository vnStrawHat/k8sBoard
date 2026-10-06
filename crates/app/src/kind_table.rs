//! The table of the explorer kinds: one delegate serves every `ResourceKind`, so a kind
//! switch only replaces the columns.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    Window, div,
};

use crate::age::format_age;
use crate::app_shell::{AppShell, Screen};
use crate::cell_truncation::{mono_capacity, qualified_text};
use crate::certificate_expiry::expiry_label;
use crate::cluster_registry::ClusterRef;
use crate::custom_rows::{date_text, date_tone};
use crate::drawer::truncated_text_with_tooltip;
use crate::filter_bar::filtered_empty_state;
use crate::kind_join::CRD_INSTANCES;
use crate::kind_row::{KindCell, KindObject, KindRow};
use crate::live_sections::{loaded_replica_sets, next_run_text};
use crate::port_forward_menu::{ForwardMenu, row_subject};
use crate::resource_actions::{
    MenuCluster, MenuExtras, browse_instances_item, browse_target, kind_menu, open_url_choice,
    open_url_menu_item, secret_menu,
};
use crate::resource_kind::{Align, NameColumn, ResourceKind, kind_columns};
use crate::row_context::TableSession;
use crate::secret_values::ValueAccess;
use crate::settings::{TablePrefs, screen_key};
use crate::status_tone::{StatusTone, tone_color, toned_text};
use crate::table_filter::FilterPreset;
use crate::table_layout::{
    ColumnPlan, TableLayout, centered_cell, clickable_row, header_cell, select_cell,
};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::table_view::{CellValue, FilteredTable, RowCheck, TableRow, TableView, default_filter};

/// The logical column of the Name column, for the kinds that show it.
const NAME: usize = 0;

/// Rows come straight from the sessions, so the table never owns a copy of the rows.
pub(crate) struct KindTableDelegate {
    /// The open cluster's session; `None` before the first one.
    session: Option<TableSession>,
    /// Every shown row of the shown kind is ticked: read by the header checkbox, computed with the
    /// rows (a rebuild or a tick) so a frame never reads the rows again.
    all_checked: bool,
    /// `None` while Pods or Nodes is shown; the table is not rendered then.
    kind: Option<ResourceKind>,
    layout: TableLayout,
    /// One view per kind, so a filter, a sort, and hidden columns survive a kind switch.
    views: HashMap<ResourceKind, TableView>,
    /// The prefs of the last run. A view created later in the session starts from them; one that
    /// already exists holds newer state.
    // ponytail: a startup copy, not live settings; it goes stale only if a CRD definition changes mid-session (column names then no longer match). Read AppSettings at view creation if that matters.
    saved: BTreeMap<String, TablePrefs>,
    /// The row menu's "Go to object" reveals a row through the shell.
    shell: WeakEntity<AppShell>,
}

/// A kind's view starts with the filter its screen starts with, and the sort and hidden columns
/// saved for it.
fn new_view(kind: ResourceKind, saved: &BTreeMap<String, TablePrefs>) -> TableView {
    let mut view = TableView::new(default_filter(Screen::Kind(kind)));
    if let Some(prefs) = saved.get(screen_key(Screen::Kind(kind))) {
        view.apply_prefs(prefs, &kind_plan(Some(kind)));
    }
    view
}
/// The logical columns of `kind`, and which one is never hidden.
fn kind_plan(kind: Option<ResourceKind>) -> ColumnPlan {
    let Some(kind) = kind else {
        return ColumnPlan {
            specs: Vec::new(),
            flexible: 0,
        };
    };
    let flexible = match kind.name_column() {
        NameColumn::Flexible => NAME,
        NameColumn::Hidden { flexible } => flexible,
    };
    ColumnPlan {
        specs: kind_columns(kind),
        flexible,
    }
}
/// The index into `KindRow::cells` that logical column `column` shows, or `None` for the Name
/// column.
fn cell_index(name_column: NameColumn, column: usize) -> Option<usize> {
    match name_column {
        NameColumn::Flexible => column.checked_sub(1),
        NameColumn::Hidden { .. } => Some(column),
    }
}

impl KindTableDelegate {
    pub(crate) fn new(
        kind: Option<ResourceKind>,
        shell: WeakEntity<AppShell>,
        saved: BTreeMap<String, TablePrefs>,
    ) -> Self {
        let views = kind
            .into_iter()
            .map(|kind| (kind, new_view(kind, &saved)))
            .collect();
        Self {
            session: None,
            all_checked: false,
            kind,
            layout: TableLayout::new(kind_plan(kind)),
            views,
            saved,
            shell,
        }
    }

    /// The session the rows come from. A tick belongs to the session it was made on: any change of
    /// the session entity, also to and from none, clears the ticks and the range anchor of every
    /// kind, so a same-named row of another cluster never inherits one. The caller refreshes the
    /// table.
    pub(crate) fn set_session(&mut self, session: Option<TableSession>) {
        let id = |session: &Option<TableSession>| {
            session.as_ref().map(|session| session.session.entity_id())
        };
        if id(&self.session) != id(&session) {
            for view in self.views.values_mut() {
                view.clear_checked();
            }
            self.all_checked = false;
        }
        self.session = session;
    }

    /// Switches the columns and returns whether the kind changed. The flexible column keeps the
    /// last table width, so it fills the table at once.
    pub(crate) fn set_kind(&mut self, kind: Option<ResourceKind>) -> bool {
        if self.kind == kind {
            return false;
        }
        self.kind = kind;
        // The next rebuild knows the ticks of the new kind.
        self.all_checked = false;
        if let Some(kind) = kind {
            self.views
                .entry(kind)
                .or_insert_with(|| new_view(kind, &self.saved));
        }
        self.layout.replace_plan(kind_plan(kind), &self.hidden());
        true
    }

    /// Restores the default filter of every kind (a context switch).
    pub(crate) fn reset_filters(&mut self) {
        for view in self.views.values_mut() {
            view.reset_filter();
        }
    }

    /// Resizes the flexible column for a table `table_width` wide. Returns whether the columns
    /// changed, so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels) -> bool {
        let hidden = self.hidden();
        self.layout.fit_width(table_width, &hidden)
    }

    fn hidden(&self) -> std::collections::BTreeSet<usize> {
        self.view()
            .map(|view| view.hidden.clone())
            .unwrap_or_default()
    }

    /// The rows of the shown kind; none while the session is not live or has no explorer for the
    /// kind yet.
    fn rows<'a>(&self, cx: &'a App) -> Vec<KindTableRow<'a>> {
        let (Some(kind), Some(session)) = (self.kind, &self.session) else {
            return Vec::new();
        };
        table_rows(session_kind_rows(session, kind, cx), kind.name_column())
    }

    /// The row shown at table row `row_ix`, and the session it belongs to.
    fn row_at<'a>(&self, row_ix: usize, cx: &'a App) -> Option<(&TableSession, &'a KindRow)> {
        let session = self.session.as_ref()?;
        let row =
            session_kind_rows(session, self.kind?, cx).get(self.view()?.item_index(row_ix)?)?;
        Some((session, row))
    }

    /// The ticked rows of the shown kind in display order, each with the cluster it came from. A
    /// bulk action reads them when it is built, never from a copy kept earlier.
    pub(crate) fn checked_rows<'a>(&'a self, cx: &'a App) -> Vec<(&'a ClusterRef, &'a KindRow)> {
        let (Some(kind), Some(session)) = (self.kind, &self.session) else {
            return Vec::new();
        };
        let Some(view) = self
            .views
            .get(&kind)
            .filter(|view| view.checked_count() > 0)
        else {
            return Vec::new();
        };
        let rows = self.rows(cx);
        let items = session_kind_rows(session, kind, cx);
        view.checked_rows(&rows)
            .into_iter()
            .filter_map(|index| Some((&session.cluster, items.get(index)?)))
            .collect()
    }

    /// The ticked rows as objects of their clusters: what a bulk Delete covers.
    pub(crate) fn checked_objects(&self, cx: &App) -> Vec<ClusterObject> {
        let Some(kind) = self.kind else {
            return Vec::new();
        };
        self.checked_rows(cx)
            .into_iter()
            .map(|(cluster, row)| {
                ClusterObject::new(cluster.clone(), ResourceKey::of_row(kind, row))
            })
            .collect()
    }

    /// Whether the row at table row `row_ix` is ticked.
    fn is_row_checked(&self, row_ix: usize, cx: &App) -> bool {
        let (Some((_, row)), Some(view), Some(kind)) =
            (self.row_at(row_ix, cx), self.view(), self.kind)
        else {
            return false;
        };
        view.is_checked(&KindTableRow {
            row,
            name_column: kind.name_column(),
        })
    }

    fn scope_label(&self, cx: &App) -> String {
        self.session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .map_or_else(String::new, |live| live.scope_label())
    }

    fn align(&self, logical: usize) -> Align {
        self.layout
            .plan
            .specs
            .get(logical)
            .map_or(Align::Left, |column| column.align)
    }
}

/// The rows of `kind` in the session's explorer; none while it is not live or the explorer shows
/// another kind.
fn session_kind_rows<'a>(session: &TableSession, kind: ResourceKind, cx: &'a App) -> &'a [KindRow] {
    let explorer = session
        .session
        .read(cx)
        .live()
        .and_then(|live| live.kind_list(kind));
    explorer.map_or(&[], |explorer| explorer.list.items())
}

/// A row with the Name layout of its kind, so a logical column maps to a cell without guessing
/// the kind from the row.
struct KindTableRow<'a> {
    row: &'a KindRow,
    name_column: NameColumn,
}

/// The rows of a kind with its Name layout, for the toolkit.
fn table_rows(rows: &[KindRow], name_column: NameColumn) -> Vec<KindTableRow<'_>> {
    rows.iter()
        .map(|row| KindTableRow { row, name_column })
        .collect()
}

impl TableRow for KindTableRow<'_> {
    fn namespace(&self) -> Option<&str> {
        self.row.namespace.as_deref()
    }

    fn name(&self) -> &str {
        &self.row.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        self.row.labels.iter().map(SharedString::as_ref)
    }

    fn tone(&self) -> StatusTone {
        self.row.status.tone
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        let Some(cell) = cell_index(self.name_column, column) else {
            return CellValue::Qualified {
                prefix: self.row.namespace.as_deref(),
                text: &self.row.name,
            };
        };
        match self.row.cells.get(cell) {
            Some(KindCell::Text(text) | KindCell::Mono(text) | KindCell::Hinted { text, .. }) => {
                CellValue::Text(Cow::Borrowed(text.as_ref()))
            }
            Some(KindCell::Qualified { prefix, text }) => CellValue::Qualified {
                prefix: prefix.as_deref(),
                text,
            },
            Some(KindCell::Toned(label)) => CellValue::Status {
                tone: label.tone,
                text: label.text.clone(),
            },
            Some(KindCell::Age { at, .. }) => CellValue::Age(*at),
            Some(KindCell::Duration {
                started_at,
                finished_at,
            }) => CellValue::Span {
                started: *started_at,
                finished: *finished_at,
            },
            // Ascending is soonest first; a schedule with no next run sorts last.
            Some(KindCell::NextRun(schedule)) => schedule
                .next_after(jiff::Timestamp::now())
                .map_or(CellValue::Absent, |next| {
                    CellValue::Number(next.timestamp().as_second())
                }),
            Some(KindCell::MonoWithMore { text, .. }) => {
                CellValue::Text(Cow::Borrowed(text.as_ref()))
            }
            Some(KindCell::Quantity { value, .. }) => {
                CellValue::Number(i64::try_from(*value).unwrap_or(i64::MAX))
            }
            Some(KindCell::Expiry { not_after }) => CellValue::Number(not_after.as_second()),
            Some(KindCell::Date { at, .. }) => CellValue::Number(at.as_second()),
            Some(KindCell::Absent) | None => CellValue::Absent,
        }
    }

    fn in_preset(&self, preset: &FilterPreset) -> bool {
        match preset {
            // A scaled-to-zero set is `Done`.
            FilterPreset::HideInactive => self.row.status.tone != StatusTone::Done,
            // A failing object is never hidden by a name rule.
            FilterPreset::HideSystem => {
                self.row.status.tone == StatusTone::Bad || !self.row.name.starts_with("system:")
            }
            FilterPreset::Nodes(_) => true,
        }
    }
}

impl FilteredTable for KindTableDelegate {
    fn view(&self) -> Option<&TableView> {
        self.views.get(&self.kind?)
    }

    fn view_mut(&mut self) -> Option<&mut TableView> {
        self.views.get_mut(&self.kind?)
    }

    fn column_plan(&self) -> Option<&ColumnPlan> {
        self.kind.map(|_| &self.layout.plan)
    }

    fn check_rows(&mut self, change: RowCheck, cx: &App) {
        let Some(kind) = self.kind else {
            return;
        };
        let rows = self.rows(cx);
        if let Some(view) = self.views.get_mut(&kind) {
            view.apply_check(&rows, change);
            self.all_checked = view.all_checked(&rows);
        }
    }

    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "KindTableDelegate")
    )]
    fn rebuild_view(&mut self, cx: &App) -> bool {
        let Some(kind) = self.kind else {
            return false;
        };
        let rows = self.rows(cx);
        let view = self
            .views
            .entry(kind)
            .or_insert_with(|| new_view(kind, &self.saved));
        view.rebuild(&rows, self.layout.plan.specs.len(), jiff::Timestamp::now());
        self.all_checked = view.all_checked(&rows);
        self.layout.relayout(&view.hidden)
    }
}

impl KindTableDelegate {
    /// The content of one body cell; the trait method centres it.
    fn cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        cx: &mut Context<TableState<Self>>,
    ) -> AnyElement {
        if self.layout.columns.is_select(col_ix) {
            let is_checked = self.is_row_checked(row_ix, cx);
            return select_cell(row_ix, is_checked, &self.shell);
        }
        let capacity = mono_capacity(self.layout.columns.columns.get(col_ix), cx);
        let (Some((_, row)), Some(logical), Some(kind)) = (
            self.row_at(row_ix, cx),
            self.layout.columns.logical(col_ix),
            self.kind,
        ) else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        let Some(cell_ix) = cell_index(kind.name_column(), logical) else {
            return name_cell(row, row_ix, capacity, cx);
        };
        match row.cells.get(cell_ix) {
            Some(cell) => {
                let slot = CellSlot {
                    row_ix,
                    col_ix,
                    capacity,
                };
                if kind == ResourceKind::Crds
                    && cell_ix == CRD_INSTANCES
                    && let Some(link) = self.instances_link(row, cell, row_ix, mono.clone(), cx)
                {
                    return link;
                }
                cell_element(cell, slot, self.align(logical), mono, cx)
            }
            None => div().into_any_element(),
        }
    }

    /// The Instances count of a CRD as a link to the instances, when the count is known and the
    /// kind is served.
    fn instances_link(
        &self,
        row: &KindRow,
        cell: &KindCell,
        row_ix: usize,
        mono: SharedString,
        cx: &App,
    ) -> Option<AnyElement> {
        let (KindCell::Quantity { text, .. }, KindObject::Crd(crd)) = (cell, &row.object) else {
            return None;
        };
        let live = self.session.as_ref()?.session.read(cx).live()?;
        let target = Screen::Kind(ResourceKind::Custom(browse_target(
            &crd.name,
            live.crd_kinds(),
        )?));
        let shell = self.shell.clone();
        let theme = cx.theme();
        Some(
            div()
                .id(("crd-instances", row_ix))
                .w_full()
                .truncate()
                .text_right()
                .font_family(mono)
                .cursor_pointer()
                .text_color(theme.link)
                .underline()
                .tooltip(|window, cx| Tooltip::new("Browse instances").build(window, cx))
                .on_click(move |_, _, cx| {
                    // The row's own click would open the CRD drawer as well.
                    cx.stop_propagation();
                    let _ = shell.update(cx, |shell, cx| shell.show_screen(target, cx));
                })
                .child(text.clone())
                .into_any_element(),
        )
    }
}

impl TableDelegate for KindTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.layout.columns.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.view().map_or(0, |view| view.rows().len())
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
        let sort = self.view().and_then(|view| view.sort);
        let all_checked = self.layout.columns.is_select(col_ix) && self.all_checked;
        header_cell(&self.layout, sort, all_checked, &self.shell, col_ix, cx)
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
        let Some(kind) = self.kind else {
            return menu;
        };
        // Cloned so the session is not borrowed while a submenu is built: that needs the app
        // mutably.
        let Some((open, row)) = self.row_at(row_ix, cx) else {
            return menu;
        };
        let row = row.clone();
        let row_context = open.row_context(cx);
        let open_url = (kind == ResourceKind::Ingresses)
            .then(|| open_url_menu_item(open_url_choice(&row), &row_context, window, cx));
        let secret = (kind == ResourceKind::Secrets)
            .then(|| {
                let access = self
                    .shell
                    .read_with(cx, |shell, _| shell.secret_value_access())
                    .unwrap_or(ValueAccess::Blocked);
                let object = row_context.object(ResourceKey::of_row(kind, &row));
                secret_menu(&row, &row_context, object, access, &self.shell, window, cx)
            })
            .flatten();
        // The submenu is built from the app, so its owned parts are made before the session is
        // borrowed again.
        let forward_menu = open.session.read(cx).guard(cx).and_then(|guard| {
            row_subject(&row).map(|subject| ForwardMenu::of(subject, &open.cluster, &guard))
        });
        let port_forward = forward_menu.map(|menu| menu.item(&self.shell, window, cx));
        let default_namespace = self
            .shell
            .read_with(cx, |shell, cx| shell.default_namespace(&open.cluster, cx))
            .ok()
            .flatten();
        let session = open.session.read(cx);
        let (Some(live), Some(guard)) = (session.live(), session.guard(cx)) else {
            return menu;
        };
        kind_menu(
            menu,
            kind,
            &row,
            &MenuCluster {
                guard: &guard,
                pods: live.pods.items(),
                context: &row_context,
                replica_sets: loaded_replica_sets(kind, &row, live),
            },
            &self.shell,
            MenuExtras {
                open_url,
                port_forward,
                secret,
                browse: browse_instances_item(&row, live.crd_kinds(), &self.shell),
                default_namespace,
                scope: Some(live.scope.clone()),
            },
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let (empty, plural) = self.kind.map_or((String::new(), ""), |kind| {
            (empty_text(kind, &self.scope_label(cx)), kind.plural())
        });
        match self.view() {
            Some(view) => filtered_empty_state(view, empty, plural, &self.shell, cx),
            None => div().into_any_element(),
        }
    }

    /// Loading also covers a kind switch, while a session still shows the previous kind; it
    /// lasts while the live session still waits for its list.
    fn loading(&self, cx: &App) -> bool {
        let Some(kind) = self.kind else {
            return false;
        };
        self.session
            .as_ref()
            .and_then(|session| session.session.read(cx).live())
            .is_some_and(|live| {
                live.kind_list(kind)
                    .is_none_or(|explorer| explorer.list.is_loading())
            })
    }
}

/// `No namespaces`, or `No deployments in team-a` for a namespaced kind. An empty NetworkPolicies
/// list adds what it means: nothing restricts the traffic.
fn empty_text(kind: ResourceKind, scope_label: &str) -> String {
    if kind == ResourceKind::NetworkPolicies {
        let place = if scope_label == "all namespaces" {
            "the whole cluster"
        } else {
            "this namespace"
        };
        return format!(
            "No {} in {scope_label}\nNo policy means all traffic is allowed in {place}.",
            kind.plural()
        );
    }
    if kind.is_namespaced() {
        format!("No {} in {scope_label}", kind.plural())
    } else {
        format!("No {}", kind.plural())
    }
}

/// `{namespace}/` is muted so the name stands out.
fn name_cell(row: &KindRow, row_ix: usize, capacity: usize, cx: &App) -> AnyElement {
    qualified_text(
        ("kind-name", row_ix),
        row.namespace.as_deref(),
        &row.name,
        capacity,
        cx,
    )
}

/// Where a body cell is and how much text its column holds, for its tooltip id and its cut.
#[derive(Clone, Copy)]
struct CellSlot {
    row_ix: usize,
    col_ix: usize,
    /// Mono characters that fit in the column.
    capacity: usize,
}

fn cell_element(
    cell: &KindCell,
    slot: CellSlot,
    align: Align,
    mono: SharedString,
    cx: &App,
) -> AnyElement {
    let CellSlot {
        row_ix,
        col_ix,
        capacity,
    } = slot;
    let base = || {
        let cell = div().w_full().truncate();
        match align {
            Align::Left => cell,
            Align::Right => cell.text_right(),
        }
    };
    // Text can be cut by a narrow column, so it carries its full value as a tooltip. A row holds
    // several of these, so the column joins the row in the id.
    let hover_text = |text: &SharedString, tooltip: &SharedString, mono: Option<SharedString>| {
        let id = ("kind-cell", (row_ix << 8) | col_ix);
        let cut = truncated_text_with_tooltip(id, text.clone(), tooltip.clone()).w_full();
        let cut = match align {
            Align::Left => cut,
            Align::Right => cut.text_right(),
        };
        match mono {
            Some(mono) => cut.font_family(mono),
            None => cut,
        }
        .into_any_element()
    };
    match cell {
        KindCell::Text(text) => return hover_text(text, text, None),
        KindCell::Hinted { text, tooltip } => return hover_text(text, tooltip, None),
        KindCell::Mono(text) => return hover_text(text, text, Some(mono)),
        // One qualified column per kind, so the row index alone makes the id unique.
        KindCell::Qualified { prefix, text } => {
            return qualified_text(
                ("kind-qualified", row_ix),
                prefix.as_deref(),
                text,
                capacity,
                cx,
            );
        }
        KindCell::Toned(label) => base().child(toned_text(label.clone(), cx)),
        KindCell::MonoWithMore { text, more } => base().child(
            h_flex()
                .w_full()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(mono)
                        .child(text.clone()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("+{more}")),
                ),
        ),
        KindCell::Quantity { text, tone, .. } => {
            let quantity = base().font_family(mono).child(text.clone());
            match tone {
                Some(tone) => quantity.text_color(tone_color(*tone, cx)),
                None => quantity,
            }
        }
        KindCell::Absent => base().text_color(cx.theme().muted_foreground).child("—"),
        KindCell::Duration {
            started_at: None, ..
        } => base().text_color(cx.theme().muted_foreground).child("—"),
        KindCell::Duration {
            started_at,
            finished_at,
        } => {
            let now = jiff::Timestamp::now();
            base()
                .font_family(mono)
                .child(format_age(*started_at, finished_at.unwrap_or(now)))
        }
        KindCell::NextRun(schedule) => {
            // Read per cell, like an age: the countdown never goes stale.
            match next_run_text(schedule, jiff::Timestamp::now()) {
                Some(text) => base().child(text),
                None => base().text_color(cx.theme().muted_foreground).child("—"),
            }
        }
        KindCell::Expiry { not_after } => {
            // Read per cell: days left change while the screen is open.
            let label = expiry_label(*not_after, jiff::Timestamp::now());
            base().child(toned_text(label, cx))
        }
        KindCell::Date { at, rule } => {
            let now = jiff::Timestamp::now();
            let text = base().child(date_text(*at, now));
            match date_tone(*rule, *at, now) {
                Some(tone) => text.text_color(tone_color(tone, cx)),
                None => text,
            }
        }
        KindCell::Age { at, tone } => {
            // Read per cell: a render has no shared clock, and a second of skew is invisible.
            let age = base()
                .font_family(mono)
                .child(format_age(*at, jiff::Timestamp::now()));
            match tone {
                Some(tone) => age.text_color(tone_color(*tone, cx)),
                None => age,
            }
        }
    }
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gpui_kit::px;

    use crate::resource_kind::NAME_COLUMN;
    use crate::status_tone::StatusLabel;
    use crate::table_layout::layout_columns;

    use super::*;
    use crate::kind_row::KindObject;

    #[test]
    fn empty_text_mentions_scope_only_for_namespaced_kinds() {
        assert_eq!(
            empty_text(ResourceKind::Namespaces, "all namespaces"),
            "No namespaces"
        );
        assert_eq!(
            empty_text(ResourceKind::Deployments, "team-a"),
            "No deployments in team-a"
        );
        assert_eq!(
            empty_text(ResourceKind::Deployments, "all namespaces"),
            "No deployments in all namespaces"
        );
    }

    #[test]
    fn empty_network_policies_say_that_no_policy_allows_all_traffic() {
        assert_eq!(
            empty_text(ResourceKind::NetworkPolicies, "postgres"),
            "No networkpolicies in postgres\nNo policy means all traffic is allowed in this namespace."
        );
        assert_eq!(
            empty_text(ResourceKind::NetworkPolicies, "all namespaces"),
            "No networkpolicies in all namespaces\nNo policy means all traffic is allowed in the whole cluster."
        );
    }

    fn delegate(kind: Option<ResourceKind>) -> KindTableDelegate {
        KindTableDelegate::new(kind, WeakEntity::new_invalid(), BTreeMap::new())
    }

    fn extra_columns(kind: ResourceKind) -> usize {
        match kind.name_column() {
            NameColumn::Flexible => 1,
            NameColumn::Hidden { .. } => 0,
        }
    }

    #[test]
    fn columns_start_with_name_unless_the_kind_hides_it() {
        assert!(kind_plan(None).specs.is_empty());
        for kind in ResourceKind::ALL {
            let columns = kind_columns(kind);
            assert_eq!(columns.len(), kind.columns().len() + extra_columns(kind));
            assert_eq!(
                columns.first().map(|column| column.name),
                Some(if kind == ResourceKind::Events {
                    "Type"
                } else {
                    "Name"
                })
            );
        }
    }

    #[test]
    fn cell_index_skips_name_only_when_shown() {
        assert_eq!(cell_index(NameColumn::Flexible, 0), None);
        assert_eq!(cell_index(NameColumn::Flexible, 1), Some(0));
        assert_eq!(cell_index(NameColumn::Flexible, 3), Some(2));
        let hidden = NameColumn::Hidden { flexible: 3 };
        assert_eq!(cell_index(hidden, 0), Some(0));
        assert_eq!(cell_index(hidden, 5), Some(5));
    }

    #[test]
    fn events_columns_flex_message_with_minimum_width() {
        let plan = kind_plan(Some(ResourceKind::Events));
        let layout = layout_columns(
            &plan.specs,
            plan.flexible,
            Pixels::ZERO,
            &Default::default(),
        );
        let message = layout.columns.get(4).expect("a Message column");
        assert_eq!(message.name.as_ref(), "Message");
        assert_eq!(message.width, px(160.));
        assert_eq!(message.min_width, px(160.));
        let reason = layout.columns.get(2).expect("a Reason column");
        assert_eq!(reason.width, px(260.));
    }

    #[test]
    fn fit_width_resizes_the_flexible_column() {
        let mut events = delegate(Some(ResourceKind::Events));
        assert!(events.fit_width(px(1400.)));
        assert!(!events.fit_width(px(1400.)));
        let message_width = events.layout.columns.columns.get(4).map(|c| c.width);
        assert!(message_width > Some(px(160.)));

        let mut deployments = delegate(Some(ResourceKind::Deployments));
        assert!(deployments.fit_width(px(1400.)));
        let name_width = deployments.layout.columns.columns.get(1).map(|c| c.width);
        assert!(name_width > Some(px(NAME_COLUMN.width)));
    }

    #[test]
    fn hpa_target_and_metrics_fit_their_text_at_1320_px() {
        let mut hpas = delegate(Some(ResourceKind::HorizontalPodAutoscalers));
        // A 1320 px window less the 220 px sidebar, drawer closed.
        hpas.fit_width(px(1100.));
        let width = |name: &str| {
            hpas.layout
                .columns
                .columns
                .iter()
                .find(|column| column.name.as_ref() == name)
                .map(|column| column.width)
        };
        // `deployment/opentelemetry-collector` and `s0-prometheus <unknown> / 27k` of the UAT rows.
        assert!(width("Target") >= Some(px(280.)), "{:?}", width("Target"));
        assert!(width("Metrics") >= Some(px(260.)), "{:?}", width("Metrics"));
    }

    #[test]
    fn fit_width_does_nothing_without_a_kind() {
        assert!(!delegate(None).fit_width(px(1400.)));
    }

    fn saved_last_column(kind: ResourceKind) -> BTreeMap<String, TablePrefs> {
        let plan = kind_plan(Some(kind));
        let hidden_name = plan.specs[plan.specs.len() - 1].name;
        BTreeMap::from([(
            screen_key(Screen::Kind(kind)).to_owned(),
            TablePrefs {
                sort: None,
                hidden: vec![hidden_name.to_owned()],
            },
        )])
    }

    #[test]
    fn new_view_applies_saved_prefs() {
        let kind = ResourceKind::Deployments;
        let columns = kind_plan(Some(kind)).specs.len();
        let mut delegate =
            KindTableDelegate::new(None, WeakEntity::new_invalid(), saved_last_column(kind));
        delegate.set_kind(Some(kind));
        let view = delegate.view().expect("a view for the kind");
        assert_eq!(view.hidden, BTreeSet::from([columns - 1]));
    }

    #[test]
    fn new_view_of_a_kind_without_saved_prefs_keeps_the_defaults() {
        let mut delegate = KindTableDelegate::new(
            None,
            WeakEntity::new_invalid(),
            saved_last_column(ResourceKind::Deployments),
        );
        delegate.set_kind(Some(ResourceKind::Services));
        let view = delegate.view().expect("a view for the kind");
        assert!(view.hidden.is_empty());
    }

    #[test]
    fn set_kind_reports_whether_the_kind_changed() {
        let mut delegate = delegate(None);
        assert!(delegate.set_kind(Some(ResourceKind::Deployments)));
        assert!(!delegate.set_kind(Some(ResourceKind::Deployments)));
    }

    #[test]
    fn set_kind_keeps_the_table_width_and_the_view_of_each_kind() {
        let mut delegate = delegate(Some(ResourceKind::Deployments));
        assert!(delegate.fit_width(px(1400.)));
        if let Some(view) = delegate.view_mut() {
            view.filter.text = "api".to_owned();
        }
        assert!(delegate.set_kind(Some(ResourceKind::Events)));
        let message = delegate.layout.columns.columns.get(4).map(|c| c.width);
        assert!(message > Some(px(160.)));
        assert!(delegate.view().is_some_and(|view| !view.is_filtering()));
        delegate.set_kind(Some(ResourceKind::Deployments));
        assert_eq!(
            delegate.view().map(|view| view.filter.text.as_str()),
            Some("api")
        );
        delegate.reset_filters();
        assert!(delegate.view().is_some_and(|view| !view.is_filtering()));
    }

    fn row(cells: Vec<KindCell>) -> KindRow {
        KindRow {
            namespace: Some("team-a".to_owned()),
            name: "api".to_owned(),
            created_at: None,
            status: StatusLabel {
                text: "Active".into(),
                tone: StatusTone::Ok,
            },
            cells,
            sections: Vec::new(),
            related_pods: None,
            event: None,
            labels: vec!["app=api".into()],
            object: KindObject::Plain,
        }
    }

    #[test]
    fn kind_row_values_follow_columns() {
        let at = jiff::Timestamp::from_second(100).expect("valid timestamp");
        let status = StatusLabel {
            text: "Failed".into(),
            tone: StatusTone::Bad,
        };
        let row = row(vec![
            KindCell::Text("3/3".into()),
            KindCell::Mono("RollingUpdate".into()),
            KindCell::Qualified {
                prefix: Some("ns".into()),
                text: "svc".into(),
            },
            KindCell::Toned(status),
            KindCell::Absent,
            KindCell::Age {
                at: Some(at),
                tone: None,
            },
            KindCell::Duration {
                started_at: Some(at),
                finished_at: None,
            },
        ]);
        let row = KindTableRow {
            row: &row,
            name_column: NameColumn::Flexible,
        };
        assert!(matches!(
            row.value(0),
            CellValue::Qualified {
                prefix: Some("team-a"),
                text: "api"
            }
        ));
        assert!(matches!(row.value(1), CellValue::Text(text) if text == "3/3"));
        assert!(matches!(row.value(2), CellValue::Text(text) if text == "RollingUpdate"));
        assert!(matches!(
            row.value(3),
            CellValue::Qualified {
                prefix: Some("ns"),
                text: "svc"
            }
        ));
        assert!(matches!(
            row.value(4),
            CellValue::Status {
                tone: StatusTone::Bad,
                ..
            }
        ));
        assert!(matches!(row.value(5), CellValue::Absent));
        assert!(matches!(row.value(6), CellValue::Age(Some(_))));
        assert!(matches!(row.value(7), CellValue::Span { .. }));
        assert!(matches!(row.value(8), CellValue::Absent));
    }

    #[test]
    fn hide_inactive_drops_done_rows() {
        let at = |tone| {
            let mut row = row(vec![KindCell::Text("0".into())]);
            row.status.tone = tone;
            row
        };
        let in_preset = |row: &KindRow, preset: &FilterPreset| {
            KindTableRow {
                row,
                name_column: NameColumn::Flexible,
            }
            .in_preset(preset)
        };
        for (tone, kept) in [
            (StatusTone::Ok, true),
            (StatusTone::Warn, true),
            (StatusTone::Bad, true),
            (StatusTone::Done, false),
        ] {
            assert_eq!(
                in_preset(&at(tone), &FilterPreset::HideInactive),
                kept,
                "{tone:?}"
            );
        }
        // A kind has no use for a Nodes group.
        let group = FilterPreset::Nodes(crate::node_summary::NodeGroup::Ready);
        assert!(in_preset(&at(StatusTone::Done), &group));
    }

    #[test]
    fn hide_system_hides_system_prefix_only() {
        let named = |name: &str| {
            let mut row = row(vec![KindCell::Text("1".into())]);
            row.name = name.to_owned();
            row
        };
        let is_kept = |name: &str| {
            let row = named(name);
            KindTableRow {
                row: &row,
                name_column: NameColumn::Flexible,
            }
            .in_preset(&FilterPreset::HideSystem)
        };
        assert!(!is_kept("system:controller:job-controller"));
        assert!(!is_kept("system:masters"));
        assert!(is_kept("cluster-admin"));
        // Only the prefix counts, and only with its colon.
        assert!(is_kept("my-system:role"));
        assert!(is_kept("systemic"));
    }

    #[test]
    fn hide_system_keeps_bad_rows() {
        let mut row = row(vec![KindCell::Text("1".into())]);
        row.name = "system:anonymous-admin".to_owned();
        let is_kept = |row: &KindRow| {
            KindTableRow {
                row,
                name_column: NameColumn::Flexible,
            }
            .in_preset(&FilterPreset::HideSystem)
        };
        assert!(!is_kept(&row));
        // A binding that hands cluster-admin to everyone must stay in sight.
        row.status.tone = StatusTone::Bad;
        assert!(is_kept(&row));
        row.status.tone = StatusTone::Warn;
        assert!(!is_kept(&row));
    }

    #[test]
    fn rows_without_a_name_column_start_at_their_first_cell() {
        let row = row(vec![KindCell::Text("Warning".into())]);
        let row = KindTableRow {
            row: &row,
            name_column: NameColumn::Hidden { flexible: 3 },
        };
        assert!(matches!(row.value(0), CellValue::Text(text) if text == "Warning"));
        assert!(matches!(row.value(1), CellValue::Absent));
    }

    #[test]
    fn mono_with_more_filters_and_sorts_by_its_text_only() {
        let row = row(vec![KindCell::MonoWithMore {
            text: "deployment/api".into(),
            more: 5,
        }]);
        let table_row = KindTableRow {
            row: &row,
            name_column: NameColumn::Flexible,
        };
        let value = table_row.value(1);
        assert!(matches!(value, CellValue::Text(text) if text == "deployment/api"));
    }

    #[test]
    fn quantity_sorts_by_its_value_not_its_text() {
        let quantity = |text: &str, value: u64| {
            row(vec![KindCell::Quantity {
                text: text.to_owned().into(),
                value,
                tone: None,
            }])
        };
        let number = |row: &KindRow| {
            let row = KindTableRow {
                row,
                name_column: NameColumn::Flexible,
            };
            match row.value(1) {
                CellValue::Number(number) => Some(number),
                _ => None,
            }
        };
        // "9Mi" sorts after "10Mi" as text, but before it by bytes.
        assert!(number(&quantity("9Mi", 9 << 20)) < number(&quantity("10Mi", 10 << 20)));
    }

    #[test]
    fn next_run_sorts_soonest_first() {
        use std::cmp::Ordering;

        use crate::table_sort::{SortDirection, compare_values};

        let next_run = |schedule: &str| {
            row(vec![KindCell::NextRun(
                cluster::CronSchedule::parse(schedule, None).expect("valid schedule"),
            )])
        };
        let soon = next_run("* * * * *");
        let far = next_run("0 0 1 1 *");
        let never = next_run("0 0 30 2 *");
        let value = |row: &KindRow| {
            let row = KindTableRow {
                row,
                name_column: NameColumn::Flexible,
            };
            // Column 1 is the first cell: Name comes first.
            let value = row.value(1);
            match value {
                CellValue::Number(seconds) => Some(seconds),
                _ => None,
            }
        };
        let now = jiff::Timestamp::now();
        let order = |a: &KindRow, b: &KindRow| {
            let (a, b) = (
                KindTableRow {
                    row: a,
                    name_column: NameColumn::Flexible,
                },
                KindTableRow {
                    row: b,
                    name_column: NameColumn::Flexible,
                },
            );
            compare_values(&a.value(1), &b.value(1), SortDirection::Ascending, now)
        };
        assert!(value(&soon) < value(&far));
        assert_eq!(order(&soon, &far), Ordering::Less);
        // A schedule with no next run sorts last.
        assert_eq!(value(&never), None);
        assert_eq!(order(&far, &never), Ordering::Less);
        assert_eq!(order(&never, &soon), Ordering::Greater);
    }

    #[test]
    fn expiry_cell_sorts_by_not_after() {
        let expiry = |seconds: i64| {
            row(vec![KindCell::Expiry {
                not_after: jiff::Timestamp::from_second(seconds).expect("valid timestamp"),
            }])
        };
        let number = |row: &KindRow| {
            let row = KindTableRow {
                row,
                name_column: NameColumn::Flexible,
            };
            match row.value(1) {
                CellValue::Number(number) => Some(number),
                _ => None,
            }
        };
        assert_eq!(number(&expiry(500)), Some(500));
        assert!(number(&expiry(500)) < number(&expiry(900)));
    }
}
