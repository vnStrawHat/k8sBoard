//! The Data section of an open Secret drawer: masked keys with Reveal and Copy. A value is fetched
//! by one GET per action, shown for 30 s, then dropped (and wiped). The view lives exactly while
//! the Secret's Overview tab is shown, so closing the drawer or changing the subject drops every
//! revealed value at once.
//!
//! Nothing here logs, traces, persists, or `Debug`-prints a value. The text of a revealed value is
//! copied into the paint only for as long as it is shown; GPUI's own text caches are outside our
//! control (secret-safety.md).

use std::sync::Arc;
use std::time::{Duration, Instant};

use cluster::{ClusterConnection, ClusterError, SecretKey, SecretValue};
use futures::future::BoxFuture;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
};

use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::config_map_rows::format_bytes;
use crate::drawer::DrawerTab;
use crate::launch_options::LaunchOptions;
use crate::resource_kind::ResourceKind;
use crate::secret_clipboard::{ClipboardMark, write_private_text};
use crate::table_selection::ResourceKey;

/// How long a revealed value stays visible.
pub(crate) const REVEAL_DURATION: Duration = Duration::from_secs(30);
const COPIED_FEEDBACK: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_secs(1);
/// Revealed text is cut here (back to a char boundary): a huge value would stall the layout.
pub(crate) const REVEAL_DISPLAY_LIMIT: usize = 4096;
/// Why every Reveal and Copy is disabled in a screenshot run.
const BLOCKED_REASON: &str = "Disabled in screenshot runs";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SecretAction {
    RevealAll,
    Reveal(String),
    Copy(String),
}

/// Whether Reveal and Copy work at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueAccess {
    Enabled,
    /// Under `--screenshot`: no value can be revealed or copied, so no capture can show one.
    Blocked,
}

/// The only place that decides, and the only reader of the screenshot option for secrets.
/// `AppShell` stores the answer once at startup; the view and the menus read that field.
pub(crate) fn value_access(options: &LaunchOptions) -> ValueAccess {
    if options.screenshot.is_some() {
        ValueAccess::Blocked
    } else {
        ValueAccess::Enabled
    }
}

/// The Secret whose Data section gets a values view: the selected Secret, while its Overview tab
/// is shown. Anything else (another kind, another tab, no selection) has none, so the view is
/// dropped and every revealed value wiped.
pub(crate) fn values_subject(
    selected: Option<&ResourceKey>,
    tab: DrawerTab,
) -> Option<ResourceKey> {
    match selected {
        Some(
            key @ ResourceKey::Kind {
                kind: ResourceKind::Secrets,
                ..
            },
        ) if tab == DrawerTab::Overview => Some(key.clone()),
        _ => None,
    }
}

/// What the sync does with a menu action that waits for its view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingAction {
    /// The view of its Secret exists: run it.
    Run,
    /// Another Secret is shown or none: it is stale.
    Drop,
}

pub(crate) fn pending_action(
    pending: &ResourceKey,
    subject: Option<&ResourceKey>,
) -> PendingAction {
    if subject == Some(pending) {
        PendingAction::Run
    } else {
        PendingAction::Drop
    }
}

/// Emitted after a successful Copy so the shell can arm the 30 s clipboard clear: the shell
/// outlives the drawer, the view does not.
pub(crate) struct SecretCopied(pub(crate) ClipboardMark);

/// One GET of the Secret's values, on tokio. A seam so tests need no cluster.
pub(crate) type FetchValues =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Vec<SecretValue>, ClusterError>> + Send + Sync>;

/// The fetcher of one Secret over `connection`.
pub(crate) fn fetcher(
    connection: ClusterConnection,
    namespace: String,
    name: String,
) -> FetchValues {
    Arc::new(move || {
        let connection = connection.clone();
        let (namespace, name) = (namespace.clone(), name.clone());
        Box::pin(async move { connection.secret_values(&namespace, &name).await })
    })
}

/// The Data section of the open Secret drawer. Dropping it drops (and wipes) every revealed value.
pub(crate) struct SecretValuesView {
    fetch: FetchValues,
    secret: ResourceKey,
    /// From the summary; never values.
    keys: Vec<SecretKey>,
    /// Sorted by key; at most one per key.
    revealed: Vec<RevealedValue>,
    request: ValuesRequest,
    /// The key name and when it was copied; never the value.
    copied: Option<(String, Instant)>,
    access: ValueAccess,
    is_ticking: bool,
    _ticker: Option<Task<()>>,
}

struct RevealedValue {
    value: SecretValue,
    hides_at: Instant,
}

enum ValuesRequest {
    Idle,
    /// Dropping the task aborts the request.
    Running {
        _task: Task<()>,
    },
    Failed {
        message: SharedString,
    },
}

impl EventEmitter<SecretCopied> for SecretValuesView {}

// ---- Pure core ----

/// A key an action names that the Secret no longer has.
#[derive(Debug, PartialEq, Eq)]
struct MissingKey(String);

impl MissingKey {
    fn text(&self) -> String {
        format!("Key {} no longer exists in this secret.", self.0)
    }
}

/// The values the action keeps; the rest are dropped (wiped) here.
fn kept_values(
    values: Vec<SecretValue>,
    action: &SecretAction,
) -> Result<Vec<SecretValue>, MissingKey> {
    let key = match action {
        SecretAction::RevealAll => return Ok(values),
        SecretAction::Reveal(key) | SecretAction::Copy(key) => key,
    };
    let kept: Vec<SecretValue> = values
        .into_iter()
        .filter(|value| value.key() == key)
        .collect();
    if kept.is_empty() {
        return Err(MissingKey(key.clone()));
    }
    Ok(kept)
}

/// Inserts or replaces by key, each hiding at `now + REVEAL_DURATION`.
fn reveal(revealed: &mut Vec<RevealedValue>, values: Vec<SecretValue>, now: Instant) {
    for value in values {
        let entry = RevealedValue {
            value,
            hides_at: now + REVEAL_DURATION,
        };
        match revealed
            .iter()
            .position(|old| old.value.key() == entry.value.key())
        {
            Some(index) => revealed[index] = entry,
            None => revealed.push(entry),
        }
    }
    revealed.sort_by(|left, right| left.value.key().cmp(right.value.key()));
}

/// Drops the values whose time is up; true when anything changed.
fn expire(revealed: &mut Vec<RevealedValue>, now: Instant) -> bool {
    let before = revealed.len();
    revealed.retain(|entry| entry.hides_at > now);
    revealed.len() != before
}

enum ValueDisplay<'a> {
    Text { shown: &'a str, is_cut: bool },
    Binary { size_bytes: usize },
}

fn value_display(value: &SecretValue) -> ValueDisplay<'_> {
    let Some(text) = value.as_text() else {
        return ValueDisplay::Binary {
            size_bytes: value.size_bytes(),
        };
    };
    if text.len() <= REVEAL_DISPLAY_LIMIT {
        return ValueDisplay::Text {
            shown: text,
            is_cut: false,
        };
    }
    let end = text.floor_char_boundary(REVEAL_DISPLAY_LIMIT);
    ValueDisplay::Text {
        shown: &text[..end],
        is_cut: true,
    }
}

/// Whole seconds left, rounded up, for `Hides in 23s`.
fn seconds_left(hides_at: Instant, now: Instant) -> u64 {
    let left = hides_at.saturating_duration_since(now);
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

/// 403 reads as the missing right; anything else as the cluster crate words it.
fn failure_text(error: &ClusterError) -> String {
    match error {
        ClusterError::Forbidden { .. } => "Not permitted: get secrets".to_owned(),
        other => error_text(other),
    }
}

// ---- The view ----

impl SecretValuesView {
    pub(crate) fn new(
        fetch: FetchValues,
        secret: ResourceKey,
        keys: Vec<SecretKey>,
        access: ValueAccess,
    ) -> Self {
        Self {
            fetch,
            secret,
            keys,
            revealed: Vec::new(),
            request: ValuesRequest::Idle,
            copied: None,
            access,
            is_ticking: false,
            _ticker: None,
        }
    }

    pub(crate) fn is_for(&self, secret: &ResourceKey) -> bool {
        self.secret == *secret
    }

    /// Replaces the key list; a revealed value whose key is gone is dropped at once.
    pub(crate) fn set_keys(&mut self, keys: &[SecretKey]) {
        if self.keys == keys {
            return;
        }
        self.keys = keys.to_vec();
        let keys = &self.keys;
        self.revealed
            .retain(|entry| keys.iter().any(|key| key.name == entry.value.key()));
    }

    fn is_running(&self) -> bool {
        matches!(self.request, ValuesRequest::Running { .. })
    }

    /// Starts the one GET of an action. A no-op when blocked or already running. It never
    /// notifies: it may run inside `AppShell::render`.
    pub(crate) fn run(&mut self, action: SecretAction, cx: &mut Context<Self>) {
        if self.access == ValueAccess::Blocked || self.is_running() {
            return;
        }
        let fetch = Arc::clone(&self.fetch);
        let runtime = cx.global::<ClusterRuntime>().clone();
        let task = cx.spawn(async move |this, cx| {
            let fetched = runtime.spawn(async move { fetch().await }).await;
            let _ = this.update(cx, |view, cx| view.finish(&action, fetched, cx));
        });
        self.request = ValuesRequest::Running { _task: task };
    }

    /// A button press: starts the action and shows the spinner.
    fn press(&mut self, action: SecretAction, cx: &mut Context<Self>) {
        self.run(action, cx);
        cx.notify();
    }

    fn finish(
        &mut self,
        action: &SecretAction,
        fetched: Result<Result<Vec<SecretValue>, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.request = ValuesRequest::Idle;
        let values = match fetched {
            Ok(Ok(values)) => values,
            Ok(Err(error)) => return self.fail(failure_text(&error), cx),
            Err(_) => return self.fail("The request stopped before it finished".to_owned(), cx),
        };
        let kept = match kept_values(values, action) {
            Ok(kept) => kept,
            Err(missing) => return self.fail(missing.text(), cx),
        };
        match action {
            SecretAction::RevealAll | SecretAction::Reveal(_) => {
                reveal(&mut self.revealed, kept, Instant::now());
                self.start_ticker(cx);
            }
            SecretAction::Copy(key) => {
                // Dropped at the end of this arm: Copy never reveals.
                let Some(text) = kept.first().and_then(SecretValue::as_text) else {
                    return self.fail("Binary value".to_owned(), cx);
                };
                match write_private_text(text, cx) {
                    Ok(mark) => {
                        self.copied = Some((key.clone(), Instant::now()));
                        cx.emit(SecretCopied(mark));
                        self.start_ticker(cx);
                    }
                    Err(error) => return self.fail(error.to_string(), cx),
                }
            }
        }
        cx.notify();
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.request = ValuesRequest::Failed {
            message: message.into(),
        };
        cx.notify();
    }

    /// Runs once a second while anything is revealed or just copied.
    fn start_ticker(&mut self, cx: &mut Context<Self>) {
        if self.is_ticking {
            return;
        }
        self.is_ticking = true;
        self._ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keeps_going = this.update(cx, |view, cx| view.tick(Instant::now(), cx));
                if !matches!(keeps_going, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Hides what is due and clears the Copied mark; false when nothing is left to count down.
    fn tick(&mut self, now: Instant, cx: &mut Context<Self>) -> bool {
        expire(&mut self.revealed, now);
        if self
            .copied
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= COPIED_FEEDBACK)
        {
            self.copied = None;
        }
        cx.notify();
        let keeps_going = !self.revealed.is_empty() || self.copied.is_some();
        if !keeps_going {
            self.is_ticking = false;
        }
        keeps_going
    }

    fn hide_all(&mut self, cx: &mut Context<Self>) {
        self.revealed.clear();
        cx.notify();
    }

    fn hide(&mut self, key: &str, cx: &mut Context<Self>) {
        self.revealed.retain(|entry| entry.value.key() != key);
        cx.notify();
    }

    // ---- render ----

    fn blocked_tooltip(&self, enabled: &'static str) -> &'static str {
        if self.access == ValueAccess::Blocked {
            BLOCKED_REASON
        } else {
            enabled
        }
    }

    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let is_blocked = self.access == ValueAccess::Blocked;
        let toggle = if self.revealed.is_empty() {
            Button::new("secret-reveal-all")
                .label("Reveal all (30s)")
                .ghost()
                .xsmall()
                .disabled(is_blocked || self.is_running())
                .tooltip(self.blocked_tooltip("Show every value for 30 seconds"))
                .on_click(cx.listener(|view, _, _, cx| view.press(SecretAction::RevealAll, cx)))
        } else {
            Button::new("secret-hide-all")
                .label("Hide")
                .ghost()
                .xsmall()
                .on_click(cx.listener(|view, _, _, cx| view.hide_all(cx)))
        };
        h_flex()
            .gap_2()
            .items_center()
            .justify_end()
            .when(self.is_running(), |row| row.child(Spinner::new()))
            .child(toggle)
            .into_any_element()
    }

    fn render_alert(&self) -> Option<AnyElement> {
        let ValuesRequest::Failed { message } = &self.request else {
            return None;
        };
        let alert = Alert::error("secret-error", message.clone());
        Some(div().py_1().child(alert).into_any_element())
    }

    fn copy_button(&self, ix: usize, key: &SecretKey, cx: &Context<Self>) -> AnyElement {
        let is_copied = self
            .copied
            .as_ref()
            .is_some_and(|(copied, _)| *copied == key.name);
        let tooltip = if key.is_binary {
            "Binary value"
        } else if is_copied {
            "Clears from the clipboard in 30 s"
        } else {
            self.blocked_tooltip("Copy without revealing")
        };
        let name = key.name.clone();
        Button::new(("secret-copy", ix))
            .label(if is_copied { "Copied" } else { "Copy" })
            .ghost()
            .xsmall()
            .disabled(self.access == ValueAccess::Blocked || key.is_binary || self.is_running())
            .tooltip(tooltip)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.press(SecretAction::Copy(name.clone()), cx);
            }))
            .into_any_element()
    }

    fn render_row(
        &self,
        ix: usize,
        key: &SecretKey,
        now: Instant,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let mono = theme.mono_font_family.clone();
        let name = div()
            .flex_1()
            .min_w_0()
            .truncate()
            .font_family(mono.clone())
            .child(key.name.clone());
        let Some(shown) = self
            .revealed
            .iter()
            .find(|entry| entry.value.key() == key.name)
        else {
            let size = if key.is_binary {
                format!("{} · binary", format_bytes(key.size_bytes))
            } else {
                format_bytes(key.size_bytes)
            };
            let reveal_key = key.name.clone();
            return h_flex()
                .id(("secret-row", ix))
                .gap_2()
                .items_center()
                .py_1()
                .text_sm()
                .child(name)
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .font_family(mono)
                        .child(crate::secret_rows::MASK),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child(size),
                )
                .child(
                    Button::new(("secret-reveal", ix))
                        .label("Reveal")
                        .ghost()
                        .xsmall()
                        .disabled(self.access == ValueAccess::Blocked || self.is_running())
                        .tooltip(self.blocked_tooltip("Show this value for 30 seconds"))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.press(SecretAction::Reveal(reveal_key.clone()), cx);
                        })),
                )
                .child(self.copy_button(ix, key, cx))
                .into_any_element();
        };
        let hide_key = key.name.clone();
        let body: AnyElement =
            match value_display(&shown.value) {
                ValueDisplay::Text {
                    shown: text,
                    is_cut,
                } => v_flex()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .px_2()
                            .py_1p5()
                            .rounded(theme.radius)
                            .bg(theme.muted)
                            .font_family(mono)
                            .text_xs()
                            .child(text.to_owned()),
                    )
                    .when(is_cut, |column| {
                        column.child(div().text_xs().text_color(theme.muted_foreground).child(
                            format!(
                                "… {} in total; Copy for the full value",
                                format_bytes(shown.value.size_bytes())
                            ),
                        ))
                    })
                    .into_any_element(),
                ValueDisplay::Binary { size_bytes } => div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("Binary value, {}", format_bytes(size_bytes)))
                    .into_any_element(),
            };
        v_flex()
            .id(("secret-row", ix))
            .py_1()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(name)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("Hides in {}s", seconds_left(shown.hides_at, now))),
                    )
                    .child(
                        Button::new(("secret-hide", ix))
                            .label("Hide")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |view, _, _, cx| view.hide(&hide_key, cx))),
                    )
                    .child(self.copy_button(ix, key, cx)),
            )
            .child(body)
            .into_any_element()
    }
}

impl Render for SecretValuesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        v_flex()
            .gap_1()
            .child(self.render_header(cx))
            .children(self.render_alert())
            .children(
                self.keys
                    .iter()
                    .enumerate()
                    .map(|(ix, key)| self.render_row(ix, key, now, cx)),
            )
    }
}

#[cfg(test)]
#[path = "secret_values_tests.rs"]
mod secret_values_tests;
