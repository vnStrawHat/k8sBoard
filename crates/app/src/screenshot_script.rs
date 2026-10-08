//! The step file of `--script`: the parser, and the player that runs the steps inside the
//! screenshot capture (docs/specs/0003-app-shell-pods-nodes/screenshot-script.md).

use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    App, InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, TouchPhase, Window, point, px,
};
#[cfg(feature = "screenshot")]
use {
    crate::app_shell::AppShell,
    crate::screenshot::{ScreenshotRequest, save_png, wait_until_settled},
    gpui_kit::component::WindowExt as _,
    gpui_kit::{AnyWindowHandle, Entity},
    std::path::Path,
};

/// Time for the work a step starts (a watch answer, an animation) before the next step.
#[cfg(feature = "screenshot")]
const STEP_DELAY: Duration = Duration::from_millis(200);
/// The kit shows a tooltip 500 ms after the pointer rests on its item.
#[cfg(feature = "screenshot")]
const HOVER_DELAY: Duration = Duration::from_millis(900);

/// One line of the file that held a step.
#[derive(Debug, PartialEq)]
pub(crate) struct ScriptStep {
    /// The text of the line, for the log.
    pub(crate) text: String,
    pub(crate) step: Step,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    /// Waits until the launch screen settles, as the final capture does.
    WaitSettled,
    Wait(Duration),
    Input(Input),
    /// Renders the window to `<name>.png` next to the `--screenshot` file.
    Shot(String),
    /// Fails the run unless one of the texts the shell reports contains this one.
    Expect(String),
}

/// What reaches the window as if the user did it.
#[derive(Debug, PartialEq)]
pub(crate) enum Input {
    /// A keystroke in `Keystroke::parse` syntax; it is checked when the file is parsed.
    Key(String),
    Type(String),
    Click {
        button: MouseButton,
        position: Point<Pixels>,
    },
    Hover(Point<Pixels>),
    /// A wheel turn at `position`; a positive `delta_y` scrolls the content down.
    Scroll {
        position: Point<Pixels>,
        delta_y: Pixels,
    },
}

#[cfg(feature = "screenshot")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScriptEnd {
    Finished,
    ExpectFailed,
}

/// Parses the file text: one step per line, `#` comments and blank lines ignored. The error names
/// the line.
pub(crate) fn parse_script(text: &str) -> Result<Vec<ScriptStep>, String> {
    let mut steps = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let step =
            parse_step(line).map_err(|reason| format!("script line {}: {reason}", index + 1))?;
        steps.push(ScriptStep {
            text: line.trim().to_owned(),
            step,
        });
    }
    Ok(steps)
}

fn parse_step(line: &str) -> Result<Step, String> {
    let line = line.trim_start();
    let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
    // `type` keeps the spaces of its text; the other arguments are single words.
    let argument = if verb == "type" { rest } else { rest.trim() };
    if argument.is_empty() {
        return Err(format!("`{verb}` needs an argument"));
    }
    match verb {
        "wait" if argument == "settled" => Ok(Step::WaitSettled),
        "wait" => argument
            .parse::<u64>()
            .map(|millis| Step::Wait(Duration::from_millis(millis)))
            .map_err(|_| "`wait` takes `settled` or a number of milliseconds".to_owned()),
        "key" => {
            gpui_kit::Keystroke::parse(argument)
                .map_err(|error| format!("`key {argument}` is not a keystroke: {error}"))?;
            Ok(Step::Input(Input::Key(argument.to_owned())))
        }
        "type" => Ok(Step::Input(Input::Type(argument.to_owned()))),
        "click" | "rclick" => {
            let button = if verb == "click" {
                MouseButton::Left
            } else {
                MouseButton::Right
            };
            let position = parse_position(argument)?;
            Ok(Step::Input(Input::Click { button, position }))
        }
        "hover" => Ok(Step::Input(Input::Hover(parse_position(argument)?))),
        "scroll" => {
            let invalid = || format!("`scroll {argument}` is not `scroll <x>,<y> <dy>`");
            let (position, delta) = argument
                .split_once(char::is_whitespace)
                .ok_or_else(invalid)?;
            let position = parse_position(position)?;
            let delta_y = delta.trim().parse::<f32>().map_err(|_| invalid())?;
            Ok(Step::Input(Input::Scroll {
                position,
                delta_y: px(delta_y),
            }))
        }
        "shot" => {
            let is_plain = argument
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !is_plain {
                return Err(format!(
                    "`shot {argument}`: a name holds letters, digits, `-` and `_` only"
                ));
            }
            Ok(Step::Shot(argument.to_owned()))
        }
        "expect" => Ok(Step::Expect(argument.to_owned())),
        _ => Err(format!("unknown step `{verb}`")),
    }
}

/// `x,y` in window pixels.
fn parse_position(text: &str) -> Result<Point<Pixels>, String> {
    let invalid = || format!("`{text}` is not x,y in window pixels");
    let (x, y) = text.split_once(',').ok_or_else(invalid)?;
    let x = x.trim().parse::<f32>().map_err(|_| invalid())?;
    let y = y.trim().parse::<f32>().map_err(|_| invalid())?;
    Ok(point(px(x), px(y)))
}

/// Runs the steps one after the other. A failed `expect` saves `failed-step-N.png` and ends the
/// run with `ExpectFailed`; any other failure is an error.
#[cfg(feature = "screenshot")]
pub(crate) async fn play(
    steps: &[ScriptStep],
    window: &AnyWindowHandle,
    shell: &Entity<AppShell>,
    request: &ScreenshotRequest,
    cx: &mut gpui_kit::AsyncApp,
) -> anyhow::Result<ScriptEnd> {
    let dir = request
        .path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    for (index, script_step) in steps.iter().enumerate() {
        let number = index + 1;
        eprintln!("step {number}: {}", script_step.text);
        match &script_step.step {
            Step::WaitSettled => {
                if !wait_until_settled(shell, request.screen, cx).await? {
                    anyhow::bail!("step {number}: the screen did not settle");
                }
            }
            Step::Wait(duration) => cx.background_executor().timer(*duration).await,
            Step::Shot(name) => {
                save_png(window, &dir.join(format!("{name}.png")), cx).await?;
            }
            Step::Expect(text) => {
                let texts = window.update(cx, |_, window, cx| reported_texts(shell, window, cx))?;
                if texts
                    .iter()
                    .any(|reported| reported.contains(text.as_str()))
                {
                    continue;
                }
                let failed = dir.join(format!("failed-step-{number}.png"));
                save_png(window, &failed, cx).await?;
                eprintln!(
                    "step {number}: expected `{text}`, the shell reports {texts:?}; saved {}",
                    failed.display()
                );
                return Ok(ScriptEnd::ExpectFailed);
            }
            Step::Input(input) => {
                window.update(cx, |_, window, cx| dispatch_input(input, window, cx))?;
                let delay = match input {
                    Input::Hover(_) => HOVER_DELAY,
                    _ => STEP_DELAY,
                };
                cx.background_executor().timer(delay).await;
                window.update(cx, |_, window, cx| window.render_frame(cx))?;
            }
        }
    }
    Ok(ScriptEnd::Finished)
}

/// What the shell can say about the window without reading pixels: its screen, cursor row,
/// drawer subject, notices, and whether a dialog is open. Text drawn inside a dialog or a menu is
/// not reported; look at the PNG for it.
#[cfg(feature = "screenshot")]
fn reported_texts(shell: &Entity<AppShell>, window: &mut Window, cx: &mut App) -> Vec<String> {
    let mut texts = shell.read(cx).reported_texts(cx);
    if window.has_active_dialog(cx) {
        texts.push("dialog open".to_owned());
    }
    texts
}

/// Sends one input to the window the way the platform would. Keys and text go to the focus.
#[cfg(any(feature = "screenshot", test))]
pub(crate) fn dispatch_input(input: &Input, window: &mut Window, cx: &mut App) {
    match input {
        Input::Key(key) => window.press(key, cx),
        Input::Type(text) => window.input(text, cx),
        Input::Hover(position) => {
            window.render_frame(cx);
            move_pointer(window, *position, cx);
        }
        Input::Scroll { position, delta_y } => {
            window.render_frame(cx);
            move_pointer(window, *position, cx);
            // gpui wheel deltas follow the content: a negative y moves it up, i.e. scrolls down.
            window.dispatch_event(
                ScrollWheelEvent {
                    position: *position,
                    delta: ScrollDelta::Pixels(point(px(0.), -*delta_y)),
                    modifiers: Default::default(),
                    touch_phase: TouchPhase::Moved,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        }
        Input::Click { button, position } => {
            window.render_frame(cx);
            move_pointer(window, *position, cx);
            window.dispatch_event(
                MouseDownEvent {
                    button: *button,
                    position: *position,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseUpEvent {
                    button: *button,
                    position: *position,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        }
    }

    fn move_pointer(window: &mut Window, position: Point<Pixels>, cx: &mut App) {
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(text: &str) -> Vec<Step> {
        parse_script(text)
            .expect("a valid script")
            .into_iter()
            .map(|script_step| script_step.step)
            .collect()
    }

    fn error(text: &str) -> String {
        parse_script(text).expect_err("an invalid script")
    }

    #[test]
    fn every_step_kind_parses() {
        let script = "\
# open the menu
wait settled

wait 250
key shift-j
type hello world
click 10, 20.5
rclick 3,4
hover 5,6
scroll 100,200 -300
shot menu-1
expect Pods
";
        assert_eq!(
            steps(script),
            vec![
                Step::WaitSettled,
                Step::Wait(Duration::from_millis(250)),
                Step::Input(Input::Key("shift-j".to_owned())),
                Step::Input(Input::Type("hello world".to_owned())),
                Step::Input(Input::Click {
                    button: MouseButton::Left,
                    position: point(px(10.), px(20.5)),
                }),
                Step::Input(Input::Click {
                    button: MouseButton::Right,
                    position: point(px(3.), px(4.)),
                }),
                Step::Input(Input::Hover(point(px(5.), px(6.)))),
                Step::Input(Input::Scroll {
                    position: point(px(100.), px(200.)),
                    delta_y: px(-300.),
                }),
                Step::Shot("menu-1".to_owned()),
                Step::Expect("Pods".to_owned()),
            ]
        );
    }

    #[test]
    fn a_step_keeps_its_line_text_for_the_log() {
        let parsed = parse_script("  key enter  \n").expect("a valid script");
        assert_eq!(parsed[0].text, "key enter");
    }

    #[test]
    fn type_keeps_inner_and_trailing_spaces() {
        assert_eq!(
            steps("type  a b "),
            vec![Step::Input(Input::Type(" a b ".to_owned()))]
        );
    }

    #[test]
    fn a_bad_line_names_its_number() {
        assert_eq!(
            error("key j\n\n# note\nfly away"),
            "script line 4: unknown step `fly`"
        );
        assert!(error("key").starts_with("script line 1: `key` needs an argument"));
        assert!(error("key a-b").starts_with("script line 1: `key a-b` is not a keystroke"));
        assert!(error("wait soon").contains("`wait` takes"));
        assert!(error("click 10").contains("x,y"));
        assert!(error("hover a,b").contains("x,y"));
        assert!(error("scroll 100,200").contains("scroll"));
        assert!(error("scroll 100,200 abc").contains("scroll"));
        assert!(error("shot ../x").contains("letters"));
        assert!(error("expect").contains("needs an argument"));
    }
}
