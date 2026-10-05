//! The copy helpers in a headless window. The clipboard is the test platform's own: the real
//! system clipboard is never touched.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, IntoElement, Modifiers, Render, TestAppContext,
    Window, WindowOptions, div,
};

use gpui_kit::component::h_flex;

use super::*;
use crate::drawer::chips;

/// A window with one chip set, one copy button, and one copyable mono value.
struct CopyProbe {
    terms: Vec<SharedString>,
}

impl Render for CopyProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(chips("probe-chips", &self.terms, cx))
            .child(h_flex().child(copy_button("probe-button", "api-0")))
            .child(copyable_mono("probe-address", "10.1.2.3", cx))
    }
}

fn terms(texts: &[&str]) -> Vec<SharedString> {
    texts.iter().map(|text| SharedString::from(*text)).collect()
}

fn open_probe(
    texts: &[&str],
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, gpui_kit::VisualTestContext) {
    let terms = terms(texts);
    let window = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| CopyProbe { terms })
        })
        .expect("open the test window")
        .0
    });
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("the window is open");
    let visual = gpui_kit::VisualTestContext::from_window(window, cx);
    (window, visual)
}

fn click(selector: &'static str, visual: &mut gpui_kit::VisualTestContext) {
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    visual.simulate_click(bounds.center(), Modifiers::default());
}

fn clipboard_text(cx: &TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[test]
fn a_set_of_terms_copies_one_per_line() {
    let terms = terms(&["app=api", "tier=web"]);
    assert_eq!(joined_terms(&terms), "app=api\ntier=web");
    assert_eq!(joined_terms(&[]), "");
}

#[gpui_kit::test]
fn copy_text_puts_the_text_on_the_clipboard(cx: &mut TestAppContext) {
    cx.update(|cx| copy_text("nginx:1.25", cx));
    assert_eq!(clipboard_text(cx).as_deref(), Some("nginx:1.25"));
}

#[gpui_kit::test]
fn a_click_on_a_chip_copies_its_key_and_value(cx: &mut TestAppContext) {
    let (_window, mut visual) = open_probe(&["app=api", "tier=web"], cx);
    click("probe-chips-chip-1", &mut visual);
    assert_eq!(clipboard_text(cx).as_deref(), Some("tier=web"));
    click("probe-chips-chip-0", &mut visual);
    assert_eq!(clipboard_text(cx).as_deref(), Some("app=api"));
}

#[gpui_kit::test]
fn the_button_after_the_chips_copies_the_whole_set(cx: &mut TestAppContext) {
    let (_window, mut visual) = open_probe(&["app=api", "tier=web"], cx);
    click("probe-chips-copy-all", &mut visual);
    assert_eq!(clipboard_text(cx).as_deref(), Some("app=api\ntier=web"));
}

#[gpui_kit::test]
fn the_copy_button_copies_its_text(cx: &mut TestAppContext) {
    let (_window, mut visual) = open_probe(&[], cx);
    click("probe-button", &mut visual);
    assert_eq!(clipboard_text(cx).as_deref(), Some("api-0"));
}

#[gpui_kit::test]
fn a_copyable_value_copies_the_full_text(cx: &mut TestAppContext) {
    let (_window, mut visual) = open_probe(&[], cx);
    click("probe-address-copy", &mut visual);
    assert_eq!(clipboard_text(cx).as_deref(), Some("10.1.2.3"));
}
