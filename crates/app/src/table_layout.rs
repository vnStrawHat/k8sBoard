use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::table::Column;
use gpui_kit::{App, Div, ParentElement as _, Pixels, Styled as _, TextAlign, div, px};

/// Width the table cannot use: the empty trailing column and the vertical scrollbar.
const TABLE_GUTTER: Pixels = px(28.);

/// The width that makes the flexible column fill what the fixed columns leave over in a
/// table `table_width` wide, never narrower than `min_width`. The columns are fixed
/// pixel widths, so this is recomputed whenever the window size changes.
pub(crate) fn flexible_width(
    table_width: Pixels,
    fixed_width: Pixels,
    min_width: Pixels,
) -> Pixels {
    (table_width - TABLE_GUTTER - fixed_width).max(min_width)
}

/// A header label in the plain foreground colour: the kit's header colour is too faint in
/// the dark theme. It honours the column alignment, which the kit does not apply.
pub(crate) fn header_cell(column: &Column, cx: &App) -> Div {
    let header = div()
        .size_full()
        .text_color(cx.theme().foreground)
        .child(column.name.clone());
    if column.align == TextAlign::Right {
        header.text_right()
    } else {
        header
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flexible_width_takes_the_spare_space() {
        let width = flexible_width(px(1100.), px(570.), px(160.));
        assert_eq!(width, px(1100. - 28. - 570.));
    }

    #[test]
    fn flexible_width_never_drops_below_minimum() {
        assert_eq!(flexible_width(px(600.), px(570.), px(160.)), px(160.));
    }
}
