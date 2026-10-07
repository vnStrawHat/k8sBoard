//! The shell side of the volume edits (spec 0032b, UX round 3): recreating a Pending claim with
//! another class, and setting the reclaim policy of a volume. Every step reads the row and the
//! guard from the subject's own cluster slot, never from the primary, and ends in `start_write`.
//!
//! A child of `app_shell`, like `resource_edit_flow`: it reads the viewed slots.

use cluster::{ObjectKind, ObjectRef, PersistentVolumeSummary, StorageClassSummary};
use gpui_kit::{App, AppContext as _, Context, SharedString, Window};

use super::AppShell;
use super::write_flow::notify;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::{CompanionLists, LiveList};
use crate::kind_row::KindObject;
use crate::live_sections::claim_pods;
use crate::resource_actions::{ResourceAction, action_label, unavailable_text};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::value_popover::ValuePopover;
use crate::volume_edits::{
    class_choices, policy_choices, reclaim_block, reclaim_policy_intent, recreate_block,
    recreate_intent,
};
use crate::workload_actions::WorkloadScope;

impl AppShell {
    /// The volume under `subject`, from the subject's own cluster as it is now.
    pub(crate) fn volume_of(
        &self,
        subject: &ClusterObject,
        cx: &App,
    ) -> Option<PersistentVolumeSummary> {
        let live = self.live_of(&subject.cluster, cx)?;
        match &live.row_of(&subject.key)?.object {
            KindObject::PersistentVolume(volume) => Some(volume.clone()),
            _ => None,
        }
    }

    /// The StorageClasses the PVCs screen watches beside its claims, empty while they load.
    fn companion_classes<'a>(
        &self,
        subject: &ClusterObject,
        cx: &'a App,
    ) -> Vec<&'a StorageClassSummary> {
        self.live_of(&subject.cluster, cx)
            .and_then(|live| live.companion())
            .and_then(CompanionLists::storage_classes)
            .and_then(LiveList::ready_items)
            .map(|classes| classes.iter().collect())
            .unwrap_or_default()
    }

    /// Recreate with class… on the cursor claim: the popover of the loaded classes, the default
    /// one picked. A claim the state refuses, or that a pod mounts, says why instead.
    pub(crate) fn open_recreate_popover(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::RecreateClaim);
        let Some(claim) = self.claim_of(subject, cx) else {
            let text = unavailable_text(label, "the object is no longer listed");
            notify(window, cx, text);
            return;
        };
        if let Some(reason) = self.recreate_refusal(subject, &claim, cx) {
            notify(window, cx, unavailable_text(label, &reason));
            return;
        }
        let classes = self.companion_classes(subject, cx);
        let Some(choices) = class_choices(&classes, claim.storage_class.as_deref()) else {
            let reason = "the storage classes are not loaded";
            notify(window, cx, unavailable_text(label, reason));
            return;
        };
        let (shell, subject) = (cx.weak_entity(), subject.clone());
        let popover = cx.new(|_| ValuePopover::recreate_one(shell, subject, claim, choices));
        self.set_value_popover(popover, cx);
    }

    /// Why `claim` cannot be recreated now: its state, or a pod that mounts it. The pods are read
    /// from the subject's own cluster; a list that has not loaded says so instead of guessing.
    fn recreate_refusal(
        &self,
        subject: &ClusterObject,
        claim: &cluster::PersistentVolumeClaimSummary,
        cx: &App,
    ) -> Option<SharedString> {
        if let Some(reason) = recreate_block(claim) {
            return Some(reason);
        }
        let live = self.live_of(&subject.cluster, cx)?;
        if live.pods.is_loading() {
            return Some("the pods are not loaded yet".into());
        }
        let pods = claim_pods(&claim.namespace, &claim.name, live.pods.items());
        let (first, _) = pods.first()?;
        let more = match pods.len() {
            1 => String::new(),
            others => format!(" and {} more", others - 1),
        };
        Some(format!("pod {}{more} mounts the claim", first.name).into())
    }

    /// Recreate of the popover: the popover closes, the claim's uid is read, and the change goes
    /// to the confirm dialog. Nothing is sent before the dialog.
    pub(crate) fn submit_recreate(
        &mut self,
        subject: &ClusterObject,
        class: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_value_popover(cx);
        let label = action_label(ResourceAction::RecreateClaim);
        let Some(claim) = self.claim_of(subject, cx) else {
            let text = unavailable_text(label, "the object is no longer listed");
            notify(window, cx, text);
            return;
        };
        if let Some(reason) = self.recreate_refusal(subject, &claim, cx) {
            notify(window, cx, unavailable_text(label, &reason));
            return;
        }
        let (Some(connection), Some(object)) = (
            self.connection_of(&subject.cluster, cx),
            ObjectRef::new(
                ObjectKind::PersistentVolumeClaim,
                Some(claim.namespace.clone()),
                claim.name.clone(),
            ),
        ) else {
            notify(
                window,
                cx,
                unavailable_text(label, "the cluster is not open"),
            );
            return;
        };
        // The delete is pinned to the uid read here, so a claim recreated meanwhile is never hit.
        let reading = cx
            .global::<ClusterRuntime>()
            .clone()
            .spawn(async move { connection.object_identity(&object).await });
        let (subject, class) = (subject.clone(), class.to_owned());
        cx.spawn_in(window, async move |shell, cx| {
            let identity = reading.await;
            let _ = shell.update_in(cx, |shell, window, cx| match identity {
                Ok(Ok(identity)) => {
                    shell.start_recreate(&subject, &identity.uid, &class, window, cx);
                }
                Ok(Err(error)) => {
                    let reason = format!("could not read claim {}: {error}", claim.name);
                    notify(window, cx, unavailable_text(label, &reason));
                }
                Err(_) => {}
            });
        })
        .detach();
    }

    /// The intent of recreating the claim under `subject` with `class`, read again so the label
    /// and the warnings name what is there now.
    fn start_recreate(
        &mut self,
        subject: &ClusterObject,
        uid: &str,
        class: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::RecreateClaim);
        let Some(claim) = self.claim_of(subject, cx) else {
            let text = unavailable_text(label, "the object is no longer listed");
            notify(window, cx, text);
            return;
        };
        if let Some(reason) = self.recreate_refusal(subject, &claim, cx) {
            notify(window, cx, unavailable_text(label, &reason));
            return;
        }
        let intent = {
            let Some(guard) = self.guard_for(&subject.cluster, cx) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            let scope = WorkloadScope {
                cluster: &subject.cluster,
                cluster_name: guard.display_name(),
            };
            recreate_intent(&scope, &claim, uid, class)
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => notify(
                window,
                cx,
                unavailable_text(label, "the claim name or the class is not valid"),
            ),
        }
    }

    /// Set reclaim policy… on the cursor volume: the popover with Retain and Delete, the other one
    /// picked.
    pub(crate) fn open_reclaim_popover(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::SetReclaimPolicy);
        let Some(volume) = self.volume_of(subject, cx) else {
            let text = unavailable_text(label, "the object is no longer listed");
            notify(window, cx, text);
            return;
        };
        if let Some(reason) = reclaim_block(&volume) {
            notify(window, cx, unavailable_text(label, &reason));
            return;
        }
        let choices = policy_choices(&volume);
        let (shell, subject) = (cx.weak_entity(), subject.clone());
        let popover = cx.new(|_| ValuePopover::reclaim_policy_one(shell, subject, volume, choices));
        self.set_value_popover(popover, cx);
    }

    /// Set policy of the popover: the popover closes, and the change goes to the confirm dialog.
    pub(crate) fn submit_reclaim_policy(
        &mut self,
        subject: &ClusterObject,
        policy: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_value_popover(cx);
        let label = action_label(ResourceAction::SetReclaimPolicy);
        let ResourceKey::Kind { .. } = &subject.key else {
            return;
        };
        let Some(volume) = self.volume_of(subject, cx) else {
            let text = unavailable_text(label, "the object is no longer listed");
            notify(window, cx, text);
            return;
        };
        if let Some(reason) = reclaim_block(&volume) {
            notify(window, cx, unavailable_text(label, &reason));
            return;
        }
        let intent = {
            let Some(guard) = self.guard_for(&subject.cluster, cx) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            let scope = WorkloadScope {
                cluster: &subject.cluster,
                cluster_name: guard.display_name(),
            };
            reclaim_policy_intent(&scope, &volume, policy)
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => notify(
                window,
                cx,
                unavailable_text(label, "the volume name or the policy is not valid"),
            ),
        }
    }
}
