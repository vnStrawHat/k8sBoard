//! The selection bar that appears while rows are ticked: how many, the screen's bulk actions
//! (gated like the menu items of the same actions), and a button that clears the selection.

use std::time::Duration;

use cluster::ObjectKind;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, Div, IntoElement, ParentElement as _, Pixels, SharedString, Styled as _, Task,
    WeakEntity, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::resource_actions::{PREVIEW_HINT, ResourceAction, with_next_step};
use crate::resource_kind::{KindAction, ResourceKind};

/// Why Roll back has no bulk form: each Deployment needs its own revision choice.
pub(crate) const ROLL_BACK_BULK_REASON: &str = "Roll back one deployment at a time";

const NODE_ACTIONS: [KindAction; 4] = [
    KindAction::keyed("Cordon", ResourceAction::Cordon),
    KindAction::keyed("Uncordon", ResourceAction::Uncordon),
    KindAction::keyed("Drain…", ResourceAction::Drain),
    KindAction::keyed("Edit labels…", ResourceAction::EditLabels),
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
    /// Runs this action in its read-only preview: the gate said no, and the tooltip gives the
    /// reason (Drain opens the plan without its confirm buttons).
    Preview(ResourceAction, SharedString),
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

/// The space the floating selection bar takes at the bottom of the table (its height and the
/// margin under it) while `checked` rows are ticked: the table body and the value popover keep
/// clear of it.
pub(crate) fn selection_bar_clearance(checked: usize) -> Pixels {
    if checked > 0 { px(64.) } else { px(0.) }
}

/// How long the notice of unticked rows stays.
pub(crate) const UNTICKED_NOTICE_LIFETIME: Duration = Duration::from_secs(8);

/// Ticked rows a filter change unticked, shown over the table until the timer ends. Dropping it
/// cancels the timer.
pub(crate) struct UntickedNotice {
    pub(crate) count: usize,
    pub(crate) _expiry: Task<()>,
}

/// `3 ticked rows hidden by the filter were unticked`.
pub(crate) fn unticked_notice_text(count: usize) -> String {
    if count == 1 {
        "1 ticked row hidden by the filter was unticked".to_owned()
    } else {
        format!("{count} ticked rows hidden by the filter were unticked")
    }
}

/// The floating pill of the notice, in the colours of the selection bar.
pub(crate) fn unticked_notice(count: usize, shell: &WeakEntity<AppShell>, cx: &App) -> AnyElement {
    let shell = shell.clone();
    floating_pill(cx)
        .child(unticked_notice_text(count))
        .child(
            Button::new("dismiss-unticked-notice")
                .small()
                .ghost()
                .icon(Icon::new(IconName::X))
                .tooltip("Dismiss")
                .on_click(move |_, _, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.dismiss_unticked_notice(cx));
                }),
        )
        .into_any_element()
}

/// The frame shared by the selection bar and the notice. They float over the table, so they use
/// the popover colours.
fn floating_pill(cx: &App) -> Div {
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
}

/// `text` is the count, such as `2 nodes selected`.
pub(crate) fn selection_bar(
    text: String,
    buttons: Vec<BulkButton>,
    shell: &WeakEntity<AppShell>,
    cx: &App,
) -> AnyElement {
    let clear_shell = shell.clone();
    floating_pill(cx)
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
                BulkState::Preview(action, reason) => {
                    let shell = shell.clone();
                    base.tooltip(format!("{PREVIEW_HINT} · {}", with_next_step(&reason)))
                        .on_click(move |_, window, cx| {
                            let _ =
                                shell.update(cx, |shell, cx| shell.run_bulk(action, window, cx));
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
