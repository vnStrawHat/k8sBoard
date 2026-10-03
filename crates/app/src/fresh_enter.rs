//! The content of a kit alert dialog whose confirm must never fire on a held Enter.
//!
//! The kit binds `enter` to Confirm inside its dialog, and that binding repeats while the key is
//! held. A user who holds Enter on the cluster switcher would then confirm a dialog opened under
//! their finger without reading it. Like the write confirm dialog, this content switches the kit
//! binding off (`keymap.rs` binds `enter` to `NoAction` in `FRESH_ENTER`) and confirms on a fresh
//! press only. A focused button keeps its own Enter, so the content takes the focus when it opens.

use std::rc::Rc;

use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, App, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, Styled as _, Window,
};

/// The key context the `enter` suppression is bound in.
pub(crate) const FRESH_ENTER: &str = "FreshEnter";

type Content = dyn Fn(&App) -> AnyElement;
type OnEnter = dyn Fn(&mut Window, &mut App);

pub(crate) struct FreshEnter {
    content: Rc<Content>,
    on_enter: Rc<OnEnter>,
    focus_handle: FocusHandle,
    needs_focus: bool,
}

impl FreshEnter {
    /// `content` draws the body; `on_enter` is what a fresh Enter does (the dialog's confirm).
    pub(crate) fn new(
        content: impl Fn(&App) -> AnyElement + 'static,
        on_enter: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            content: Rc::new(content),
            on_enter: Rc::new(on_enter),
            focus_handle: cx.focus_handle(),
            needs_focus: true,
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !is_enter(event) {
            return;
        }
        // The kit also clicks a focused element on the Enter key-up unless the press was handled.
        window.prevent_default();
        cx.stop_propagation();
        if confirms(event) {
            (self.on_enter)(window, cx);
        }
    }
}

/// An unmodified Enter, held or not.
pub(crate) fn is_enter(event: &KeyDownEvent) -> bool {
    event.keystroke.key == "enter" && !event.keystroke.modifiers.modified()
}

/// Whether the press confirms: a fresh Enter. A repeat of a held one never does.
pub(crate) fn confirms(event: &KeyDownEvent) -> bool {
    is_enter(event) && !event.is_held
}

impl Render for FreshEnter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.needs_focus = false;
            window.focus(&self.focus_handle, cx);
        }
        v_flex()
            .key_context(FRESH_ENTER)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .child((self.content)(cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::Keystroke;

    use super::*;

    fn event(key: &str, is_held: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: Keystroke::parse(key).expect("a valid keystroke"),
            is_held,
            prefer_character_input: false,
        }
    }

    #[test]
    fn a_fresh_enter_confirms_and_a_held_one_does_not() {
        assert!(confirms(&event("enter", false)));
        assert!(!confirms(&event("enter", true)));
    }

    #[test]
    fn other_keys_and_chords_never_confirm() {
        assert!(!confirms(&event("space", false)));
        assert!(!confirms(&event("shift-enter", false)));
        assert!(!confirms(&event("ctrl-enter", false)));
    }
}
