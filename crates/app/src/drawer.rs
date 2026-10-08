use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use cluster::{EventSummary, NamespaceScope};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::active_session::ActiveConnection;
use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::cell_truncation::cell_tooltip;
use crate::clipboard_copy::{copy_button, copy_text, joined_terms};
use crate::cluster_metrics::FeedStatus;
use crate::cluster_session::LiveList;
use crate::helm_release_view::{HelmReleaseView, ValuesLayout};
use crate::history_rings::Resolution;
use crate::monitor_data::MonitorData;
use crate::monitor_source::SourceFetch;
use crate::object_events::events_title;
use crate::port_forward_menu::{
    PortButton, PortButtons, copy_address_tooltip, forward_address_text,
};
use crate::resource_actions::{short_reason, with_next_step};
use crate::resource_kind::ResourceKind;
use crate::secret_values::{SecretAction, SecretValuesView};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::usage_format::group_digits;
use crate::yaml_view::{YamlView, object_ref};

/// How long a drawer subject must rest before its background fetch starts (the object events
/// watch, the first YAML GET): arrowing through rows must not send one request per row.
pub(crate) const DRAWER_SUBJECT_DELAY: Duration = Duration::from_millis(250);
/// The drawer takes this share of the workspace, but never less than `DRAWER_MIN_WIDTH`.
const DRAWER_STANDARD_SHARE: f32 = 0.5;
/// The Pod drawer takes this share: its Containers tab shows the container list beside the detail.
const DRAWER_WIDE_SHARE: f32 = 0.75;
const DRAWER_MIN_WIDTH: f32 = 480.;
/// The workspace width before the first paint has measured it.
const WORKSPACE_FALLBACK_WIDTH: Pixels = px(1100.);

/// How much of the workspace a drawer takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawerSize {
    /// Every drawer but the Pod drawer's.
    Standard,
    /// The Pod drawer.
    Wide,
}

impl DrawerSize {
    /// The size of the drawer that opens for `key`.
    pub(crate) fn of(key: &ResourceKey) -> Self {
        match key {
            ResourceKey::Pod { .. } => Self::Wide,
            ResourceKey::Node { .. } | ResourceKey::Kind { .. } => Self::Standard,
        }
    }
}

/// The width of the drawer overlay in a workspace `workspace` wide: a share of it, at least
/// `DRAWER_MIN_WIDTH`, and never wider than the workspace itself (a small window gets a drawer
/// that fills it).
pub(crate) fn drawer_width(size: DrawerSize, workspace: Pixels) -> Pixels {
    let workspace = f32::from(workspace).max(0.);
    let share = match size {
        DrawerSize::Standard => DRAWER_STANDARD_SHARE,
        DrawerSize::Wide => DRAWER_WIDE_SHARE,
    };
    px((workspace * share).max(DRAWER_MIN_WIDTH).min(workspace))
}
pub(crate) const LABEL_WIDTH: Pixels = px(104.);
/// The longest label column `fitted_label_width` gives.
const FITTED_LABEL_MAX: Pixels = px(260.);
/// Average glyph width of a `text_sm` label (14 px proportional UI font, about 0.55 em).
const LABEL_CHAR_WIDTH: f32 = 7.7;
const LABEL_PADDING: f32 = 8.;
/// The kind drawers have longer labels, such as "Concurrency policy". Anything longer still
/// truncates with a tooltip, or uses `DetailRow::Stacked`.
pub(crate) const WIDE_LABEL_WIDTH: Pixels = px(136.);

/// What the user changes inside an open drawer, plus whether it is open. The tab survives a
/// change of subject on the same screen; `show_screen` resets the tab to Overview. The selected container does not survive a change of subject.
pub(crate) struct DrawerState {
    /// Whether the drawer is shown. The row cursor (`AppShell::selected`) can rest on a row while
    /// the drawer is closed; the flag implies a selection.
    pub(crate) is_open: bool,
    pub(crate) tab: DrawerTab,
    pub(crate) selected_container: Option<String>,
    /// The sub-tab of the container detail. Like `tab`, it survives a change of container and
    /// subject; `show_screen` resets it.
    pub(crate) container_tab: ContainerTab,
    /// The YAML tab's view; `AppShell::sync_yaml_view` keeps it for the shown subject only.
    pub(crate) yaml: Option<Entity<YamlView>>,
    /// The Data section of an open Secret drawer; `AppShell::sync_secret_values` keeps it for the
    /// shown subject only, so dropping it wipes every revealed value.
    pub(crate) secret_values: Option<Entity<SecretValuesView>>,
    /// A menu's Reveal or Copy that waits for the view of its Secret to exist.
    pub(crate) pending_secret_action: Option<(ResourceKey, SecretAction)>,
    /// The Helm content of an open release drawer; `AppShell::sync_helm_view` keeps it for the
    /// shown revision only, so dropping it aborts requests and wipes every text.
    pub(crate) helm: Option<Entity<HelmReleaseView>>,
    /// The revision a History button chose; `None` is the latest. Reset on a change of subject.
    pub(crate) helm_revision: Option<u32>,
    /// A History button's layout that waits for the view of its revision to exist.
    pub(crate) pending_helm_layout: Option<(ResourceKey, ValuesLayout)>,
    /// The Monitor tab: the range survives a change of subject, the scope does not.
    pub(crate) monitor: MonitorState,
    /// The scroll position of the body of an overview drawer, so a menu can move it.
    pub(crate) scroll: ScrollHandle,
    /// The title of an Overview section a menu item asked to see (Roll back…, Show remaining
    /// resources, Show selected pods): the next paint of the drawer scrolls to it and clears the
    /// request. A `Cell` because painting reads the state and never writes it.
    pub(crate) reveal_section: Cell<Option<&'static str>>,
    /// The width of the workspace region, set by each paint of the workspace so the drawer, and
    /// the bars that sit left of it, size from the window. A `Cell` for the same reason as
    /// `reveal_section`.
    workspace_width: Cell<Pixels>,
}

impl DrawerState {
    pub(crate) fn new() -> Self {
        Self {
            is_open: false,
            tab: DrawerTab::Overview,
            selected_container: None,
            container_tab: ContainerTab::Info,
            yaml: None,
            secret_values: None,
            pending_secret_action: None,
            helm: None,
            helm_revision: None,
            pending_helm_layout: None,
            monitor: MonitorState::new(),
            scroll: ScrollHandle::new(),
            reveal_section: Cell::new(None),
            workspace_width: Cell::new(WORKSPACE_FALLBACK_WIDTH),
        }
    }

    pub(crate) fn set_workspace_width(&self, width: Pixels) {
        self.workspace_width.set(width);
    }

    pub(crate) fn workspace_width(&self) -> Pixels {
        self.workspace_width.get()
    }

    pub(crate) fn width(&self, size: DrawerSize) -> Pixels {
        drawer_width(size, self.workspace_width.get())
    }

    /// Moves the body of the open drawer by a key: a page, or to the top or bottom.
    pub(crate) fn scroll_body(&self, step: DrawerScroll) {
        let offset = self.scroll.offset();
        let y = scrolled_offset(
            f32::from(offset.y),
            f32::from(self.scroll.bounds().size.height),
            f32::from(self.scroll.max_offset().y),
            step,
        );
        self.scroll.set_offset(gpui_kit::point(offset.x, px(y)));
    }
}

/// How a key moves the body of an open drawer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawerScroll {
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// The scroll offset after `step`. An offset is 0 at the top and falls to `-max` at the bottom; a
/// page is 90 % of the viewport, so the last line of the page before stays in view.
pub(crate) fn scrolled_offset(offset: f32, viewport: f32, max: f32, step: DrawerScroll) -> f32 {
    let max = max.max(0.);
    let wanted = match step {
        DrawerScroll::PageUp => offset + viewport * 0.9,
        DrawerScroll::PageDown => offset - viewport * 0.9,
        DrawerScroll::Top => 0.,
        DrawerScroll::Bottom => -max,
    };
    wanted.clamp(-max, 0.)
}

/// What the Monitor tab shows: the range, which part of the subject, and
/// the data memoized for the current key (see `MonitorKey`).
pub(crate) struct MonitorState {
    pub(crate) range: MonitorRange,
    pub(crate) scope: MonitorScope,
    pub(crate) cache: Option<MonitorCache>,
    /// The metrics source query of the shown Monitor (spec 0048); `Some` exactly while the source is
    /// ready, the subject has a target, and a Monitor tab shows. It lives with the range and scope
    /// it was fetched for, so another drawer subject drops it.
    pub(crate) source: Option<SourceFetch>,
}

impl MonitorState {
    pub(crate) fn new() -> Self {
        Self {
            range: MonitorRange::Minutes15,
            scope: MonitorScope::Total,
            cache: None,
            source: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MonitorRange {
    Minutes15,
    Hour1,
    Hours6,
    Hours24,
    Days7,
    Days30,
}

impl MonitorRange {
    /// What the app's own sampler can show: it keeps 24 hours.
    pub(crate) const SAMPLER: [Self; 4] =
        [Self::Minutes15, Self::Hour1, Self::Hours6, Self::Hours24];
    /// What a ready metrics source adds to: 7 and 30 days.
    pub(crate) const SOURCE: [Self; 6] = [
        Self::Minutes15,
        Self::Hour1,
        Self::Hours6,
        Self::Hours24,
        Self::Days7,
        Self::Days30,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Minutes15 => "15m",
            Self::Hour1 => "1h",
            Self::Hours6 => "6h",
            Self::Hours24 => "24h",
            Self::Days7 => "7d",
            Self::Days30 => "30d",
        }
    }

    pub(crate) fn duration(self) -> Duration {
        let minutes = match self {
            Self::Minutes15 => 15,
            Self::Hour1 => 60,
            Self::Hours6 => 6 * 60,
            Self::Hours24 => 24 * 60,
            Self::Days7 => 7 * 24 * 60,
            Self::Days30 => 30 * 24 * 60,
        };
        Duration::from_secs(minutes * 60)
    }

    /// The step of a metrics source query for this range (the `cluster::RANGE_STEPS` table).
    pub(crate) fn source_step(self) -> Duration {
        let duration = self.duration();
        cluster::RANGE_STEPS
            .iter()
            .find(|(span, _)| *span == duration)
            .map_or(Duration::from_secs(60), |(_, step)| *step)
    }

    /// 7d and 30d: nothing reads the sampler for them, which keeps 24 hours.
    pub(crate) fn is_long(self) -> bool {
        matches!(self, Self::Days7 | Self::Days30)
    }

    /// The short ranges read the fine ticks; the long ones read the coarse points. The two source
    /// ranges are coarse too, but no code reads the sampler for them.
    pub(crate) fn resolution(self) -> Resolution {
        match self {
            Self::Minutes15 | Self::Hour1 => Resolution::Fine,
            Self::Hours6 | Self::Hours24 | Self::Days7 | Self::Days30 => Resolution::Coarse,
        }
    }
}

/// Which part of the subject the charts show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MonitorScope {
    Total,
    /// A container of a pod, or a pod of a workload.
    Part(String),
}

/// What the cached data was built for. The cache is reused while the key is the same, so a hover
/// repaint or an unrelated notify never rebuilds the series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MonitorKey {
    /// The object and its cluster: the same name exists in several clusters.
    pub(crate) subject: ClusterObject,
    pub(crate) container: Option<String>,
    /// The feed's `tick_count()`.
    pub(crate) ticks: u64,
    /// The kubelet feed's `tick_count()` and status: its cards and notices follow them.
    pub(crate) kubelet_ticks: u64,
    pub(crate) kubelet_status: FeedStatus,
    pub(crate) scope: MonitorScope,
    pub(crate) range: MonitorRange,
}

pub(crate) struct MonitorCache {
    pub(crate) key: MonitorKey,
    pub(crate) data: MonitorData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerTab {
    Info,
    Env,
    Mounts,
    Logs,
    Monitor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawerTab {
    Overview,
    Containers,
    /// A node drawer's pods.
    Pods,
    Monitor,
    Yaml,
    Events,
    /// The Helm tabs of a release drawer.
    Values,
    Manifest,
    Notes,
}

/// The tabs a drawer shows, in wireframe order. An event's own drawer has no events of its own,
/// and a key the cluster crate cannot address has no YAML tab.
pub(crate) fn drawer_tabs(key: &ResourceKey) -> &'static [DrawerTab] {
    let has_yaml = object_ref(key).is_some();
    match key {
        ResourceKey::Pod { .. } if has_yaml => &[
            DrawerTab::Overview,
            DrawerTab::Containers,
            DrawerTab::Monitor,
            DrawerTab::Yaml,
            DrawerTab::Events,
        ],
        ResourceKey::Pod { .. } => &[
            DrawerTab::Overview,
            DrawerTab::Containers,
            DrawerTab::Monitor,
            DrawerTab::Events,
        ],
        ResourceKey::Kind {
            kind: ResourceKind::Events,
            ..
        } if has_yaml => &[DrawerTab::Overview, DrawerTab::Yaml],
        ResourceKey::Kind {
            kind: ResourceKind::Events,
            ..
        } => &[DrawerTab::Overview],
        ResourceKey::Node { .. } if has_yaml => &[
            DrawerTab::Overview,
            DrawerTab::Pods,
            DrawerTab::Monitor,
            DrawerTab::Yaml,
            DrawerTab::Events,
        ],
        ResourceKey::Node { .. } => &[
            DrawerTab::Overview,
            DrawerTab::Pods,
            DrawerTab::Monitor,
            DrawerTab::Events,
        ],
        // A release has no YAML (the Secret is a masked blob) and no events of its own; its Helm
        // tabs show the values, manifest, and notes.
        ResourceKey::Kind {
            kind: ResourceKind::HelmReleases,
            ..
        } => &[
            DrawerTab::Overview,
            DrawerTab::Values,
            DrawerTab::Manifest,
            DrawerTab::Notes,
        ],
        ResourceKey::Kind { kind, .. } if kind.has_monitor() && has_yaml => &[
            DrawerTab::Overview,
            DrawerTab::Monitor,
            DrawerTab::Yaml,
            DrawerTab::Events,
        ],
        ResourceKey::Kind { kind, .. } if kind.has_monitor() => {
            &[DrawerTab::Overview, DrawerTab::Monitor, DrawerTab::Events]
        }
        ResourceKey::Kind { .. } if has_yaml => {
            &[DrawerTab::Overview, DrawerTab::Yaml, DrawerTab::Events]
        }
        ResourceKey::Kind { .. } => &[DrawerTab::Overview, DrawerTab::Events],
    }
}

/// `tab` when the drawer has it, else `Overview`.
pub(crate) fn shown_tab(tabs: &[DrawerTab], tab: DrawerTab) -> DrawerTab {
    if tabs.contains(&tab) {
        tab
    } else {
        DrawerTab::Overview
    }
}

/// The counts in the tab titles: containers for a pod drawer's Containers tab, pods for a node
/// drawer's Pods tab (`None` without a live cluster).
#[derive(Clone, Copy, Default)]
pub(crate) struct TabCounts {
    pub(crate) containers: usize,
    pub(crate) pods: Option<usize>,
}

/// The label of each tab.
pub(crate) fn tab_titles(
    tabs: &[DrawerTab],
    counts: TabCounts,
    events: Option<&LiveList<EventSummary>>,
) -> Vec<(DrawerTab, SharedString)> {
    tabs.iter()
        .map(|&tab| {
            let title = match tab {
                DrawerTab::Overview => "Overview".to_owned(),
                DrawerTab::Containers => format!("Containers {}", counts.containers),
                DrawerTab::Pods => counts
                    .pods
                    .map_or_else(|| "Pods".to_owned(), |pods| format!("Pods {pods}")),
                DrawerTab::Monitor => "Monitor".to_owned(),
                DrawerTab::Yaml => "YAML".to_owned(),
                DrawerTab::Events => events_title(events),
                DrawerTab::Values => "Values".to_owned(),
                DrawerTab::Manifest => "Manifest".to_owned(),
                DrawerTab::Notes => "Notes".to_owned(),
            };
            (tab, title.into())
        })
        .collect()
}

/// The underline tab bar shared by every drawer; `None` when there is only one tab. A click
/// calls `AppShell::set_drawer_tab`.
pub(crate) fn drawer_tab_bar(
    tabs: Vec<(DrawerTab, SharedString)>,
    shown: DrawerTab,
    cx: &Context<AppShell>,
) -> Option<AnyElement> {
    if tabs.len() < 2 {
        return None;
    }
    let selected_index = tabs.iter().position(|(tab, _)| *tab == shown).unwrap_or(0);
    let order: Vec<DrawerTab> = tabs.iter().map(|(tab, _)| *tab).collect();
    let bar = TabBar::new("drawer-tabs")
        .underline()
        .selected_index(selected_index)
        .on_click(cx.listener(move |shell, index: &usize, _, cx| {
            if let Some(&tab) = order.get(*index) {
                shell.set_drawer_tab(tab, cx);
            }
        }))
        // Same horizontal padding as the drawer header.
        .prefix(div().w_4())
        .children(tabs.into_iter().map(|(_, title)| Tab::new().label(title)));
    Some(bar.into_any_element())
}

/// The YAML tab body: the view fills the drawer, and there is nothing while it is not created yet.
pub(crate) fn yaml_body(state: &DrawerState) -> DrawerBody {
    DrawerBody::Filling(match &state.yaml {
        Some(view) => view.clone().into_any_element(),
        None => div().into_any_element(),
    })
}

/// The Helm tabs: the view fills the drawer, and there is nothing while it is not created yet.
pub(crate) fn helm_body(state: &DrawerState) -> DrawerBody {
    DrawerBody::Filling(match &state.helm {
        Some(view) => view.clone().into_any_element(),
        None => div().into_any_element(),
    })
}

pub(crate) type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(crate) struct DrawerHeader {
    /// The kind icon in the chip, and the kind name shown before the object name.
    pub(crate) kind_icon: IconName,
    pub(crate) kind_name: SharedString,
    pub(crate) name: SharedString,
    pub(crate) subtitle: AnyElement,
    /// The ⋯ button with its dropdown menu.
    pub(crate) menu: AnyElement,
    pub(crate) on_close: ClickHandler,
    pub(crate) navigation: DrawerNavigation,
}

/// The header's navigation controls (spec 0056): Back with its "from X" label, and Previous / Next
/// over the table rows. The default shows neither.
#[derive(Default)]
pub(crate) struct DrawerNavigation {
    pub(crate) back: Option<BackTarget>,
    /// `None` hides Previous / Next.
    pub(crate) rows: Option<RowControls>,
    /// Runs on a left press anywhere in the drawer: the page keys then move the drawer, not the table.
    pub(crate) on_press: Option<PressHandler>,
}

/// A left press in the drawer.
pub(crate) type PressHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// The Back button: an arrow whose tooltip names the place it leads to.
pub(crate) struct BackTarget {
    pub(crate) tooltip: SharedString,
    pub(crate) on_click: ClickHandler,
}

/// Previous / Next with the cursor position. `position` is `None` while the subject is not a
/// visible row; the buttons are then disabled, as they are for a table of one row.
pub(crate) struct RowControls {
    pub(crate) position: Option<(usize, usize)>,
    pub(crate) on_previous: ClickHandler,
    pub(crate) on_next: ClickHandler,
}

impl RowControls {
    fn can_step(&self) -> bool {
        self.position.is_some_and(|(_, visible)| visible > 1)
    }
}

/// What the shell hands a drawer besides its subject: the Forward buttons' state and the header's
/// navigation controls.
pub(crate) struct DrawerChrome<'a> {
    pub(crate) forward: &'a PortButtons<'a>,
    pub(crate) navigation: DrawerNavigation,
}

/// What fills a drawer below its tab bar.
pub(crate) enum DrawerBody {
    /// Padded, and scrolled by the frame.
    Scrolling(AnyElement),
    /// Fills the rest with no padding; the content (the code editor) scrolls itself.
    Filling(AnyElement),
    /// Scrolled like `Scrolling`, but the sections are the direct children of the scrolled box, so
    /// `scroll` can bring one of them to the top (Roll back… shows the Revisions).
    Sections {
        sections: Vec<AnyElement>,
        scroll: ScrollHandle,
    },
}

/// The shared frame: header, subtitle, optional tab bar, and the body. It is a
/// plain element laid over the workspace (the caller's container is `.relative()`), not
/// the kit `Sheet`: that one covers the sidebar and takes focus from the table.
pub(crate) fn drawer_frame(
    header: DrawerHeader,
    tabs: Option<AnyElement>,
    body: DrawerBody,
    width: Pixels,
    scroll: &ScrollHandle,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let on_press = header.navigation.on_press.clone();
    v_flex()
        .key_context("Drawer")
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(width)
        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
            if let Some(on_press) = &on_press {
                on_press(window, cx);
            }
        })
        .bg(theme.background)
        .border_l_1()
        .border_color(theme.border)
        .shadow_lg()
        // Clicks must not fall through to the table underneath.
        .occlude()
        .child(header_row(header, cx))
        .when_some(tabs, |this, tabs| this.child(tabs))
        .child(match body {
            // The padding sits on a child of the scrolled box: padding on the scrolled box itself is
            // left out of the scroll extent, so the last line stopped short of the bottom edge.
            DrawerBody::Scrolling(body) => div()
                .id("drawer-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .child(div().p_4().child(body))
                .into_any_element(),
            DrawerBody::Filling(body) => div()
                .id("drawer-body")
                .flex_1()
                .min_h_0()
                .child(body)
                .into_any_element(),
            // The sections are direct children of the scrolled box: `scroll_to_top_of_item` counts
            // them, so one box around them would leave Roll back… and Show remaining resources
            // with nothing to scroll to. The bottom padding is a spacer for the reason above.
            DrawerBody::Sections { sections, scroll } => v_flex()
                .id("drawer-body")
                .flex_1()
                .min_h_0()
                .px_4()
                .pt_4()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .children(sections)
                .child(div().h_4().flex_shrink_0())
                .into_any_element(),
        })
}

fn header_row(header: DrawerHeader, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let on_close = header.on_close;
    let DrawerNavigation { back, rows, .. } = header.navigation;
    v_flex()
        .flex_shrink_0()
        .gap_1()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .children(back.map(back_button))
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(theme.radius)
                        .bg(theme.muted)
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(header.kind_icon).size_4()),
                )
                // Like the Topology card captions. It never shrinks, so a long name is the one
                // that truncates.
                .child(
                    truncated_text_with_tooltip(
                        "drawer-kind",
                        kind_label(&header.kind_name),
                        header.kind_name.clone(),
                    )
                    .flex_shrink_0()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground),
                )
                .child(
                    truncated_text("drawer-title", header.name.clone())
                        .min_w_0()
                        .font_semibold()
                        .font_family(theme.mono_font_family.clone()),
                )
                .child(copy_button("drawer-title-copy", header.name))
                // Pushes the menu and close buttons to the right edge.
                .child(div().flex_1())
                .children(rows.map(|rows| row_controls(rows, cx)))
                .child(header.menu)
                .child(
                    Button::new("drawer-close")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::X))
                        .on_click(move |event, window, cx| on_close(event, window, cx)),
                ),
        )
        .child(header.subtitle)
}

/// The kind caption before the object name: the kubectl short name in upper case (`STS`, `SVC`),
/// or the whole kind when it has none (`POD`, `HELM RELEASE`, a custom kind). The tooltip keeps the
/// full kind.
fn kind_label(kind_name: &str) -> String {
    match ResourceKind::from_object_kind(kind_name) {
        Some(kind) => kind.short_kind().to_uppercase(),
        None => kind_name.to_uppercase(),
    }
}

/// `←`: the arrow alone; the tooltip names the place Back leads to.
fn back_button(back: BackTarget) -> impl IntoElement {
    let on_click = back.on_click;
    Button::new("drawer-back")
        .ghost()
        .small()
        .icon(Icon::new(IconName::ArrowLeft))
        .tooltip(back.tooltip)
        .on_click(move |event, window, cx| on_click(event, window, cx))
}

/// `[^] [v] 12 of 40`: the row cursor controls. The text shows only while the buttons can step.
fn row_controls(rows: RowControls, cx: &App) -> impl IntoElement {
    let can_step = rows.can_step();
    let position = rows
        .position
        .filter(|_| can_step)
        .map(|(row, visible)| format!("{} of {}", group_digits(row), group_digits(visible)));
    let (on_previous, on_next) = (rows.on_previous, rows.on_next);
    h_flex()
        .items_center()
        .child(
            Button::new("drawer-previous-row")
                .ghost()
                .small()
                .icon(Icon::new(IconName::ChevronUp))
                .tooltip("Previous row (K)")
                .disabled(!can_step)
                .on_click(move |event, window, cx| on_previous(event, window, cx)),
        )
        .child(
            Button::new("drawer-next-row")
                .ghost()
                .small()
                .icon(Icon::new(IconName::ChevronDown))
                .tooltip("Next row (J)")
                .disabled(!can_step)
                .on_click(move |event, window, cx| on_next(event, window, cx)),
        )
        .children(position.map(|text| {
            div()
                .px_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(text)
        }))
}

/// The ⋯ button; the caller attaches the dropdown menu to it.
pub(crate) fn menu_button() -> Button {
    Button::new("drawer-menu")
        .ghost()
        .small()
        .icon(Icon::new(IconName::Ellipsis))
}

/// A section heading inside a drawer body: semibold, with a rule below it and room above, so the
/// groups of an Overview read as separate blocks. The one heading every drawer kind uses.
pub(crate) fn section_title(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    section_heading(text, cx).mt_6()
}

/// The heading of the first section of a body: the body's own padding is the only room above it.
pub(crate) fn first_section_title(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    section_heading(text, cx)
}

fn section_heading(text: impl Into<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .mb_2()
        .pb_1p5()
        .border_b_1()
        .border_color(theme.border)
        .text_sm()
        .font_semibold()
        .text_color(theme.foreground)
        .child(text.into())
}

/// The Annotations section of an Overview: the terms as plain chips like Labels. The terms are
/// `key=value`, already cut and masked by the cluster crate.
pub(crate) fn annotations_section(annotations: &[String], cx: &App) -> Vec<AnyElement> {
    let terms: Vec<SharedString> = annotations
        .iter()
        .cloned()
        .map(SharedString::from)
        .collect();
    vec![
        section_title("Annotations", cx).into_any_element(),
        chips("annotations", &terms, cx),
    ]
}

/// A label with its value, in the two-column layout shared by both drawers.
pub(crate) fn detail_row(
    label: impl Into<SharedString>,
    value: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    labeled_row(LABEL_WIDTH, label.into(), value, cx)
}

/// The label column that fits the longest of `labels`, between `LABEL_WIDTH` and
/// `FITTED_LABEL_MAX`. The width is an estimate (chars times the average glyph width), since the
/// UI font is proportional and text is not measured before layout.
pub(crate) fn fitted_label_width<'a>(labels: impl Iterator<Item = &'a str>) -> Pixels {
    let longest = labels.map(|label| label.chars().count()).max().unwrap_or(0);
    let estimate = px(longest as f32 * LABEL_CHAR_WIDTH + LABEL_PADDING);
    estimate.max(LABEL_WIDTH).min(FITTED_LABEL_MAX)
}

/// `detail_row` with a label column of the given width.
pub(crate) fn detail_row_with_label_width(
    label_width: Pixels,
    label: impl Into<SharedString>,
    value: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    labeled_row(label_width, label.into(), value, cx)
}

/// `detail_row` with the wider label column of the kind drawers.
pub(crate) fn wide_detail_row(
    label: impl Into<SharedString>,
    value: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    labeled_row(WIDE_LABEL_WIDTH, label.into(), value, cx)
}

fn labeled_row(
    label_width: Pixels,
    label: SharedString,
    value: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .gap_3()
        .py_1()
        .items_start()
        .text_sm()
        .child(
            // Dynamic labels (container and key names) can be long, so the full text is a tooltip.
            truncated_text(label.clone(), label)
                .w(label_width)
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground),
        )
        // `overflow_hidden` keeps a long value inside the drawer; text values ellipsize.
        .child(div().flex_1().min_w_0().overflow_hidden().child(value))
}

/// Text cut with an ellipsis that shows its full value in a tooltip on hover. The `id` must
/// be unique among the elements that can be on screen together.
pub(crate) fn truncated_text(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
) -> Stateful<Div> {
    let text = text.into();
    truncated_text_with_tooltip(id, text.clone(), text)
}

/// Like `truncated_text`, with a tooltip that says more than the text: a short form that stands
/// for a longer one.
pub(crate) fn truncated_text_with_tooltip(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    tooltip: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .truncate()
        .child(text.into())
        .tooltip(cell_tooltip(tooltip.into()))
}

/// A muted dash for a value the object does not have.
pub(crate) fn absent_text(cx: &App) -> impl IntoElement {
    div().text_color(cx.theme().muted_foreground).child("—")
}

/// The value as text, or the muted dash when it is missing.
pub(crate) fn value_or_absent(value: Option<&str>, cx: &App) -> AnyElement {
    match value {
        Some(text) => div().truncate().child(text.to_owned()).into_any_element(),
        None => absent_text(cx).into_any_element(),
    }
}

/// `created 3d ago` for a subtitle; `None` when the creation time is unknown.
pub(crate) fn created_text(
    created_at: Option<jiff::Timestamp>,
    now: jiff::Timestamp,
) -> Option<String> {
    created_at.map(|created_at| format!("created {} ago", format_age(Some(created_at), now)))
}

/// The one look of text that opens another object: link color, underline, pointer, and an
/// `Open {name}` tooltip. A list row keeps its own click and draws only the name with this.
pub(crate) fn link_style(element: Stateful<Div>, name: &SharedString, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let tooltip_text = SharedString::from(format!("Open {name}"));
    element
        .min_w_0()
        .truncate()
        .cursor_pointer()
        .font_family(theme.mono_font_family.clone())
        .text_color(theme.link)
        .underline()
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
}

/// The chokepoint every link click goes through: a denied or out-of-scope target is refused with
/// a notice in `follow_link`, anything else opens its screen.
pub(crate) fn open_link(
    shell: &mut AppShell,
    target: ResourceKey,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    shell.follow_link(target, window, cx);
}

/// A mono value that opens `target` on its own screen, named like every object in a drawer (see
/// `object_text`): `pod:web-0`, or `pod:lab-shop/web-0` when the open session's scope covers
/// more than one namespace, so two objects of the same name are told apart (UX round 3, P33).
pub(crate) fn link_text(
    id: usize,
    text: &SharedString,
    target: ResourceKey,
    cx: &Context<AppShell>,
) -> AnyElement {
    let text = object_text(text.clone(), Some(&target), cx);
    link_style(div().id(("link", id)), &text, cx)
        .on_click(cx.listener(move |shell, _, window, cx| {
            open_link(shell, target.clone(), window, cx);
        }))
        .child(text)
        .into_any_element()
}

/// How a drawer names an object in text or in a link: `kind: text`, the kind in its lowercase
/// kubectl short form, with the namespace of `target` in front of the text when the open session's
/// scope covers more than one namespace. Every object mention in a drawer goes through this one
/// rule; a mention without a `target` stays as given.
pub(crate) fn object_text(
    text: impl Into<SharedString>,
    target: Option<&ResourceKey>,
    cx: &App,
) -> SharedString {
    let text = text.into();
    match target {
        Some(target) => named_object_text(&text, target, scope_has_many_namespaces(cx)),
        None => text,
    }
}

/// Whether the open session lists more than one namespace: All, or several picked. A drawer link
/// is built while the shell renders, so the scope comes from the session entity, not the shell.
fn scope_has_many_namespaces(cx: &App) -> bool {
    cx.try_global::<ActiveConnection>()
        .and_then(|active| active.session.upgrade())
        .and_then(|session| {
            session
                .read(cx)
                .live()
                .map(|live| scope_shows_namespace(&live.scope))
        })
        .unwrap_or(false)
}

/// Whether links name their namespace under `scope`: every scope but a single namespace. Pure.
fn scope_shows_namespace(scope: &NamespaceScope) -> bool {
    !matches!(scope, NamespaceScope::Named(_))
}

/// `kind: [namespace/]name` for `target`. A leading namespace or kind that `text` already carries
/// (`lab-shop/deployment/web`, `replicaset/web-abc`) is dropped first, so each reads once. Pure.
pub(crate) fn named_object_text(
    text: &str,
    target: &ResourceKey,
    shows_namespace: bool,
) -> SharedString {
    let (short_kind, kind_name, namespace) = match target {
        ResourceKey::Pod { namespace, .. } => ("pod".to_owned(), "Pod", Some(namespace)),
        ResourceKey::Node { .. } => ("node".to_owned(), "Node", None),
        ResourceKey::Kind {
            kind, namespace, ..
        } => (kind.short_kind(), kind.display_name(), namespace.as_ref()),
    };
    let mut name = text;
    if let Some(rest) = namespace.and_then(|namespace| {
        name.strip_prefix(namespace.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
    }) {
        name = rest;
    }
    if let Some((word, rest)) = name.split_once('/')
        && (word.eq_ignore_ascii_case(kind_name) || word.eq_ignore_ascii_case(&short_kind))
    {
        name = rest;
    }
    match namespace {
        Some(namespace) if shows_namespace => format!("{short_kind}:{namespace}/{name}").into(),
        _ => format!("{short_kind}:{name}").into(),
    }
}

/// A list row's object name: link-styled text inside a row whose own click opens the object. The
/// caller names the object with `named_object_text`, so the row reads like every other link.
pub(crate) fn link_name(id: usize, name: impl Into<SharedString>, cx: &App) -> AnyElement {
    let name = name.into();
    link_style(div().id(("link-name", id)), &name, cx)
        .child(name)
        .into_any_element()
}

/// Wrapping chips, or a dash when there are none. A click on a chip copies its text, and a button
/// after the set copies every chip, one per line. The `id` must be unique among the chip sets
/// that can be on screen together.
pub(crate) fn chips(id: impl Into<ElementId>, terms: &[SharedString], cx: &App) -> AnyElement {
    if terms.is_empty() {
        return absent_text(cx).into_any_element();
    }
    let id = id.into();
    let theme = cx.theme();
    let chips =
        h_flex()
            .flex_1()
            .min_w_0()
            .flex_wrap()
            .gap_1()
            .children(terms.iter().enumerate().map(|(index, term)| {
                let chip_id: ElementId = (id.clone(), format!("chip-{index}")).into();
                let selector = chip_id.to_string();
                let text = term.clone();
                div()
                    .id(chip_id)
                    .debug_selector(move || selector)
                    .max_w_full()
                    .truncate()
                    .px_1p5()
                    .rounded(theme.radius)
                    .bg(theme.muted)
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .cursor_pointer()
                    .hover(|chip| chip.bg(theme.accent))
                    .tooltip(|window, cx| Tooltip::new("Click to copy").build(window, cx))
                    .on_click(move |_, _, cx| copy_text(&text, cx))
                    .child(term.clone())
            }));
    h_flex()
        .items_start()
        .gap_1()
        .child(chips)
        .child(copy_button((id, "copy-all"), joined_terms(terms)))
        .into_any_element()
}

/// A port with its Forward button: Forward to start, `● localhost:19090` (click copies) and a
/// Stop button while a forward of the port runs, or a disabled button whose tooltip says why (spec 0035).
pub(crate) fn port_row(
    text: &SharedString,
    id: usize,
    button: &PortButton,
    on_click: Option<ClickHandler>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    // Under the port, so a disabled Forward says why without cutting the port's own text.
    let mut reason_line: Option<AnyElement> = None;
    let action: AnyElement = match (button, on_click) {
        // The address copies and a separate Stop stops, so a click on the address never ends the
        // forward (UX walk I3).
        (PortButton::Live { local_port, .. }, Some(on_click)) => {
            let address = forward_address_text(*local_port);
            let tooltip = copy_address_tooltip(*local_port);
            let shown = format!("● {address}");
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .id(("forward-address", id))
                        .cursor_pointer()
                        .text_xs()
                        .text_color(theme.success)
                        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                        .on_click(move |_, _, cx| copy_text(&address, cx))
                        .child(shown),
                )
                .child(
                    Button::new(("forward-stop", id))
                        .label("Stop")
                        .xsmall()
                        .ghost()
                        .tooltip("Stop this forward")
                        .on_click(move |event, window, cx| on_click(event, window, cx)),
                )
                .into_any_element()
        }
        (PortButton::Offer, Some(on_click)) => Button::new(("forward", id))
            .label("Forward")
            .icon(Icon::new(IconName::ArrowLeftRight))
            .xsmall()
            .ghost()
            .on_click(move |event, window, cx| on_click(event, window, cx))
            .into_any_element(),
        // A tooltip alone hides why Forward is off, so the short reason is drawn too.
        (PortButton::Disabled(reason), _) => {
            let full = with_next_step(reason);
            let tooltip = full.clone();
            reason_line = Some(
                div()
                    .id(("forward-reason", id))
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .child(short_reason(reason))
                    .into_any_element(),
            );
            Button::new(("forward", id))
                .label("Forward")
                .icon(Icon::new(IconName::ArrowLeftRight))
                .xsmall()
                .ghost()
                .disabled(true)
                .tooltip(full)
                .into_any_element()
        }
        // A button state without its click is not drawn as clickable.
        (PortButton::Live { .. } | PortButton::Offer, None) => Button::new(("forward", id))
            .label("Forward")
            .xsmall()
            .ghost()
            .disabled(true)
            .into_any_element(),
    };
    h_flex()
        .gap_2()
        .py_1()
        .items_center()
        .text_sm()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    truncated_text(("port", id), text.clone())
                        .font_family(theme.mono_font_family.clone()),
                )
                .children(reason_line),
        )
        .child(action)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fitted_label_width_grows_with_the_longest_label_up_to_a_cap() {
        assert_eq!(fitted_label_width(["data", "tmp"].into_iter()), LABEL_WIDTH);
        assert_eq!(fitted_label_width(std::iter::empty()), LABEL_WIDTH);
        let medium = "usr-local-share-ca-certificates-xx".to_owned();
        assert!(fitted_label_width([medium.as_str()].into_iter()) > LABEL_WIDTH);
        let long = "a".repeat(200);
        assert_eq!(
            fitted_label_width([long.as_str()].into_iter()),
            FITTED_LABEL_MAX
        );
    }

    #[test]
    fn links_name_their_namespace_unless_the_scope_is_one_namespace() {
        assert!(!scope_shows_namespace(&NamespaceScope::Named(
            "a".to_owned()
        )));
        assert!(scope_shows_namespace(&NamespaceScope::All));
        assert!(scope_shows_namespace(&NamespaceScope::of_namespaces([
            "a".to_owned(),
            "b".to_owned()
        ])));
    }

    fn pod(name: &str) -> ResourceKey {
        ResourceKey::Pod {
            namespace: "lab-shop".to_owned(),
            name: name.to_owned(),
        }
    }

    fn kind_key(kind: ResourceKind, namespace: Option<&str>, name: &str) -> ResourceKey {
        ResourceKey::Kind {
            kind,
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
        }
    }

    #[test]
    fn a_link_reads_short_kind_then_name() {
        let web = pod("web-0");
        assert_eq!(named_object_text("web-0", &web, false), "pod:web-0");
        let service = kind_key(ResourceKind::Services, Some("lab-shop"), "api");
        assert_eq!(named_object_text("api", &service, false), "svc:api");
        let config = kind_key(ResourceKind::ConfigMaps, Some("lab-shop"), "app");
        assert_eq!(named_object_text("app", &config, false), "cm:app");
        let secret = kind_key(ResourceKind::Secrets, Some("lab-shop"), "tls");
        assert_eq!(named_object_text("tls", &secret, false), "secret:tls");
    }

    #[test]
    fn a_namespaced_link_reads_kind_then_namespace_slash_name_once() {
        let web = pod("web-0");
        assert_eq!(named_object_text("web-0", &web, true), "pod:lab-shop/web-0");
        // The namespace already in the text is not repeated.
        assert_eq!(
            named_object_text("lab-shop/web-0", &web, true),
            "pod:lab-shop/web-0"
        );
        assert_eq!(
            named_object_text("lab-shop/web-0", &web, false),
            "pod:web-0"
        );
    }

    #[test]
    fn a_kind_the_text_already_names_is_not_repeated() {
        let deployment = kind_key(ResourceKind::Deployments, Some("lab-shop-stg"), "web");
        assert_eq!(
            named_object_text("deployment/web", &deployment, true),
            "deploy:lab-shop-stg/web"
        );
        assert_eq!(
            named_object_text("lab-shop-stg/deployment/web", &deployment, true),
            "deploy:lab-shop-stg/web"
        );
        let replica_set = kind_key(ResourceKind::ReplicaSets, Some("kube-system"), "coredns-76");
        assert_eq!(
            named_object_text("replicaset/coredns-76", &replica_set, false),
            "rs:coredns-76"
        );
        // A path that only looks like a kind prefix stays whole.
        let ingress = kind_key(ResourceKind::Ingresses, Some("shop"), "web");
        assert_eq!(
            named_object_text("shop.example/api", &ingress, false),
            "ing:shop.example/api"
        );
    }

    #[test]
    fn a_cluster_scoped_link_has_no_namespace() {
        let node = ResourceKey::Node {
            name: "node-1".to_owned(),
        };
        let namespaces = kind_key(ResourceKind::Namespaces, None, "lab-shop");
        assert_eq!(named_object_text("node-1", &node, true), "node:node-1");
        assert_eq!(
            named_object_text("lab-shop", &namespaces, true),
            "ns:lab-shop"
        );
    }

    #[test]
    fn a_drawer_captions_the_kind_by_its_short_name() {
        for (kind_name, caption) in [
            ("Service", "SVC"),
            ("Deployment", "DEPLOY"),
            ("StatefulSet", "STS"),
            ("DaemonSet", "DS"),
            ("ReplicaSet", "RS"),
            ("CronJob", "CJ"),
            ("ConfigMap", "CM"),
            ("Ingress", "ING"),
            ("NetworkPolicy", "NETPOL"),
            ("PodDisruptionBudget", "PDB"),
            ("HorizontalPodAutoscaler", "HPA"),
            ("ResourceQuota", "QUOTA"),
            ("PersistentVolumeClaim", "PVC"),
            ("PersistentVolume", "PV"),
            ("StorageClass", "SC"),
            ("Namespace", "NS"),
            ("Event", "EV"),
            ("ServiceAccount", "SA"),
            ("RoleBinding", "RB"),
            ("ClusterRole", "CR"),
            ("ClusterRoleBinding", "CRB"),
            ("CustomResourceDefinition", "CRD"),
            // Kinds whose short name is not clearer keep the whole word.
            ("Pod", "POD"),
            ("Node", "NODE"),
            ("Job", "JOB"),
            ("Secret", "SECRET"),
            ("Role", "ROLE"),
            ("Helm release", "HELM RELEASE"),
            ("Widget", "WIDGET"),
        ] {
            assert_eq!(kind_label(kind_name), caption, "{kind_name}");
        }
    }

    #[test]
    fn created_text_shows_age_or_nothing() {
        let now = jiff::Timestamp::from_second(7_200).expect("valid timestamp");
        let created = jiff::Timestamp::from_second(0).expect("valid timestamp");
        assert_eq!(
            created_text(Some(created), now).as_deref(),
            Some("created 2h ago")
        );
        assert_eq!(created_text(None, now), None);
    }

    #[test]
    fn drawer_tabs_follow_the_wireframe_order() {
        let pod = ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: "api-0".to_owned(),
        };
        let node = ResourceKey::Node {
            name: "node-1".to_owned(),
        };
        let kind = |kind| ResourceKey::Kind {
            kind,
            namespace: Some("shop".to_owned()),
            name: "x".to_owned(),
        };
        let with_events = [DrawerTab::Overview, DrawerTab::Yaml, DrawerTab::Events];
        let with_monitor = [
            DrawerTab::Overview,
            DrawerTab::Monitor,
            DrawerTab::Yaml,
            DrawerTab::Events,
        ];
        assert_eq!(
            drawer_tabs(&pod),
            [
                DrawerTab::Overview,
                DrawerTab::Containers,
                DrawerTab::Monitor,
                DrawerTab::Yaml,
                DrawerTab::Events
            ]
        );
        assert_eq!(
            drawer_tabs(&node),
            [
                DrawerTab::Overview,
                DrawerTab::Pods,
                DrawerTab::Monitor,
                DrawerTab::Yaml,
                DrawerTab::Events
            ]
        );
        assert_eq!(drawer_tabs(&kind(ResourceKind::Deployments)), with_monitor);
        assert_eq!(drawer_tabs(&kind(ResourceKind::Jobs)), with_monitor);
        // ConfigMaps and CronJobs have no pods to monitor.
        assert_eq!(drawer_tabs(&kind(ResourceKind::ConfigMaps)), with_events);
        assert_eq!(drawer_tabs(&kind(ResourceKind::CronJobs)), with_events);
        assert_eq!(
            drawer_tabs(&kind(ResourceKind::Events)),
            [DrawerTab::Overview, DrawerTab::Yaml]
        );
    }

    #[test]
    fn helm_release_drawer_has_helm_tabs() {
        let key = ResourceKey::Kind {
            kind: ResourceKind::HelmReleases,
            namespace: Some("shop".to_owned()),
            name: "api".to_owned(),
        };
        assert_eq!(
            drawer_tabs(&key),
            [
                DrawerTab::Overview,
                DrawerTab::Values,
                DrawerTab::Manifest,
                DrawerTab::Notes
            ]
        );
    }

    #[test]
    fn drawer_tabs_omit_yaml_for_an_unaddressable_key() {
        // A cluster-scoped kind never has a namespace, so the cluster crate rejects it.
        let key = ResourceKey::Kind {
            kind: ResourceKind::Namespaces,
            namespace: Some("shop".to_owned()),
            name: "x".to_owned(),
        };
        assert_eq!(drawer_tabs(&key), [DrawerTab::Overview, DrawerTab::Events]);
    }

    #[test]
    fn shown_tab_falls_back_to_overview() {
        let tabs = [DrawerTab::Overview, DrawerTab::Events];
        assert_eq!(shown_tab(&tabs, DrawerTab::Events), DrawerTab::Events);
        assert_eq!(shown_tab(&tabs, DrawerTab::Containers), DrawerTab::Overview);
        assert_eq!(
            shown_tab(&[DrawerTab::Overview], DrawerTab::Events),
            DrawerTab::Overview
        );
    }

    #[test]
    fn tab_titles_count_containers_and_events() {
        let tabs = [
            DrawerTab::Overview,
            DrawerTab::Containers,
            DrawerTab::Yaml,
            DrawerTab::Events,
        ];
        let titles: Vec<String> = tab_titles(
            &tabs,
            TabCounts {
                containers: 3,
                pods: None,
            },
            None,
        )
        .into_iter()
        .map(|(_, title)| title.to_string())
        .collect();
        assert_eq!(titles, ["Overview", "Containers 3", "YAML", "Events"]);
    }

    #[test]
    fn tab_titles_count_pods_only_with_a_live_cluster() {
        let tabs = [DrawerTab::Overview, DrawerTab::Pods];
        let titles = |pods| -> Vec<String> {
            tab_titles(
                &tabs,
                TabCounts {
                    containers: 0,
                    pods,
                },
                None,
            )
            .into_iter()
            .map(|(_, title)| title.to_string())
            .collect()
        };
        assert_eq!(titles(Some(23)), ["Overview", "Pods 23"]);
        assert_eq!(titles(None), ["Overview", "Pods"]);
    }

    #[test]
    fn long_ranges_read_coarse_points() {
        assert_eq!(MonitorRange::Minutes15.resolution(), Resolution::Fine);
        assert_eq!(MonitorRange::Hour1.resolution(), Resolution::Fine);
        assert_eq!(MonitorRange::Hours6.resolution(), Resolution::Coarse);
        assert_eq!(MonitorRange::Hours24.resolution(), Resolution::Coarse);
        assert_eq!(
            MonitorRange::Hours24.duration(),
            Duration::from_secs(86_400)
        );
        let labels: Vec<_> = MonitorRange::SOURCE
            .iter()
            .map(|range| range.label())
            .collect();
        assert_eq!(labels, ["15m", "1h", "6h", "24h", "7d", "30d"]);
        assert_eq!(MonitorRange::SAMPLER.len(), 4);
        assert!(MonitorRange::Days7.is_long() && MonitorRange::Days30.is_long());
        assert!(!MonitorRange::Hours24.is_long());
        assert_eq!(
            MonitorRange::Days30.source_step(),
            Duration::from_secs(7_200)
        );
        assert_eq!(
            MonitorRange::Minutes15.source_step(),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn monitor_state_starts_on_the_short_range_without_a_cache() {
        let state = DrawerState::new();
        assert_eq!(state.monitor.range, MonitorRange::Minutes15);
        assert_eq!(state.monitor.scope, MonitorScope::Total);
        assert!(state.monitor.cache.is_none());
    }

    #[test]
    fn drawer_state_starts_on_info() {
        let state = DrawerState::new();
        assert_eq!(state.container_tab, ContainerTab::Info);
        assert_eq!(state.tab, DrawerTab::Overview);
    }

    #[test]
    fn drawer_state_sizes_from_the_workspace() {
        let state = DrawerState::new();
        state.set_workspace_width(px(1200.));
        assert_eq!(state.width(DrawerSize::Standard), px(600.));
        assert_eq!(state.width(DrawerSize::Wide), px(900.));
    }

    #[test]
    fn drawer_width_is_half_the_workspace_and_three_quarters_for_the_wide_one() {
        assert_eq!(drawer_width(DrawerSize::Standard, px(1000.)), px(500.));
        assert_eq!(drawer_width(DrawerSize::Wide, px(1000.)), px(750.));
        assert_eq!(drawer_width(DrawerSize::Standard, px(2000.)), px(1000.));
        assert_eq!(drawer_width(DrawerSize::Wide, px(2000.)), px(1500.));
    }

    #[test]
    fn only_a_pod_gets_the_wide_drawer() {
        let kind = ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("shop".to_owned()),
            name: "api".to_owned(),
        };
        let node = ResourceKey::Node {
            name: "node-1".to_owned(),
        };
        let pod = ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: "api-0".to_owned(),
        };
        assert_eq!(DrawerSize::of(&kind), DrawerSize::Standard);
        assert_eq!(DrawerSize::of(&node), DrawerSize::Standard);
        assert_eq!(DrawerSize::of(&pod), DrawerSize::Wide);
    }

    #[test]
    fn drawer_width_has_a_floor_in_a_narrow_workspace() {
        // 50% of 800 is 400: the floor wins; the wide share (600) is already above it.
        assert_eq!(drawer_width(DrawerSize::Standard, px(800.)), px(480.));
        assert_eq!(drawer_width(DrawerSize::Wide, px(800.)), px(600.));
        assert_eq!(drawer_width(DrawerSize::Wide, px(600.)), px(480.));
    }

    #[test]
    fn drawer_width_never_exceeds_a_small_workspace() {
        for size in [DrawerSize::Standard, DrawerSize::Wide] {
            assert_eq!(drawer_width(size, px(400.)), px(400.));
            assert_eq!(drawer_width(size, px(0.)), px(0.));
            assert_eq!(drawer_width(size, px(-50.)), px(0.));
        }
    }

    #[test]
    fn a_page_key_moves_the_drawer_by_nine_tenths_of_its_view_and_stops_at_the_ends() {
        let (viewport, max) = (500., 1200.);
        let down = scrolled_offset(0., viewport, max, DrawerScroll::PageDown);
        assert_eq!(down, -450.);
        assert_eq!(
            scrolled_offset(down, viewport, max, DrawerScroll::PageUp),
            0.
        );
        assert_eq!(
            scrolled_offset(-1100., viewport, max, DrawerScroll::PageDown),
            -1200.
        );
        assert_eq!(scrolled_offset(0., viewport, max, DrawerScroll::PageUp), 0.);
        assert_eq!(scrolled_offset(-300., viewport, max, DrawerScroll::Top), 0.);
        assert_eq!(
            scrolled_offset(-300., viewport, max, DrawerScroll::Bottom),
            -1200.
        );
    }

    #[test]
    fn a_drawer_that_fits_its_view_does_not_scroll() {
        for step in [
            DrawerScroll::PageDown,
            DrawerScroll::Bottom,
            DrawerScroll::PageUp,
        ] {
            assert_eq!(scrolled_offset(0., 500., 0., step), 0.);
        }
    }
}
