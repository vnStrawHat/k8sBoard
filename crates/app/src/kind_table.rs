//! The table of the explorer kinds: one delegate serves every `ResourceKind`, so a kind
//! switch only replaces the columns.

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledText, Window, div, px,
};

use crate::age::format_age;
use crate::cluster_session::{ClusterSession, LiveCluster};
use crate::drawer::truncated_text;
use crate::kind_row::{KindCell, KindRow};
use crate::resource_actions::kind_menu;
use crate::resource_kind::{Align, ResourceKind};
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
}

/// Name first, then the kind's own columns.
fn columns(kind: Option<ResourceKind>, name_width: Pixels) -> Vec<Column> {
    let Some(kind) = kind else {
        return Vec::new();
    };
    let name = Column::new("name", "Name")
        .width(name_width)
        .min_width(NAME_MIN_WIDTH);
    let rest = kind.columns().iter().map(|spec| {
        let column = Column::new(spec.name, spec.name).width(px(spec.width));
        match spec.align {
            Align::Left => column,
            Align::Right => column.text_right(),
        }
    });
    std::iter::once(name).chain(rest).collect()
}

/// The width of every column except Name, which takes the rest of the table.
fn fixed_width(kind: ResourceKind) -> Pixels {
    px(kind.columns().iter().map(|column| column.width).sum())
}

impl KindTableDelegate {
    pub(crate) fn new(kind: Option<ResourceKind>) -> Self {
        Self {
            session: None,
            kind,
            columns: columns(kind, NAME_MIN_WIDTH),
        }
    }

    pub(crate) fn set_session(&mut self, session: Option<Entity<ClusterSession>>) {
        self.session = session;
    }

    /// Switches the columns and returns whether the kind changed. The Name column goes back to
    /// its minimum width, so the next `fit_width` fits it again.
    pub(crate) fn set_kind(&mut self, kind: Option<ResourceKind>) -> bool {
        if self.kind == kind {
            return false;
        }
        self.kind = kind;
        self.columns = columns(kind, NAME_MIN_WIDTH);
        true
    }

    /// Resizes the Name column for a table `table_width` wide. Returns whether it changed,
    /// so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels) -> bool {
        let Some(kind) = self.kind else {
            return false;
        };
        let name_width = flexible_width(table_width, fixed_width(kind), NAME_MIN_WIDTH);
        if self
            .columns
            .get(NAME)
            .is_some_and(|column| column.width == name_width)
        {
            return false;
        }
        self.columns = columns(Some(kind), name_width);
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
            .and_then(|kind| kind.columns().get(col_ix.checked_sub(1)?))
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
        let Some(cell_ix) = col_ix.checked_sub(1) else {
            return name_cell(row, row_ix, mono, cx);
        };
        match row.cells.get(cell_ix) {
            Some(cell) => cell_element(cell, self.align(col_ix), mono, cx),
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
            Some(row) => kind_menu(menu, kind, row, &live.access),
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

/// `{namespace}/` is muted so the name stands out; both share one text run so a long name is
/// cut with an ellipsis instead of wrapping.
fn name_cell(row: &KindRow, row_ix: usize, mono: SharedString, cx: &App) -> AnyElement {
    let Some(namespace) = &row.namespace else {
        return truncated_text(("kind-name", row_ix), row.name.clone())
            .w_full()
            .font_family(mono)
            .into_any_element();
    };
    let prefix = format!("{namespace}/");
    let muted = HighlightStyle {
        color: Some(cx.theme().muted_foreground),
        ..Default::default()
    };
    let highlights = vec![(0..prefix.len(), muted)];
    let text = format!("{prefix}{}", row.name);
    let tooltip_text = SharedString::from(text.clone());
    div()
        .id(("kind-name", row_ix))
        .w_full()
        .truncate()
        .font_family(mono)
        .child(StyledText::new(text).with_highlights(highlights))
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .into_any_element()
}

fn cell_element(cell: &KindCell, align: Align, mono: SharedString, cx: &App) -> AnyElement {
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

    #[test]
    fn columns_start_with_name_and_follow_the_kind() {
        assert!(columns(None, NAME_MIN_WIDTH).is_empty());
        for kind in ResourceKind::ALL {
            assert_eq!(
                columns(Some(kind), NAME_MIN_WIDTH).len(),
                kind.columns().len() + 1
            );
        }
    }

    #[test]
    fn fit_width_fills_the_spare_space_once_per_kind() {
        let mut delegate = KindTableDelegate::new(Some(ResourceKind::Deployments));
        assert!(delegate.fit_width(px(1400.)));
        assert!(!delegate.fit_width(px(1400.)));
        let name_width = delegate.columns.first().map(|column| column.width);
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
    fn fit_width_does_nothing_without_a_kind() {
        assert!(!KindTableDelegate::new(None).fit_width(px(1400.)));
    }

    #[test]
    fn set_kind_reports_whether_the_kind_changed() {
        let mut delegate = KindTableDelegate::new(None);
        assert!(delegate.set_kind(Some(ResourceKind::Deployments)));
        assert!(!delegate.set_kind(Some(ResourceKind::Deployments)));
    }

    #[test]
    fn set_kind_resets_name_width_to_minimum() {
        let mut delegate = KindTableDelegate::new(Some(ResourceKind::Deployments));
        assert!(delegate.fit_width(px(1400.)));
        assert!(delegate.set_kind(Some(ResourceKind::Namespaces)));
        assert_eq!(
            delegate.columns.first().map(|column| column.width),
            Some(NAME_MIN_WIDTH)
        );
    }
}
