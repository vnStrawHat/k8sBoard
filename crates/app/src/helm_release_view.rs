//! The Helm content of an open release drawer: the Overview diff ("Values changed in rev N") and
//! the Values, Manifest, and Notes tabs. One entity per shown revision; dropping it aborts every
//! request and wipes every text it holds (`HelmText`).
//!
//! Values and notes open masked. A Reveal fetches them once, shows them for 30 s, and drops them;
//! any tab change hides at once, and a changed subject or a closed drawer drops the whole view.
//! While revealed text is on screen, the editor's Copy and Cut go through the private clipboard of
//! 0016, never the normal one. Nothing here logs, traces, or `Debug`-prints a text.

use std::future::Future;
use std::time::{Duration, Instant};

use cluster::{
    ClusterConnection, ClusterError, EnvValues, HelmReleaseDetail, HelmReleaseSummary,
    HelmRevealed, HelmRevisionRef, HelmValuesDiff, ValueChange, ValueVisibility,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Copy, Cut, Editor, EditorState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
};
use zeroize::Zeroizing;

use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::drawer::{DRAWER_SUBJECT_DELAY, DrawerTab, section_title};
use crate::helm_rows::chart_text;
use crate::resource_kind::ResourceKind;
use crate::secret_clipboard::{ClipboardMark, write_private_text};
use crate::secret_values::{REVEAL_DURATION, SecretCopied, ValueAccess};
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ResourceKey;

const TICK: Duration = Duration::from_secs(1);
const BLOCKED_REASON: &str = "Disabled in screenshot runs";
const COPY_FAILED: &str = "Copy failed: the clipboard is unavailable.";
const NO_EARLIER: &str = "No earlier revision";
const HISTORY_FAILED: &str = "The history could not be read.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HelmTab {
    Overview,
    Values,
    Manifest,
    Notes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValuesSource {
    User,
    Computed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValuesLayout {
    Document,
    Diff,
}

/// What the History watch says about the shown revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryState {
    Loading,
    Failed,
    /// Loaded; `earlier` is the highest revision below the shown one.
    Loaded {
        earlier: Option<u32>,
    },
}

/// Emitted by the header's Latest button; the shell shows the latest revision again.
pub(crate) struct ShowLatest;

// ---- Pure core ----

/// The revision and tab a release drawer shows. Overview always uses the latest revision; the
/// other tabs use the revision the History chose, else the latest. Anything that is not a
/// release, a Helm tab, or a loaded row has none, so the view is dropped.
pub(crate) fn helm_subject(
    selected: Option<&ResourceKey>,
    tab: DrawerTab,
    summary: Option<&HelmReleaseSummary>,
    revision: Option<u32>,
) -> Option<(HelmRevisionRef, HelmTab)> {
    let ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        ..
    } = selected?
    else {
        return None;
    };
    let summary = summary?;
    let helm_tab = match tab {
        DrawerTab::Overview => HelmTab::Overview,
        DrawerTab::Values => HelmTab::Values,
        DrawerTab::Manifest => HelmTab::Manifest,
        DrawerTab::Notes => HelmTab::Notes,
        DrawerTab::Containers
        | DrawerTab::Pods
        | DrawerTab::Monitor
        | DrawerTab::Yaml
        | DrawerTab::Events => {
            return None;
        }
    };
    let shown = match helm_tab {
        HelmTab::Overview => summary.revision,
        HelmTab::Values | HelmTab::Manifest | HelmTab::Notes => {
            revision.unwrap_or(summary.revision)
        }
    };
    Some((
        HelmRevisionRef {
            namespace: summary.namespace.clone(),
            release: summary.name.clone(),
            revision: shown,
        },
        helm_tab,
    ))
}

/// The state of one fetched slot, for the pure decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Absent,
    Running,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RevealedSlots {
    payload: SlotState,
    diff: SlotState,
}

/// Everything `next_need` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SlotStates {
    /// An earlier revision is known, so a diff is possible.
    has_earlier: bool,
    detail: SlotState,
    diff: SlotState,
    /// `Some` while a Reveal is active.
    revealed: Option<RevealedSlots>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    Detail,
    Revealed,
    Diff(ValueVisibility),
}

/// The slot the shown state reads, and its state. `None` when nothing can be read (a diff
/// without an earlier revision).
fn wanted(tab: HelmTab, layout: ValuesLayout, slots: &SlotStates) -> Option<(Need, SlotState)> {
    let diff = || match slots.revealed {
        Some(revealed) => (Need::Diff(ValueVisibility::Revealed), revealed.diff),
        None => (Need::Diff(ValueVisibility::Masked), slots.diff),
    };
    let document = || match slots.revealed {
        Some(revealed) => (Need::Revealed, revealed.payload),
        None => (Need::Detail, slots.detail),
    };
    match (tab, layout) {
        (HelmTab::Overview, _) | (HelmTab::Values, ValuesLayout::Diff) => {
            slots.has_earlier.then(diff)
        }
        (HelmTab::Values, ValuesLayout::Document) | (HelmTab::Notes, _) => Some(document()),
        // The manifest is never revealed.
        (HelmTab::Manifest, _) => Some((Need::Detail, slots.detail)),
    }
}

/// The fetch the shown state is missing. Only an `Absent` slot is fetched: a running one is on its
/// way, and a failed one waits for Refresh.
fn next_need(tab: HelmTab, layout: ValuesLayout, slots: SlotStates) -> Option<Need> {
    let (need, state) = wanted(tab, layout, &slots)?;
    (state == SlotState::Absent).then_some(need)
}

/// The highest revision of `history` below `revision`.
pub(crate) fn earlier_revision(
    history: impl IntoIterator<Item = u32>,
    revision: u32,
) -> Option<u32> {
    history
        .into_iter()
        .filter(|number| *number < revision)
        .max()
}

/// What the editor holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShownText {
    Values(ValuesSource, ValueVisibility),
    Manifest(EnvValues),
}

/// The text the editor should hold; `None` when the shown state has no editor (Overview, Notes,
/// or a diff). `is_revealed` is true once the revealed texts have arrived.
fn shown_text(
    tab: HelmTab,
    layout: ValuesLayout,
    source: ValuesSource,
    env: EnvValues,
    is_revealed: bool,
) -> Option<ShownText> {
    match (tab, layout) {
        (HelmTab::Values, ValuesLayout::Document) => {
            let visibility = if is_revealed {
                ValueVisibility::Revealed
            } else {
                ValueVisibility::Masked
            };
            Some(ShownText::Values(source, visibility))
        }
        (HelmTab::Manifest, _) => Some(ShownText::Manifest(env)),
        (HelmTab::Overview | HelmTab::Notes, _) | (HelmTab::Values, ValuesLayout::Diff) => None,
    }
}

fn is_expired(hides_at: Option<Instant>, now: Instant) -> bool {
    hides_at.is_some_and(|hides_at| now >= hides_at)
}

/// Whole seconds left, rounded up, for `Hides in 23s`.
fn seconds_left(hides_at: Instant, now: Instant) -> u64 {
    let left = hides_at.saturating_duration_since(now);
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

/// Any change of tab hides what a Reveal showed.
fn tab_change_hides(from: HelmTab, to: HelmTab) -> bool {
    from != to
}

/// What the Overview part of a release drawer says before it has a diff to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValuesChange {
    /// The History has not loaded.
    Loading,
    /// The History could not be read.
    HistoryFailed,
    /// The History is loaded and has nothing before this revision.
    FirstRevision,
    Diff,
}

fn values_change(history: HistoryState) -> ValuesChange {
    match history {
        HistoryState::Loaded { earlier: Some(_) } => ValuesChange::Diff,
        HistoryState::Loaded { earlier: None } => ValuesChange::FirstRevision,
        HistoryState::Failed => ValuesChange::HistoryFailed,
        HistoryState::Loading => ValuesChange::Loading,
    }
}

/// Why there is nothing to compare with, for the Diff button and the empty diff.
fn no_earlier_text(history: HistoryState) -> &'static str {
    match history {
        HistoryState::Loading => "Loading the history…",
        HistoryState::Failed => HISTORY_FAILED,
        HistoryState::Loaded { .. } => NO_EARLIER,
    }
}

/// A revealed copy: an empty selection copies nothing; otherwise the writer's result, and never
/// another clipboard path. The selection is wiped when this returns.
fn private_copy<E>(
    selection: Zeroizing<String>,
    write: impl FnOnce(&str) -> Result<ClipboardMark, E>,
) -> Option<Result<ClipboardMark, E>> {
    if selection.is_empty() {
        return None;
    }
    Some(write(&selection))
}

/// 403 reads as the missing right, 404 as a pruned revision; anything else as the cluster crate
/// words it (fixed texts only).
fn failure_text(error: &ClusterError) -> SharedString {
    match error {
        ClusterError::Forbidden { .. } => "Not permitted: get secrets".into(),
        ClusterError::Api { code: 404, .. } => {
            "This revision no longer exists: Helm pruned it.".into()
        }
        other => error_text(other).into(),
    }
}

// ---- State ----

/// One fetched piece. Dropping `Running` aborts its request.
enum Fetched<T> {
    Absent,
    Running { _task: Task<()> },
    Ready(T),
    Failed { message: SharedString },
}

impl<T> Fetched<T> {
    fn state(&self) -> SlotState {
        match self {
            Self::Absent => SlotState::Absent,
            Self::Running { .. } => SlotState::Running,
            Self::Ready(_) => SlotState::Ready,
            Self::Failed { .. } => SlotState::Failed,
        }
    }

    fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    fn failure(&self) -> Option<&SharedString> {
        match self {
            Self::Failed { message } => Some(message),
            _ => None,
        }
    }

    fn settle(&mut self, result: Result<T, SharedString>) {
        *self = match result {
            Ok(value) => Self::Ready(value),
            Err(message) => Self::Failed { message },
        };
    }
}

/// A Reveal in progress or showing. Dropping it wipes the texts.
struct Revealed {
    /// First revealed arrival + 30 s; unset until something arrived.
    hides_at: Option<Instant>,
    payload: Fetched<HelmRevealed>,
    diff: Fetched<HelmValuesDiff>,
}

/// Whether a fetch waits for the drawer subject to rest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetchStart {
    Debounced,
    Immediate,
}

/// Helm content of one revision. Dropping it aborts requests and wipes every text.
pub(crate) struct HelmReleaseView {
    connection: ClusterConnection,
    /// The cluster of the release: the same release name exists in several clusters.
    cluster: ClusterRef,
    revision: HelmRevisionRef,
    latest_revision: u32,
    access: ValueAccess,
    tab: HelmTab,
    source: ValuesSource,
    layout: ValuesLayout,
    env: EnvValues,
    /// What the History says: loading, failed, or the highest revision below this one.
    history: HistoryState,
    editor: Entity<EditorState>,
    /// What the editor holds; `set_value` runs only when it changes.
    shown: Option<ShownText>,
    /// Masked; carries no notes text.
    detail: Fetched<HelmReleaseDetail>,
    /// Masked, against `earlier_revision`.
    diff: Fetched<HelmValuesDiff>,
    revealed: Option<Revealed>,
    copy_error: Option<SharedString>,
    /// The first fetch waits for the subject to rest; later ones start at once.
    has_fetched: bool,
    _ticker: Option<Task<()>>,
}

impl EventEmitter<ShowLatest> for HelmReleaseView {}
impl EventEmitter<SecretCopied> for HelmReleaseView {}

/// Where a release view reads from: the cluster and its connection.
pub(crate) struct HelmSource {
    pub(crate) cluster: ClusterRef,
    pub(crate) connection: ClusterConnection,
}

impl HelmReleaseView {
    /// Never notifies: it runs inside `AppShell::render`, and a notify there would re-render
    /// forever. The first fetch waits `DRAWER_SUBJECT_DELAY`, so arrowing through rows sends no
    /// request.
    pub(crate) fn new(
        source: HelmSource,
        revision: HelmRevisionRef,
        latest_revision: u32,
        access: ValueAccess,
        tab: HelmTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
        });
        let mut view = Self {
            connection: source.connection,
            cluster: source.cluster,
            revision,
            latest_revision,
            access,
            tab,
            source: ValuesSource::User,
            layout: ValuesLayout::Document,
            env: EnvValues::Hidden,
            history: HistoryState::Loading,
            editor,
            shown: None,
            detail: Fetched::Absent,
            diff: Fetched::Absent,
            revealed: None,
            copy_error: None,
            has_fetched: false,
            _ticker: None,
        };
        view.advance(window, cx);
        view
    }

    pub(crate) fn is_for(&self, cluster: &ClusterRef, revision: &HelmRevisionRef) -> bool {
        self.cluster == *cluster && self.revision == *revision
    }

    /// The release's latest revision, for the header's Latest button; never notifies.
    pub(crate) fn set_latest(&mut self, latest: u32) {
        self.latest_revision = latest;
    }

    /// The first fetch of what is shown is still in flight, so there is nothing to show yet.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_loading(&self) -> bool {
        match wanted(self.tab, self.layout, &self.slot_states()) {
            Some((_, state)) => matches!(state, SlotState::Absent | SlotState::Running),
            // A failed History is settled: the Overview says so instead of waiting.
            None => self.history == HistoryState::Loading && self.tab == HelmTab::Overview,
        }
    }

    fn slot_states(&self) -> SlotStates {
        SlotStates {
            has_earlier: self.earlier().is_some(),
            detail: self.detail.state(),
            diff: self.diff.state(),
            revealed: self.revealed.as_ref().map(|revealed| RevealedSlots {
                payload: revealed.payload.state(),
                diff: revealed.diff.state(),
            }),
        }
    }

    fn is_blocked(&self) -> bool {
        self.access == ValueAccess::Blocked
    }

    /// Called by the shell's sync on every render; never notifies. A different tab hides what a
    /// Reveal showed at once.
    pub(crate) fn set_tab(&mut self, tab: HelmTab, window: &mut Window, cx: &mut Context<Self>) {
        if tab == self.tab {
            return;
        }
        if tab_change_hides(self.tab, tab) {
            self.revealed = None;
            self.copy_error = None;
        }
        self.tab = tab;
        self.sync_editor(window, cx);
        self.advance(window, cx);
    }

    /// Called once after a History button chose a layout; never notifies.
    pub(crate) fn set_layout(
        &mut self,
        layout: ValuesLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if layout == self.layout {
            return;
        }
        self.layout = layout;
        self.sync_editor(window, cx);
        self.advance(window, cx);
    }

    /// The highest History revision below this one, once the History has loaded.
    fn earlier(&self) -> Option<u32> {
        match self.history {
            HistoryState::Loaded { earlier } => earlier,
            HistoryState::Loading | HistoryState::Failed => None,
        }
    }

    /// Called on every render with what the History says; never notifies. A changed earlier
    /// revision resets the diff.
    pub(crate) fn set_history(
        &mut self,
        history: HistoryState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if history == self.history {
            return;
        }
        let before = self.earlier();
        self.history = history;
        if before != self.earlier() {
            self.diff = Fetched::Absent;
            if let Some(revealed) = &mut self.revealed {
                revealed.diff = Fetched::Absent;
            }
        }
        self.advance(window, cx);
    }

    /// Starts the fetch the shown state is missing, if any.
    fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(need) = next_need(self.tab, self.layout, self.slot_states()) else {
            return;
        };
        let start = if self.has_fetched {
            FetchStart::Immediate
        } else {
            FetchStart::Debounced
        };
        self.fetch(need, start, window, cx);
    }

    fn fetch(
        &mut self,
        need: Need,
        start: FetchStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.has_fetched = true;
        let delay = (start == FetchStart::Debounced).then_some(DRAWER_SUBJECT_DELAY);
        let connection = self.connection.clone();
        let revision = self.revision.clone();
        match need {
            Need::Detail => {
                let env = self.env;
                let task = Self::spawn_fetch(
                    window,
                    cx,
                    delay,
                    async move { connection.helm_release_detail(&revision, env).await },
                    |view, result, window, cx| {
                        view.detail.settle(result);
                        view.arrived(window, cx);
                    },
                );
                self.detail = Fetched::Running { _task: task };
            }
            Need::Revealed => {
                let task = Self::spawn_fetch(
                    window,
                    cx,
                    delay,
                    async move { connection.helm_revealed(&revision).await },
                    |view, result, window, cx| {
                        if let Some(revealed) = &mut view.revealed {
                            revealed.payload.settle(result);
                            revealed.start_countdown();
                        }
                        view.arrived(window, cx);
                    },
                );
                if let Some(revealed) = &mut self.revealed {
                    revealed.payload = Fetched::Running { _task: task };
                }
            }
            Need::Diff(visibility) => {
                let Some(earlier) = self.earlier() else {
                    return;
                };
                let before = HelmRevisionRef {
                    revision: earlier,
                    ..revision.clone()
                };
                let task = Self::spawn_fetch(
                    window,
                    cx,
                    delay,
                    async move {
                        connection
                            .helm_values_diff(&before, &revision, visibility)
                            .await
                    },
                    move |view, result, window, cx| {
                        match visibility {
                            ValueVisibility::Masked => view.diff.settle(result),
                            ValueVisibility::Revealed => {
                                if let Some(revealed) = &mut view.revealed {
                                    revealed.diff.settle(result);
                                    revealed.start_countdown();
                                }
                            }
                        }
                        view.arrived(window, cx);
                    },
                );
                match visibility {
                    ValueVisibility::Masked => self.diff = Fetched::Running { _task: task },
                    ValueVisibility::Revealed => {
                        if let Some(revealed) = &mut self.revealed {
                            revealed.diff = Fetched::Running { _task: task };
                        }
                    }
                }
            }
        }
    }

    /// Runs `request` on the cluster runtime and hands the result to `apply` on this thread.
    /// Dropping the returned task aborts both a pending delay and a running request.
    fn spawn_fetch<T, F>(
        window: &mut Window,
        cx: &mut Context<Self>,
        delay: Option<Duration>,
        request: F,
        apply: impl FnOnce(&mut Self, Result<T, SharedString>, &mut Window, &mut Context<Self>)
        + 'static,
    ) -> Task<()>
    where
        T: Send + 'static,
        F: Future<Output = Result<T, ClusterError>> + Send + 'static,
    {
        let runtime = cx.global::<ClusterRuntime>().clone();
        cx.spawn_in(window, async move |this, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            let fetched = runtime.spawn(request).await;
            let result = match fetched {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(failure_text(&error)),
                Err(_) => Err("The request stopped before it finished".into()),
            };
            let _ = this.update_in(cx, |view, window, cx| apply(view, result, window, cx));
        })
    }

    /// A result arrived: show it, start what is still missing, and repaint.
    fn arrived(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor(window, cx);
        self.advance(window, cx);
        cx.notify();
    }

    /// Puts the right text in the editor, and only when it changed. Masked text again as soon as
    /// a Reveal ends.
    fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .editor_text()
            .map(|(shown, text)| (shown, text.to_owned()));
        let shown = target.as_ref().map(|(shown, _)| *shown);
        if shown == self.shown {
            return;
        }
        let text = target.map(|(_, text)| text).unwrap_or_default();
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        self.shown = shown;
    }

    /// Empties the editor now, so no old text stays while a new fetch runs.
    fn clear_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_value(String::new(), window, cx));
        self.shown = None;
    }

    fn is_revealed_ready(&self) -> bool {
        self.revealed
            .as_ref()
            .is_some_and(|revealed| revealed.payload.ready().is_some())
    }

    fn editor_text(&self) -> Option<(ShownText, &str)> {
        let shown = shown_text(
            self.tab,
            self.layout,
            self.source,
            self.env,
            self.is_revealed_ready(),
        )?;
        let text = match shown {
            ShownText::Values(source, ValueVisibility::Masked) => {
                let detail = self.detail.ready()?;
                match source {
                    ValuesSource::User => detail.user_values.as_str(),
                    ValuesSource::Computed => detail.computed_values.as_str(),
                }
            }
            ShownText::Values(source, ValueVisibility::Revealed) => {
                let payload = self.revealed.as_ref()?.payload.ready()?;
                match source {
                    ValuesSource::User => payload.user.as_str(),
                    ValuesSource::Computed => payload.computed.as_str(),
                }
            }
            ShownText::Manifest(_) => self.detail.ready()?.manifest.as_str(),
        };
        Some((shown, text))
    }

    // ---- actions ----

    /// A no-op when blocked (a screenshot run).
    fn reveal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_blocked() {
            return;
        }
        self.revealed = Some(Revealed {
            hides_at: None,
            payload: Fetched::Absent,
            diff: Fetched::Absent,
        });
        self.advance(window, cx);
        self.start_ticker(window, cx);
        cx.notify();
    }

    fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.revealed = None;
        self.copy_error = None;
        self.sync_editor(window, cx);
        cx.notify();
    }

    /// Runs once a second while a Reveal is active; ends with it.
    fn start_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._ticker = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keeps_going =
                    this.update_in(cx, |view, window, cx| view.tick(Instant::now(), window, cx));
                if !matches!(keeps_going, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Hides what is due; false when nothing is left to count down.
    fn tick(&mut self, now: Instant, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(revealed) = &self.revealed else {
            return false;
        };
        if is_expired(revealed.hides_at, now) {
            self.hide(window, cx);
            return false;
        }
        cx.notify();
        true
    }

    fn set_source(&mut self, source: ValuesSource, window: &mut Window, cx: &mut Context<Self>) {
        self.source = source;
        self.sync_editor(window, cx);
        cx.notify();
    }

    fn toggle_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let layout = match self.layout {
            ValuesLayout::Document => ValuesLayout::Diff,
            ValuesLayout::Diff => ValuesLayout::Document,
        };
        self.layout = layout;
        self.sync_editor(window, cx);
        self.advance(window, cx);
        cx.notify();
    }

    /// Flips the env literals of the manifest and fetches at once.
    fn toggle_env_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.env = match self.env {
            EnvValues::Hidden => EnvValues::Shown,
            EnvValues::Shown => EnvValues::Hidden,
        };
        self.detail = Fetched::Absent;
        self.clear_editor(window, cx);
        self.advance(window, cx);
        cx.notify();
    }

    /// Fetches the shown state again, at once.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.detail = Fetched::Absent;
        self.diff = Fetched::Absent;
        if let Some(revealed) = &mut self.revealed {
            revealed.payload = Fetched::Absent;
            revealed.diff = Fetched::Absent;
        }
        self.copy_error = None;
        self.clear_editor(window, cx);
        self.advance(window, cx);
        cx.notify();
    }

    /// Copies what the editor shows. Only used for text that holds no value (the manifest).
    fn copy_masked(&self, cx: &mut Context<Self>) {
        // Revealed text goes through `copy_revealed` only.
        if self.shows_revealed_text() {
            return;
        }
        let text = self.editor.read(cx).text().to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// The editor's Copy and Cut while revealed text is shown: the selection goes to the private
    /// clipboard, and nothing at all when that is unavailable (never the normal clipboard).
    fn copy_revealed(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let selection = Zeroizing::new(self.editor.read(cx).selected_text().to_string());
        match private_copy(selection, |text| write_private_text(text, cx)) {
            None => {}
            Some(Ok(mark)) => {
                self.copy_error = None;
                cx.emit(SecretCopied(mark));
            }
            Some(Err(_)) => self.copy_error = Some(COPY_FAILED.into()),
        }
        cx.notify();
    }

    fn shows_revealed_text(&self) -> bool {
        matches!(
            self.shown,
            Some(ShownText::Values(_, ValueVisibility::Revealed))
        )
    }

    // ---- render ----

    /// `Reveal` or `Hide` with the countdown. Every Reveal is disabled in a screenshot run.
    fn reveal_controls(&self, label: &'static str, now: Instant, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(revealed) = &self.revealed else {
            let tooltip = if self.is_blocked() {
                BLOCKED_REASON
            } else {
                "Show these for 30 seconds"
            };
            return Button::new("helm-reveal")
                .icon(Icon::new(IconName::Eye))
                .label(label)
                .ghost()
                .xsmall()
                .disabled(self.is_blocked())
                .tooltip(tooltip)
                .on_click(cx.listener(|view, _, window, cx| view.reveal(window, cx)))
                .into_any_element();
        };
        h_flex()
            .gap_2()
            .items_center()
            .children(revealed.hides_at.map(|hides_at| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("Hides in {}s", seconds_left(hides_at, now)))
            }))
            .child(
                Button::new("helm-hide")
                    .icon(Icon::new(IconName::EyeOff))
                    .label("Hide")
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|view, _, window, cx| view.hide(window, cx))),
            )
            .into_any_element()
    }

    /// The diff slot the shown state reads: the revealed one while revealing, else the masked one.
    fn shown_diff(&self) -> (Option<&HelmValuesDiff>, Option<&SharedString>) {
        match &self.revealed {
            Some(revealed) => (revealed.diff.ready(), revealed.diff.failure()),
            None => (self.diff.ready(), self.diff.failure()),
        }
    }

    fn loading(&self, text: &'static str, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .child(Spinner::new())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(text),
            )
            .into_any_element()
    }

    fn error(message: &SharedString) -> AnyElement {
        div()
            .flex_shrink_0()
            .px_3()
            .py_2()
            .child(Alert::error("helm-error", message.clone()).title("Cannot read the release"))
            .into_any_element()
    }

    fn muted(text: impl Into<SharedString>, cx: &App) -> AnyElement {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.into())
            .into_any_element()
    }

    /// The path list of the user or computed values, from the shown diff slot.
    fn diff_list(&self, cx: &Context<Self>) -> AnyElement {
        let (diff, failure) = self.shown_diff();
        let Some(earlier) = self.earlier() else {
            return Self::muted(no_earlier_text(self.history), cx);
        };
        if let Some(message) = failure {
            return Self::error(message);
        }
        let Some(diff) = diff else {
            return self.loading("Reading the release…", cx);
        };
        let (changes, omitted) = match self.source {
            ValuesSource::User => (&diff.user, diff.omitted_user),
            ValuesSource::Computed => (&diff.computed, diff.omitted_computed),
        };
        let mut list = v_flex().gap_1().child(Self::muted(
            format!(
                "Changes from rev {earlier} to rev {}",
                self.revision.revision
            ),
            cx,
        ));
        if changes.is_empty() {
            list = list.child(Self::muted("No value changed.", cx));
        }
        list = list.children(
            changes
                .iter()
                .enumerate()
                .map(|(ix, change)| change_row(ix, change, cx)),
        );
        if omitted > 0 {
            list = list.child(Self::muted(
                format!("{omitted} more changes not shown."),
                cx,
            ));
        }
        list.into_any_element()
    }

    /// The Overview part: "Values changed in rev N", Reveal, and the diff against the previous
    /// revision. Revealed rows are plain elements that offer no text selection.
    fn render_overview(&self, cx: &Context<Self>) -> AnyElement {
        let now = Instant::now();
        let body = match values_change(self.history) {
            ValuesChange::Loading => Self::muted("Loading…", cx),
            ValuesChange::HistoryFailed => Self::muted(HISTORY_FAILED, cx),
            ValuesChange::FirstRevision => Self::muted("This is the first stored revision.", cx),
            ValuesChange::Diff => self.diff_list(cx),
        };
        let can_reveal = self.earlier().is_some();
        v_flex()
            .child(
                h_flex()
                    .items_end()
                    .child(div().flex_1().child(section_title(
                        format!("Values changed in rev {}", self.revision.revision),
                        cx,
                    )))
                    .children(can_reveal.then(|| {
                        div()
                            .mb_2()
                            .child(self.reveal_controls("Reveal (30s)", now, cx))
                    })),
            )
            .child(body)
            .into_any_element()
    }

    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let chart = self
            .detail
            .ready()
            .and_then(|detail| detail.chart.as_ref())
            .map(chart_text);
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .py_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_sm()
                    .font_family(theme.mono_font_family.clone())
                    .child(format!("Revision {}", self.revision.revision)),
            )
            .children(chart.map(|chart| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(chart)
            }))
            .children((self.revision.revision != self.latest_revision).then(|| {
                Button::new("helm-latest")
                    .label("Latest")
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ShowLatest)))
            }))
            .child(div().ml_auto())
            .child(
                Button::new("helm-refresh")
                    .icon(Icon::new(IconName::RefreshCw))
                    .label("Refresh")
                    .ghost()
                    .xsmall()
                    .tooltip("Fetch again")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
            )
            .into_any_element()
    }

    fn render_values_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let now = Instant::now();
        let selected = match self.source {
            ValuesSource::User => 0,
            ValuesSource::Computed => 1,
        };
        let sources = ButtonGroup::new("helm-source")
            .outline()
            .xsmall()
            .child(
                Button::new("helm-user")
                    .label("User-supplied")
                    .selected(selected == 0),
            )
            .child(
                Button::new("helm-computed")
                    .label("Computed")
                    .selected(selected == 1),
            )
            .on_click(cx.listener(|view, clicks: &Vec<usize>, window, cx| {
                let source = match clicks.first() {
                    Some(1) => ValuesSource::Computed,
                    _ => ValuesSource::User,
                };
                view.set_source(source, window, cx);
            }));
        let has_earlier = self.earlier().is_some();
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .py_2()
            .items_center()
            .child(sources)
            .child(
                Button::new("helm-diff")
                    .label("Diff")
                    .outline()
                    .xsmall()
                    .selected(self.layout == ValuesLayout::Diff)
                    .disabled(!has_earlier)
                    .tooltip(if has_earlier {
                        "Compare with the previous revision"
                    } else {
                        no_earlier_text(self.history)
                    })
                    .on_click(cx.listener(|view, _, window, cx| view.toggle_diff(window, cx))),
            )
            .child(div().ml_auto())
            .child(self.reveal_controls("Reveal values (30s)", now, cx))
            .into_any_element()
    }

    fn render_manifest_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let hidden_env = self
            .detail
            .ready()
            .map_or(0, |detail| detail.hidden_env_values);
        let is_shown = self.env == EnvValues::Shown;
        let is_loading = matches!(self.detail.state(), SlotState::Running);
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .py_2()
            .items_center()
            .child(div().ml_auto())
            .when(is_shown || hidden_env > 0, |bar| {
                bar.child(
                    Button::new("helm-env")
                        .label("Env values")
                        .small()
                        .outline()
                        .selected(is_shown)
                        .disabled(is_loading)
                        .tooltip(if is_shown {
                            "Hide env values"
                        } else {
                            "Show the env values this view hides"
                        })
                        .on_click(
                            cx.listener(|view, _, window, cx| view.toggle_env_values(window, cx)),
                        ),
                )
            })
            .child(
                Button::new("helm-copy")
                    .icon(Icon::new(IconName::Copy))
                    .label("Copy")
                    .small()
                    .ghost()
                    .disabled(self.shown.is_none())
                    .tooltip("Copy the manifest (values stay masked)")
                    .on_click(cx.listener(|view, _, _, cx| view.copy_masked(cx))),
            )
            .into_any_element()
    }

    fn render_editor(&self, cx: &Context<Self>) -> AnyElement {
        let editor = Editor::new(&self.editor)
            .readonly(true)
            .bordered(false)
            .text_xs()
            .size_full();
        let mut holder = div().size_full();
        if self.shows_revealed_text() {
            holder = holder
                .capture_action(cx.listener(|view, _: &Copy, _, cx| view.copy_revealed(cx)))
                .capture_action(cx.listener(|view, _: &Cut, _, cx| view.copy_revealed(cx)));
        }
        holder.child(editor).into_any_element()
    }

    fn render_values_body(&self, cx: &Context<Self>) -> AnyElement {
        if self.layout == ValuesLayout::Diff {
            return div()
                .id("helm-diff-body")
                .size_full()
                .overflow_y_scroll()
                .p_4()
                .child(self.diff_list(cx))
                .into_any_element();
        }
        let failure = match &self.revealed {
            Some(revealed) => revealed.payload.failure(),
            None => self.detail.failure(),
        };
        if let Some(message) = failure {
            return Self::error(message);
        }
        if self.shown.is_none() {
            return self.loading("Reading the release…", cx);
        }
        self.render_editor(cx)
    }

    fn render_manifest_body(&self, cx: &Context<Self>) -> AnyElement {
        if let Some(message) = self.detail.failure() {
            return Self::error(message);
        }
        if self.shown.is_none() {
            return self.loading("Reading the release…", cx);
        }
        self.render_editor(cx)
    }

    fn render_notes(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let now = Instant::now();
        if let Some(revealed) = &self.revealed {
            if let Some(message) = revealed.payload.failure() {
                return Self::error(message);
            }
            let Some(payload) = revealed.payload.ready() else {
                return self.loading("Reading the release…", cx);
            };
            return v_flex()
                .size_full()
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .px_4()
                        .py_2()
                        .child(div().ml_auto())
                        .child(self.reveal_controls("Reveal (30s)", now, cx)),
                )
                .child(
                    div()
                        .id("helm-notes")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_4()
                        .pb_4()
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .child(payload.notes.as_str().to_owned()),
                )
                .into_any_element();
        }
        if let Some(message) = self.detail.failure() {
            return Self::error(message);
        }
        let Some(detail) = self.detail.ready() else {
            return self.loading("Reading the release…", cx);
        };
        if detail.notes_lines == 0 {
            return div()
                .p_4()
                .child(Self::muted("This release has no notes.", cx))
                .into_any_element();
        }
        v_flex()
            .gap_2()
            .p_4()
            .child(Self::muted("Notes may contain rendered passwords.", cx))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} lines", detail.notes_lines)),
            )
            .child(div().child(self.reveal_controls("Reveal (30s)", now, cx)))
            .into_any_element()
    }
}

impl Revealed {
    /// The countdown starts at the first arrival, a failure included: the ticker then drops the
    /// Reveal (and its alert) after 30 s instead of spinning for good.
    fn start_countdown(&mut self) {
        if self.hides_at.is_none() {
            self.hides_at = Some(Instant::now() + REVEAL_DURATION);
        }
    }
}

/// One change of the diff: the path, then `+ after`, `- before`, or `before → after`. Plain
/// elements, so a revealed value cannot be selected and copied through the normal clipboard.
fn change_row(ix: usize, change: &ValueChange, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (text, tone) = match (&change.before, &change.after) {
        (None, Some(after)) => (format!("+ {}", after.as_str()), StatusTone::Ok),
        (Some(before), None) => (format!("- {}", before.as_str()), StatusTone::Bad),
        (Some(before), Some(after)) => (
            format!("{} \u{2192} {}", before.as_str(), after.as_str()),
            StatusTone::Warn,
        ),
        (None, None) => (String::new(), StatusTone::Done),
    };
    v_flex()
        .id(("helm-change", ix))
        .py_1()
        .text_xs()
        .font_family(theme.mono_font_family.clone())
        .child(
            div()
                .min_w_0()
                .text_color(theme.muted_foreground)
                .child(change.path.clone()),
        )
        .child(div().min_w_0().text_color(tone_color(tone, cx)).child(text))
        .into_any_element()
}

impl Render for HelmReleaseView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tab == HelmTab::Overview {
            return self.render_overview(cx);
        }
        let toolbar = match self.tab {
            HelmTab::Values => Some(self.render_values_toolbar(cx)),
            HelmTab::Manifest => Some(self.render_manifest_toolbar(cx)),
            HelmTab::Overview | HelmTab::Notes => None,
        };
        let body = match self.tab {
            HelmTab::Values => self.render_values_body(cx),
            HelmTab::Manifest => self.render_manifest_body(cx),
            HelmTab::Notes => self.render_notes(cx),
            HelmTab::Overview => div().into_any_element(),
        };
        v_flex()
            .size_full()
            .child(self.render_header(cx))
            .children(toolbar)
            .children(self.copy_error.clone().map(|message| {
                div()
                    .flex_shrink_0()
                    .px_4()
                    .text_xs()
                    .text_color(tone_color(StatusTone::Bad, cx))
                    .child(message)
            }))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "helm_release_view_tests.rs"]
mod helm_release_view_tests;
