//! The selection bar that appears while rows are ticked: how many, the screen's bulk actions
//! (gated like the menu items of the same actions), and a button that clears the selection.

use cluster::ObjectKind;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, SharedString, Styled as _, WeakEntity,
};

use crate::app_shell::{AppShell, Screen};
use crate::resource_actions::{ResourceAction, with_next_step};
use crate::resource_kind::{KindAction, ResourceKind};

/// Why Roll back has no bulk form: each Deployment needs its own revision choice.
pub(crate) const ROLL_BACK_BULK_REASON: &str = "Roll back one deployment at a time";

const NODE_ACTIONS: [KindAction; 3] = [
    KindAction::keyed("Cordon", ResourceAction::Cordon),
    KindAction::keyed("Uncordon", ResourceAction::Uncordon),
    KindAction::keyed("Drain…", ResourceAction::Drain),
];
const DEPLOYMENT_ACTIONS: [KindAction; 3] = [
    KindAction::keyed("Scale…", ResourceAction::Scale(ObjectKind::Deployment)),
    KindAction::keyed(
        "Restart rollout",
        ResourceAction::RestartRollout(ObjectKind::Deployment),
    ),
    KindAction::keyed("Roll back…", ResourceAction::RollBack),
];
const STATEFUL_SET_ACTIONS: [KindAction; 2] = [
    KindAction::keyed("Scale…", ResourceAction::Scale(ObjectKind::StatefulSet)),
    KindAction::keyed(
        "Restart rollout",
        ResourceAction::RestartRollout(ObjectKind::StatefulSet),
    ),
];
const DAEMON_SET_ACTIONS: [KindAction; 1] = [KindAction::keyed(
    "Restart rollout",
    ResourceAction::RestartRollout(ObjectKind::DaemonSet),
)];
const JOB_ACTIONS: [KindAction; 1] = [KindAction::keyed("Re-run job", ResourceAction::RerunJob)];
const HPA_ACTIONS: [KindAction; 1] = [KindAction::keyed(
    "Edit min / max",
    ResourceAction::EditHpaRange,
)];
const CLAIM_ACTIONS: [KindAction; 1] = [KindAction::keyed("Expand", ResourceAction::ExpandClaim)];
const CLASS_ACTIONS: [KindAction; 1] = [KindAction::keyed(
    "Set default",
    ResourceAction::SetDefaultStorageClass,
)];
const CRON_JOB_ACTIONS: [KindAction; 2] = [
    KindAction::keyed("Trigger now", ResourceAction::TriggerCronJob),
    KindAction::keyed("Suspend", ResourceAction::SuspendCronJob),
];

/// The actions that act on the ticked rows, as the wireframes draw them. The workload ones go
/// through `batch_write`; the Nodes ones are built by `node_editor` (spec 0034).
pub(crate) fn bulk_actions(screen: Screen) -> &'static [KindAction] {
    match screen {
        Screen::Nodes => &NODE_ACTIONS,
        Screen::Kind(ResourceKind::Deployments) => &DEPLOYMENT_ACTIONS,
        Screen::Kind(ResourceKind::StatefulSets) => &STATEFUL_SET_ACTIONS,
        Screen::Kind(ResourceKind::DaemonSets) => &DAEMON_SET_ACTIONS,
        Screen::Kind(ResourceKind::Jobs) => &JOB_ACTIONS,
        Screen::Kind(ResourceKind::CronJobs) => &CRON_JOB_ACTIONS,
        Screen::Kind(ResourceKind::HorizontalPodAutoscalers) => &HPA_ACTIONS,
        Screen::Kind(ResourceKind::PersistentVolumeClaims) => &CLAIM_ACTIONS,
        Screen::Kind(ResourceKind::StorageClasses) => &CLASS_ACTIONS,
        Screen::Overview
        | Screen::Pods
        | Screen::Issues
        | Screen::Topology
        | Screen::PortForwarding
        | Screen::Kind(_) => &[],
    }
}

/// Whether a bulk button works now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BulkState {
    /// Runs this action on the ticked rows.
    Ready(ResourceAction),
    /// Off, with the reason its tooltip gives.
    Off(SharedString),
}

/// One button of the selection bar, as the shell decided it for the ticked rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BulkButton {
    pub(crate) label: SharedString,
    pub(crate) state: BulkState,
    /// Drawn as the danger button: Delete (spec 0033).
    pub(crate) is_danger: bool,
}

/// `text` is the count, such as `2 nodes selected`. The bar floats over the table, so it uses
/// the popover colours.
pub(crate) fn selection_bar(
    text: String,
    buttons: Vec<BulkButton>,
    shell: &WeakEntity<AppShell>,
    cx: &App,
) -> AnyElement {
    let clear_shell = shell.clone();
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
        .children(buttons.into_iter().map(|button| {
            let id = button.label.clone();
            let base = Button::new(id).small().outline().label(button.label);
            let base = if button.is_danger {
                base.danger()
            } else {
                base
            };
            match button.state {
                BulkState::Ready(action) => {
                    let shell = shell.clone();
                    base.on_click(move |_, window, cx| {
                        let _ = shell.update(cx, |shell, cx| shell.run_bulk(action, window, cx));
                    })
                }
                BulkState::Off(reason) => base.disabled(true).tooltip(with_next_step(&reason)),
            }
        }))
        .child(
            Button::new("clear-selection")
                .small()
                .ghost()
                .icon(Icon::new(IconName::X))
                .tooltip("Clear selection")
                .on_click(move |_, _, cx| {
                    let _ = clear_shell.update(cx, |shell, cx| shell.clear_checked(cx));
                }),
        )
        .into_any_element()
}

#[cfg(test)]
#[path = "row_selection_tests.rs"]
mod row_selection_tests;
