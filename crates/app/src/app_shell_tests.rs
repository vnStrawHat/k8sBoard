//! The `/` shortcut in a headless window. The kubeconfig is missing, so nothing touches the
//! network: the shell stays without a session, and the filter bar is not drawn. The quick
//! filter input is therefore out of the element tree, which is exactly what a focused input
//! that disappears looks like to the window.

use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};

use super::*;
use crate::launch_options::{LaunchRequest, parse_launch_options};

/// Which of the shell's two focus targets holds the focus: the root, and the quick filter.
type Focus = (bool, bool);

const ROOT: Focus = (true, false);

fn open_shell(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<AppShell>) {
    let args = ["--kubeconfig", "does-not-exist/kubeconfig.yml"].map(str::to_owned);
    let Ok(LaunchRequest::Run(options)) = parse_launch_options(args.into_iter()) else {
        panic!("the launch flags are valid");
    };
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1320.), px(900.)),
        };
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| AppShell::new(options, window, cx)),
        )
        .expect("open the test window");
        (window.downcast::<Root>().expect("a Root window"), shell)
    })
}

fn render(window: WindowHandle<Root>, cx: &mut TestAppContext) {
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("the window is open");
    cx.run_until_parked();
}

fn focus_in(shell: &Entity<AppShell>, window: &Window, cx: &App) -> Focus {
    let shell = shell.read(cx);
    (
        shell.focus_handle.is_focused(window),
        shell
            .quick_filter
            .read(cx)
            .focus_handle(cx)
            .is_focused(window),
    )
}

fn focus_of(
    window: WindowHandle<Root>,
    shell: &Entity<AppShell>,
    cx: &mut TestAppContext,
) -> Focus {
    cx.update_window(window.into(), |_, window, cx| focus_in(shell, window, cx))
        .expect("the window is open")
}

fn press_slash(window: WindowHandle<Root>, cx: &mut TestAppContext) {
    cx.update_window(window.into(), |_, window, cx| window.press("/", cx))
        .expect("the window is open");
    cx.run_until_parked();
}

/// Whether the `/` action has a handler where the keyboard is, which is what a key binding
/// needs to fire.
fn is_slash_available(window: WindowHandle<Root>, cx: &mut TestAppContext) -> bool {
    cx.update_window(window.into(), |_, window, cx| {
        window.is_action_available(&FocusQuickFilter, cx)
    })
    .expect("the window is open")
}

#[gpui_kit::test]
fn slash_is_available_before_any_click(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    assert_eq!(focus_of(window, &shell, cx), ROOT);
    assert!(is_slash_available(window, cx));
}

#[gpui_kit::test]
fn focus_returns_to_the_shell_when_the_focused_input_leaves_the_tree(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    // `/` moves the focus to the quick filter. There is no session, so the filter bar is not
    // drawn and that input is not in the tree: the window reports the focus as lost, exactly
    // as when the editor of a closed drawer disappears. Without the shell's restore handler
    // nothing would be focused and `/` would stop matching.
    for _ in 0..2 {
        press_slash(window, cx);
        render(window, cx);
        assert_eq!(focus_of(window, &shell, cx), ROOT);
        assert!(is_slash_available(window, cx));
    }
}
