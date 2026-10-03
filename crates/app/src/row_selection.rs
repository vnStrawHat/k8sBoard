//! The selection bar that appears while rows are ticked: how many, the screen's bulk actions
//! (shown disabled in this read-only version), and a button that clears the selection.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex};
use gpui_kit::{AnyElement, App, IntoElement, ParentElement as _, Styled as _, WeakEntity};

use crate::app_shell::{AppShell, Screen};
use crate::resource_actions::NOT_SHIPPED_REASON;
use crate::resource_kind::ResourceKind;

/// The actions that act on the ticked rows, as the wireframes draw them. Every one is disabled
/// until 0032 to 0034 enable them.
pub(crate) fn bulk_actions(screen: Screen) -> &'static [&'static str] {
    match screen {
        Screen::Nodes => &["Cordon", "Uncordon", "Drain…"],
        Screen::Kind(ResourceKind::Deployments) => &["Scale…", "Restart", "Roll back…"],
        Screen::Kind(ResourceKind::StatefulSets) => &["Scale…", "Restart"],
        Screen::Kind(ResourceKind::DaemonSets) => &["Restart"],
        Screen::Kind(ResourceKind::Jobs) => &["Re-run"],
        Screen::Kind(ResourceKind::CronJobs) => &["Trigger now", "Suspend"],
        Screen::Overview | Screen::Pods | Screen::Issues | Screen::Topology | Screen::Kind(_) => {
            &[]
        }
    }
}

/// `text` is the count, such as `2 nodes selected`. The bar floats over the table, so it uses
/// the popover colours.
pub(crate) fn selection_bar(
    text: String,
    actions: &'static [&'static str],
    shell: &WeakEntity<AppShell>,
    cx: &App,
) -> AnyElement {
    let shell = shell.clone();
    let theme = cx.theme();
    h_flex()
        .gap_2()
        .items_center()
        .px_3()
        .py_2()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .shadow_md()
        .child(text)
        .children(actions.iter().map(|label| {
            Button::new(*label)
                .small()
                .outline()
                .label(*label)
                .disabled(true)
                .tooltip(NOT_SHIPPED_REASON)
        }))
        .child(
            Button::new("clear-selection")
                .small()
                .ghost()
                .icon(Icon::new(IconName::X))
                .tooltip("Clear selection")
                .on_click(move |_, _, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.clear_checked(cx));
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_actions_follow_the_wireframe() {
        assert_eq!(
            bulk_actions(Screen::Nodes),
            ["Cordon", "Uncordon", "Drain…"]
        );
        assert_eq!(
            bulk_actions(Screen::Kind(ResourceKind::Deployments)),
            ["Scale…", "Restart", "Roll back…"]
        );
        assert_eq!(
            bulk_actions(Screen::Kind(ResourceKind::StatefulSets)),
            ["Scale…", "Restart"]
        );
        assert_eq!(
            bulk_actions(Screen::Kind(ResourceKind::DaemonSets)),
            ["Restart"]
        );
        assert_eq!(bulk_actions(Screen::Kind(ResourceKind::Jobs)), ["Re-run"]);
        assert_eq!(
            bulk_actions(Screen::Kind(ResourceKind::CronJobs)),
            ["Trigger now", "Suspend"]
        );
    }

    #[test]
    fn screens_without_bulk_actions_show_only_the_count() {
        assert!(bulk_actions(Screen::Pods).is_empty());
        for kind in [
            ResourceKind::Events,
            ResourceKind::Services,
            ResourceKind::ReplicaSets,
            ResourceKind::ConfigMaps,
        ] {
            assert!(bulk_actions(Screen::Kind(kind)).is_empty(), "{kind:?}");
        }
    }
}
