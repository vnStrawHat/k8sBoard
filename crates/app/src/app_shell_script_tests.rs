//! The inputs of a screenshot script reach the shell the way a user's would.

use gpui_kit::TestAppContext;

use super::app_shell_switch_tests::open_switch_fixture;
use super::*;
use crate::screenshot_script::{Input, dispatch_input};

#[gpui_kit::test]
fn a_key_step_runs_the_binding_of_the_focused_control(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("script-key", cx);
    fixture.open_switcher(cx);
    assert_eq!(fixture.highlight(cx).as_deref(), Some("prod-a"));
    fixture.with_window(cx, |window, cx| {
        dispatch_input(&Input::Key("down".to_owned()), window, cx);
    });
    assert_eq!(fixture.highlight(cx).as_deref(), Some("stg-b"));
}

#[gpui_kit::test]
fn a_type_step_fills_the_focused_text_input(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("script-type", cx);
    fixture.open_switcher(cx);
    fixture.with_window(cx, |window, cx| {
        dispatch_input(&Input::Type("dev".to_owned()), window, cx);
    });
    let typed = fixture.shell.read_with(cx, |shell, cx| {
        shell.switcher.filter().read(cx).value().to_string()
    });
    assert_eq!(typed, "dev");
}

#[gpui_kit::test]
fn the_reported_texts_name_the_screen(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("script-texts", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Nodes, cx));
    let texts = fixture
        .shell
        .read_with(cx, |shell, cx| shell.reported_texts(cx));
    assert!(texts.iter().any(|text| text.contains("Nodes")), "{texts:?}");
}
