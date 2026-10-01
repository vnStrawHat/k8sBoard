//! The table of the explorer kinds: one delegate serves every `ResourceKind`, so a kind
//! switch only replaces the columns.

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledText, WeakEntity, Window, div, px,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::cluster_session::{ClusterSession, LiveCluster};
use crate::drawer::truncated_text;
use crate::kind_row::{KindCell, KindRow};
use crate::resource_actions::kind_menu;
use crate::resource_kind::{Align, NameColumn, ResourceKind};
use crate::status_tone::{tone_color, toned_text};
use crate::table_layout::{flexible_width, header_cell};

const NAME: usize = 0;
const NAME_MIN_WIDTH: Pixels = px(200.);

/// Rows come straight from the session, so the table never owns a copy of the rows.
pub(crate) struct KindTableDelegate {
    session: Option<Entity<ClusterSession>>,
    /// `None` while Pods or Nodes is shown; the table is not rendered then.
    kind: Option<ResourceKind>,
    columns: Vec<Column>,
    /// The row menu's "Go to object" reveals a row through the shell.
    shell: WeakEntity<AppShell>,
}

/// Name first, then the kind's own columns; or only the kind's columns when it hides Name.
/// The flexible column gets `flexible_width`.
fn columns(kind: Option<ResourceKind>, flexible_width: Pixels) -> Vec<Column> {
    let Some(kind) = kind else {
        return Vec::new();
    };
    let name_column = kind.name_column();
    let rest = kind.columns().iter().enumerate().map(|(index, spec)| {
        let column = Column::new(spec.name, spec.name).width(px(spec.width));
        let column = match spec.align {
            Align::Left => column,
            Align::Right => column.text_right(),
        };
        match name_column {
            NameColumn::Hidden { flexible } if flexible == index => {
                column.width(flexible_width).min_width(px(spec.width))
            }
            NameColumn::Hidden { .. } | NameColumn::Flexible => column,
        }
    });
    match name_column {
        NameColumn::Flexible => {
            let name = Column::new("name", "Name")
                .width(flexible_width)
                .min_width(NAME_MIN_WIDTH);
            std::iter::once(name).chain(rest).collect()
        }
        NameColumn::Hidden { .. } => rest.collect(),
    }
}

/// The index in `columns` of the column that takes the rest of the table, and its minimum
/// width.
fn flexible_column(kind: ResourceKind) -> (usize, Pixels) {
    match kind.name_column() {
        NameColumn::Flexible => (NAME, NAME_MIN_WIDTH),
        NameColumn::Hidden { flexible } => {
            let width = kind
                .columns()
                .get(flexible)
                .map_or(0., |column| column.width);
            (flexible, px(width))
        }
    }
}

/// The columns of a kind before any `fit_width`: the flexible column at its minimum.
fn initial_columns(kind: Option<ResourceKind>) -> Vec<Column> {
    kind.map_or_else(Vec::new, |kind| {
        columns(Some(kind), flexible_column(kind).1)
    })
}

/// The width of every column except the flexible one.
fn fixed_width(kind: ResourceKind) -> Pixels {
    let flexible = match kind.name_column() {
        NameColumn::Flexible => None,
        NameColumn::Hidden { flexible } => Some(flexible),
    };
    px(kind
        .columns()
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != flexible)
        .map(|(_, column)| column.width)
        .sum())
}

/// The index into `KindRow::cells` that table column `col_ix` shows, or `None` for the Name
/// column.
fn cell_index(name_column: NameColumn, col_ix: usize) -> Option<usize> {
    match name_column {
        NameColumn::Flexible => col_ix.checked_sub(1),
        NameColumn::Hidden { .. } => Some(col_ix),
    }
}

impl KindTableDelegate {
    pub(crate) fn new(kind: Option<ResourceKind>, shell: WeakEntity<AppShell>) -> Self {
        Self {
            session: None,
            kind,
            columns: initial_columns(kind),
            shell,
        }
    }

    pub(crate) fn set_session(&mut self, session: Option<Entity<ClusterSession>>) {
        self.session = session;
    }

    /// Switches the columns and returns whether the kind changed. The flexible column goes back
    /// to its minimum width, so the next `fit_width` fits it again.
    pub(crate) fn set_kind(&mut self, kind: Option<ResourceKind>) -> bool {
        if self.kind == kind {
            return false;
        }
        self.kind = kind;
        self.columns = initial_columns(kind);
        true
    }

    /// Resizes the flexible column for a table `table_width` wide. Returns whether it changed,
    /// so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels) -> bool {
        let Some(kind) = self.kind else {
            return false;
        };
        let (index, min_width) = flexible_column(kind);
        let width = flexible_width(table_width, fixed_width(kind), min_width);
        if self
            .columns
            .get(index)
            .is_some_and(|column| column.width == width)
        {
            return false;
        }
        self.columns = columns(Some(kind), width);
        true
    }

    fn live<'a>(&self, cx: &'a App) -> Option<&'a LiveCluster> {
        self.session.as_ref()?.read(cx).live()
    }

    fn rows<'a>(&self, cx: &'a App) -> &'a [KindRow] {
        let explorer = self.kind.and_then(|kind| self.live(cx)?.kind_list(kind));
        explorer.map_or(&[], |explorer| explorer.list.items())
    }

    fn scope_label(&self, cx: &App) -> String {
        self.live(cx)
            .map_or_else(String::new, |live| live.scope_label())
    }

    fn align(&self, col_ix: usize) -> Align {
        self.kind
            .and_then(|kind| kind.columns().get(cell_index(kind.name_column(), col_ix)?))
            .map_or(Align::Left, |column| column.align)
    }
}

impl TableDelegate for KindTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, cx: &App) -> usize {
        self.rows(cx).len()
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
        let Some(row) = self.rows(cx).get(row_ix) else {
            return div().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        let Some(cell_ix) = self
            .kind
            .and_then(|kind| cell_index(kind.name_column(), col_ix))
        else {
            return name_cell(row, row_ix, mono, cx);
        };
        match row.cells.get(cell_ix) {
            Some(cell) => cell_element(cell, row_ix, self.align(col_ix), mono, cx),
            None => div().into_any_element(),
        }
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let (Some(kind), Some(live)) = (self.kind, self.live(cx)) else {
            return menu;
        };
        match self.rows(cx).get(row_ix) {
            Some(row) => kind_menu(menu, kind, row, &live.access, &self.shell),
            None => menu,
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let text = self
            .kind
            .map(|kind| empty_text(kind, &self.scope_label(cx)))
            .unwrap_or_default();
        h_flex()
            .size_full()
            .justify_center()
            .items_center()
            .text_color(cx.theme().muted_foreground)
            .child(text)
    }

    /// Loading also covers a kind switch, while the session still shows the previous kind.
    fn loading(&self, cx: &App) -> bool {
        let Some(kind) = self.kind else {
            return false;
        };
        self.live(cx).is_some_and(|live| {
            live.kind_list(kind)
                .is_none_or(|explorer| explorer.list.is_loading())
        })
    }
}

/// `No namespaces`, or `No deployments in team-a` for a namespaced kind.
fn empty_text(kind: ResourceKind, scope_label: &str) -> String {
    if kind.is_namespaced() {
        format!("No {} in {scope_label}", kind.plural())
    } else {
        format!("No {}", kind.plural())
    }
}

/// `{namespace}/` is muted so the name stands out.
fn name_cell(row: &KindRow, row_ix: usize, mono: SharedString, cx: &App) -> AnyElement {
    qualified_text(
        ("kind-name", row_ix),
        row.namespace.as_deref(),
        &row.name,
        mono,
        cx,
    )
}

/// Mono text with a muted `{prefix}/`. Both share one text run so a long value is cut with an
/// ellipsis instead of wrapping, and the tooltip shows the whole text.
fn qualified_text(
    id: (&'static str, usize),
    prefix: Option<&str>,
    text: &str,
    mono: SharedString,
    cx: &App,
) -> AnyElement {
    let Some(prefix) = prefix else {
        return truncated_text(id, text.to_owned())
            .w_full()
            .font_family(mono)
            .into_any_element();
    };
    let prefix = format!("{prefix}/");
    let muted = HighlightStyle {
        color: Some(cx.theme().muted_foreground),
        ..Default::default()
    };
    let highlights = vec![(0..prefix.len(), muted)];
    let text = format!("{prefix}{text}");
    let tooltip_text = SharedString::from(text.clone());
    div()
        .id(id)
        .w_full()
        .truncate()
        .font_family(mono)
        .child(StyledText::new(text).with_highlights(highlights))
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .into_any_element()
}

fn cell_element(
    cell: &KindCell,
    row_ix: usize,
    align: Align,
    mono: SharedString,
    cx: &App,
) -> AnyElement {
    let base = || {
        let cell = div().w_full().truncate();
        match align {
            Align::Left => cell,
            Align::Right => cell.text_right(),
        }
    };
    match cell {
        KindCell::Text(text) => base().child(text.clone()),
        KindCell::Mono(text) => base().font_family(mono).child(text.clone()),
        // One qualified column per kind, so the row index alone makes the id unique.
        KindCell::Qualified { prefix, text } => {
            return qualified_text(
                ("kind-qualified", row_ix),
                prefix.as_deref(),
                text,
                mono,
                cx,
            );
        }
        KindCell::Toned(label) => base().child(toned_text(label.clone(), cx)),
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
    use super::*;

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

    fn delegate(kind: Option<ResourceKind>) -> KindTableDelegate {
        KindTableDelegate::new(kind, WeakEntity::new_invalid())
    }

    fn extra_columns(kind: ResourceKind) -> usize {
        match kind.name_column() {
            NameColumn::Flexible => 1,
            NameColumn::Hidden { .. } => 0,
        }
    }

    #[test]
    fn columns_start_with_name_unless_the_kind_hides_it() {
        assert!(columns(None, NAME_MIN_WIDTH).is_empty());
        for kind in ResourceKind::ALL {
            let columns = columns(Some(kind), NAME_MIN_WIDTH);
            assert_eq!(columns.len(), kind.columns().len() + extra_columns(kind));
            assert_eq!(
                columns.first().map(|column| column.name.as_ref()),
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
        let columns = columns(Some(ResourceKind::Events), px(640.));
        let message = columns.get(3).expect("a Message column");
        assert_eq!(message.name.as_ref(), "Message");
        assert_eq!(message.width, px(640.));
        assert_eq!(message.min_width, px(280.));
        let reason = columns.get(1).expect("a Reason column");
        assert_eq!(reason.width, px(170.));
    }

    #[test]
    fn fit_width_resizes_the_flexible_column() {
        let mut events = delegate(Some(ResourceKind::Events));
        assert!(events.fit_width(px(1400.)));
        assert!(!events.fit_width(px(1400.)));
        let message_width = events.columns.get(3).map(|column| column.width);
        assert_eq!(
            message_width,
            Some(flexible_width(
                px(1400.),
                fixed_width(ResourceKind::Events),
                px(280.)
            ))
        );
        assert!(message_width > Some(px(280.)));

        let mut deployments = delegate(Some(ResourceKind::Deployments));
        assert!(deployments.fit_width(px(1400.)));
        let name_width = deployments.columns.first().map(|column| column.width);
        assert_eq!(
            name_width,
            Some(flexible_width(
                px(1400.),
                fixed_width(ResourceKind::Deployments),
                NAME_MIN_WIDTH
            ))
        );
    }

    #[test]
    fn fixed_width_leaves_out_the_flexible_column() {
        let total =
            |kind: ResourceKind| -> f32 { kind.columns().iter().map(|column| column.width).sum() };
        let NameColumn::Hidden { flexible } = ResourceKind::Events.name_column() else {
            panic!("Events hide the Name column");
        };
        let flexible_width = ResourceKind::Events
            .columns()
            .get(flexible)
            .map_or(0., |column| column.width);
        assert_eq!(
            fixed_width(ResourceKind::Events),
            px(total(ResourceKind::Events) - flexible_width)
        );
        // Name is the flexible column of the other kinds and is not part of `columns`.
        assert_eq!(
            fixed_width(ResourceKind::ConfigMaps),
            px(total(ResourceKind::ConfigMaps))
        );
    }

    #[test]
    fn fit_width_does_nothing_without_a_kind() {
        assert!(!delegate(None).fit_width(px(1400.)));
    }

    #[test]
    fn set_kind_reports_whether_the_kind_changed() {
        let mut delegate = delegate(None);
        assert!(delegate.set_kind(Some(ResourceKind::Deployments)));
        assert!(!delegate.set_kind(Some(ResourceKind::Deployments)));
    }

    #[test]
    fn set_kind_resets_the_flexible_column_to_its_minimum() {
        let mut delegate = delegate(Some(ResourceKind::Deployments));
        assert!(delegate.fit_width(px(1400.)));
        assert!(delegate.set_kind(Some(ResourceKind::Namespaces)));
        assert_eq!(
            delegate.columns.first().map(|column| column.width),
            Some(NAME_MIN_WIDTH)
        );
        assert!(delegate.set_kind(Some(ResourceKind::Events)));
        assert_eq!(
            delegate.columns.get(3).map(|column| column.width),
            Some(px(280.))
        );
    }
}
