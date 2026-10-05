//! The two-column table the status bar tooltips show (spec 0054): section titles and values in
//! the tooltip's own text colour, row names muted.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, FontWeight, Hsla, IntoElement, ParentElement as _, Styled as _, Window, div,
};

/// A titled group of `(name, value)` rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Section {
    pub(crate) title: &'static str,
    pub(crate) rows: Vec<(&'static str, String)>,
}

/// Two columns of fixed-height cells rather than a grid: the tooltip sizes to its content, and a
/// grid there collapsed to zero-width columns.
pub(crate) fn details_table(sections: Vec<Section>, muted: Hsla) -> impl IntoElement {
    let cell = || div().h_5().flex().items_center().whitespace_nowrap();
    let (mut names, mut values) = (Vec::new(), Vec::new());
    for section in sections {
        names.push(
            cell()
                .font_weight(FontWeight::SEMIBOLD)
                .child(section.title),
        );
        values.push(cell());
        for (name, value) in section.rows {
            names.push(cell().text_color(muted).child(name));
            values.push(cell().justify_end().child(value));
        }
    }
    div()
        .flex()
        .flex_none()
        .gap_4()
        .py_0p5()
        .child(div().flex().flex_col().children(names))
        .child(div().flex().flex_col().children(values))
}

/// A tooltip builder for a table that does not change while it is open.
pub(crate) fn table_tooltip(sections: Vec<Section>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |window, cx| {
        let muted = cx.theme().muted_foreground;
        let sections = sections.clone();
        Tooltip::element(move |_, _| details_table(sections.clone(), muted)).build(window, cx)
    }
}
