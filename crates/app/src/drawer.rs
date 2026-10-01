use std::rc::Rc;
use std::time::Duration;

use cluster::EventSummary;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::cluster_session::LiveList;
use crate::object_events::events_title;
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;
use crate::yaml_view::{YamlView, object_ref};

/// How long a drawer subject must rest before its background fetch starts (the object events
/// watch, the first YAML GET): arrowing through rows must not send one request per row.
pub(crate) const DRAWER_SUBJECT_DELAY: Duration = Duration::from_millis(250);
pub(crate) const DRAWER_WIDTH: Pixels = px(420.);
pub(crate) const DRAWER_EXPANDED_WIDTH: Pixels = px(640.);
const LABEL_WIDTH: Pixels = px(104.);
/// The kind drawers have longer labels, such as "Concurrency policy". Anything longer still
/// truncates with a tooltip, or uses `DetailRow::Stacked`.
pub(crate) const WIDE_LABEL_WIDTH: Pixels = px(136.);

/// The drawer is open exactly while a row is selected, so this holds only what the user
/// changes inside an open drawer. The tab and the expanded flag survive a change of
/// subject on the same screen; `show_screen` resets the tab to Overview. The selected container
/// does not survive a change of subject.
pub(crate) struct DrawerState {
    pub(crate) tab: DrawerTab,
    pub(crate) is_expanded: bool,
    pub(crate) selected_container: Option<String>,
    /// The sub-tab of the container detail. Like `tab`, it survives a change of container and
    /// subject; `show_screen` resets it.
    pub(crate) container_tab: ContainerTab,
    /// The YAML tab's view; `AppShell::sync_yaml_view` keeps it for the shown subject only.
    pub(crate) yaml: Option<Entity<YamlView>>,
}

impl DrawerState {
    pub(crate) fn new() -> Self {
        Self {
            tab: DrawerTab::Overview,
            is_expanded: false,
            selected_container: None,
            container_tab: ContainerTab::Info,
            yaml: None,
        }
    }

    pub(crate) fn width(&self) -> Pixels {
        if self.is_expanded {
            DRAWER_EXPANDED_WIDTH
        } else {
            DRAWER_WIDTH
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerTab {
    Info,
    Env,
    Mounts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawerTab {
    Overview,
    Containers,
    Yaml,
    Events,
}

/// The tabs a drawer shows, in wireframe order. An event's own drawer has no events of its own,
/// and a key the cluster crate cannot address has no YAML tab.
pub(crate) fn drawer_tabs(key: &ResourceKey) -> &'static [DrawerTab] {
    let has_yaml = object_ref(key).is_some();
    match key {
        ResourceKey::Pod { .. } if has_yaml => &[
            DrawerTab::Overview,
            DrawerTab::Containers,
            DrawerTab::Yaml,
            DrawerTab::Events,
        ],
        ResourceKey::Pod { .. } => &[
            DrawerTab::Overview,
            DrawerTab::Containers,
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
        ResourceKey::Node { .. } | ResourceKey::Kind { .. } if has_yaml => {
            &[DrawerTab::Overview, DrawerTab::Yaml, DrawerTab::Events]
        }
        ResourceKey::Node { .. } | ResourceKey::Kind { .. } => {
            &[DrawerTab::Overview, DrawerTab::Events]
        }
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

/// The label of each tab. `containers` is the pod's container count, which only a pod drawer
/// has a Containers tab for.
pub(crate) fn tab_titles(
    tabs: &[DrawerTab],
    containers: usize,
    events: Option<&LiveList<EventSummary>>,
) -> Vec<(DrawerTab, SharedString)> {
    tabs.iter()
        .map(|&tab| {
            let title = match tab {
                DrawerTab::Overview => "Overview".to_owned(),
                DrawerTab::Containers => format!("Containers {containers}"),
                DrawerTab::Yaml => "YAML".to_owned(),
                DrawerTab::Events => events_title(events),
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

/// The ⤢/⤡ button of every drawer.
pub(crate) fn expand_toggle(state: &DrawerState, cx: &Context<AppShell>) -> ExpandToggle {
    ExpandToggle {
        is_expanded: state.is_expanded,
        on_click: Rc::new(cx.listener(|shell, _, _, cx| shell.toggle_drawer_expanded(cx))),
    }
}

pub(crate) type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(crate) struct ExpandToggle {
    pub(crate) is_expanded: bool,
    pub(crate) on_click: ClickHandler,
}

pub(crate) struct DrawerHeader {
    /// Two letters naming the kind, "Po" or "No".
    pub(crate) kind_badge: &'static str,
    pub(crate) name: SharedString,
    pub(crate) subtitle: AnyElement,
    /// The ⋯ button with its dropdown menu.
    pub(crate) menu: AnyElement,
    pub(crate) expand: ExpandToggle,
    pub(crate) on_close: ClickHandler,
}

/// What fills a drawer below its tab bar.
pub(crate) enum DrawerBody {
    /// Padded, and scrolled by the frame.
    Scrolling(AnyElement),
    /// Fills the rest with no padding; the content (the code editor) scrolls itself.
    Filling(AnyElement),
}

/// The shared frame: header, subtitle, optional tab bar, and the body. It is a
/// plain element laid over the workspace (the caller's container is `.relative()`), not
/// the kit `Sheet`: that one covers the sidebar and takes focus from the table.
pub(crate) fn drawer_frame(
    header: DrawerHeader,
    tabs: Option<AnyElement>,
    body: DrawerBody,
    width: Pixels,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(width)
        .bg(theme.background)
        .border_l_1()
        .border_color(theme.border)
        .shadow_lg()
        // Clicks must not fall through to the table underneath.
        .occlude()
        .child(header_row(header, cx))
        .when_some(tabs, |this, tabs| this.child(tabs))
        .child(match body {
            DrawerBody::Scrolling(body) => div()
                .id("drawer-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .child(body)
                .into_any_element(),
            DrawerBody::Filling(body) => div()
                .id("drawer-body")
                .flex_1()
                .min_h_0()
                .child(body)
                .into_any_element(),
        })
}

fn header_row(header: DrawerHeader, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let on_close = header.on_close;
    let on_expand = header.expand.on_click;
    let expand_icon = if header.expand.is_expanded {
        IconName::Minimize2
    } else {
        IconName::Maximize2
    };
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
                .child(
                    div()
                        .px_1p5()
                        .rounded(theme.radius)
                        .bg(theme.muted)
                        .text_color(theme.muted_foreground)
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .child(header.kind_badge),
                )
                .child(
                    truncated_text("drawer-title", header.name)
                        .flex_1()
                        .min_w_0()
                        .font_semibold()
                        .font_family(theme.mono_font_family.clone()),
                )
                .child(header.menu)
                .child(
                    Button::new("drawer-expand")
                        .ghost()
                        .small()
                        .icon(Icon::new(expand_icon))
                        .on_click(move |event, window, cx| on_expand(event, window, cx)),
                )
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

/// The ⋯ button; the caller attaches the dropdown menu to it.
pub(crate) fn menu_button() -> Button {
    Button::new("drawer-menu")
        .ghost()
        .small()
        .icon(Icon::new(IconName::Ellipsis))
}

/// A muted section heading inside a drawer body.
pub(crate) fn section_title(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .mt_4()
        .mb_2()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// A label with its value, in the two-column layout shared by both drawers.
pub(crate) fn detail_row(
    label: impl Into<SharedString>,
    value: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    labeled_row(LABEL_WIDTH, label.into(), value, cx)
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
    let tooltip_text = text.clone();
    div()
        .id(id)
        .truncate()
        .child(text)
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
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

/// A mono value that opens `target` on its own screen.
pub(crate) fn link_text(
    id: usize,
    text: &SharedString,
    target: ResourceKey,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let tooltip_text = SharedString::from(format!("Open {text}"));
    div()
        .id(("link", id))
        .truncate()
        .cursor_pointer()
        .font_family(theme.mono_font_family.clone())
        .text_color(theme.link)
        .underline()
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .on_click(cx.listener(move |shell, _, _, cx| shell.reveal(target.clone(), cx)))
        .child(text.clone())
        .into_any_element()
}

/// Wrapping chips, or a dash when there are none.
pub(crate) fn chips(terms: &[SharedString], cx: &App) -> AnyElement {
    if terms.is_empty() {
        return absent_text(cx).into_any_element();
    }
    let theme = cx.theme();
    h_flex()
        .flex_wrap()
        .gap_1()
        .children(terms.iter().map(|term| {
            div()
                .max_w_full()
                .truncate()
                .px_1p5()
                .rounded(theme.radius)
                .bg(theme.muted)
                .font_family(theme.mono_font_family.clone())
                .text_xs()
                .child(term.clone())
        }))
        .into_any_element()
}

/// A port with its Forward button. The button is always disabled: port-forwarding is not
/// available in this version, and the tooltip says why.
pub(crate) fn port_row(
    text: &SharedString,
    id: usize,
    reason: &SharedString,
    cx: &App,
) -> AnyElement {
    h_flex()
        .gap_2()
        .py_1()
        .items_center()
        .text_sm()
        .child(
            truncated_text(("port", id), text.clone())
                .flex_1()
                .min_w_0()
                .font_family(cx.theme().mono_font_family.clone()),
        )
        .child(
            Button::new(("forward", id))
                .label("Forward")
                .icon(Icon::new(IconName::ArrowLeftRight))
                .xsmall()
                .ghost()
                .disabled(true)
                .tooltip(reason.clone()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            drawer_tabs(&pod),
            [
                DrawerTab::Overview,
                DrawerTab::Containers,
                DrawerTab::Yaml,
                DrawerTab::Events
            ]
        );
        assert_eq!(drawer_tabs(&node), with_events);
        assert_eq!(drawer_tabs(&kind(ResourceKind::Deployments)), with_events);
        assert_eq!(
            drawer_tabs(&kind(ResourceKind::Events)),
            [DrawerTab::Overview, DrawerTab::Yaml]
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
        let titles: Vec<String> = tab_titles(&tabs, 3, None)
            .into_iter()
            .map(|(_, title)| title.to_string())
            .collect();
        assert_eq!(titles, ["Overview", "Containers 3", "YAML", "Events"]);
    }

    #[test]
    fn drawer_state_starts_on_info() {
        let state = DrawerState::new();
        assert_eq!(state.container_tab, ContainerTab::Info);
        assert_eq!(state.tab, DrawerTab::Overview);
    }

    #[test]
    fn drawer_is_wider_when_expanded() {
        let mut state = DrawerState::new();
        assert_eq!(state.width(), DRAWER_WIDTH);
        state.is_expanded = true;
        assert_eq!(state.width(), DRAWER_EXPANDED_WIDTH);
    }
}
