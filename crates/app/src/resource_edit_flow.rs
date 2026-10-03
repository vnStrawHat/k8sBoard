//! The shell side of the resource edits (spec 0032b): the popovers of the HPA range and the PVC size,
//! Set as default storage class, and the guarded starts they hand their values to. Every step reads the row and the guard from the
//! subject's own cluster slot, never from the primary, and ends in `start_write` or `start_batch`.
//!
//! A child of `app_shell`, like `write_flow`: it reads the viewed slots.

use cluster::{HorizontalPodAutoscalerSummary, PersistentVolumeClaimSummary};
use gpui_kit::{App, AppContext as _, Context, SharedString, Window};

use super::AppShell;
use super::batch_write::BulkValue;
use super::write_flow::notify;
use crate::kind_row::KindObject;
use crate::resource_actions::{ResourceAction, action_label, unavailable_text};
use crate::resource_edits::{
    StorageInput, claim_block, claim_floor, class_block, default_class_intent, expand_intent,
    hpa_range_intent, storage_input,
};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::value_popover::ValuePopover;
use crate::workload_actions::WorkloadScope;

/// Whether a start checks the state of its row before it plans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowCheck {
    /// A row the state refuses (`row_block`) says why and starts nothing.
    Enforced,
    /// The Retry of a partial run: the row is in the state the first run left it in.
    Bypassed,
}

impl AppShell {
    /// The HPA under `subject`, from the subject's own cluster as it is now.
    pub(crate) fn hpa_of(
        &self,
        subject: &ClusterObject,
        cx: &App,
    ) -> Option<HorizontalPodAutoscalerSummary> {
        let live = self.slot_live(&subject.cluster, cx)?;
        match &live.row_of(&subject.key)?.object {
            KindObject::HorizontalPodAutoscaler(hpa) => Some(hpa.clone()),
            _ => None,
        }
    }

    /// Edit min / max on the cursor row: the one popover that the menu, the key, and the palette
    /// share.
    pub(crate) fn open_hpa_range_popover(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hpa) = self.hpa_of(subject, cx) else {
            let text = unavailable_text(
                action_label(ResourceAction::EditHpaRange),
                "the object is no longer listed",
            );
            notify(window, cx, text);
            return;
        };
        let (shell, subject) = (cx.weak_entity(), subject.clone());
        let popover = cx.new(|cx| ValuePopover::hpa_range_one(shell, subject, hpa, window, cx));
        self.set_value_popover(popover, cx);
    }

    /// The Edit limits popover for the ticked HPAs, empty: one range for all of them.
    pub(crate) fn open_bulk_hpa_range_popover(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.kind_table.read(cx).delegate().checked_rows(cx).len();
        let shell = cx.weak_entity();
        let popover = cx.new(|cx| ValuePopover::hpa_range_ticked(shell, count, window, cx));
        self.set_value_popover(popover, cx);
    }

    /// Set limits of the popover: the popover closes, and the change goes to the confirm dialog.
    pub(crate) fn submit_hpa_range(
        &mut self,
        subject: &ClusterObject,
        min: u32,
        max: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_value_popover(cx);
        self.start_hpa_range(subject, min, max, window, cx);
    }

    /// The intent of the range for `subject`, on the row's own cluster. The row is read again so the
    /// label and the warnings name what is there now, not what the form was opened on.
    fn start_hpa_range(
        &mut self,
        subject: &ClusterObject,
        min: u32,
        max: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::EditHpaRange);
        let Some(hpa) = self.hpa_of(subject, cx) else {
            let text = unavailable_text(label, "the object is no longer listed");
            notify(window, cx, text);
            return;
        };
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
            hpa_range_intent(&scope, &hpa, min, max)
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => notify(
                window,
                cx,
                unavailable_text(label, "the object name or the range is not valid"),
            ),
        }
    }

    /// Set limits of the bulk popover: the popover closes, and the batch goes to the list dialog.
    pub(crate) fn submit_bulk_hpa_range(
        &mut self,
        min: u32,
        max: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_value_popover(cx);
        let action = ResourceAction::EditHpaRange;
        match self.bulk_batch(action, BulkValue::Range { min, max }, cx) {
            Ok(intent) => self.start_batch(intent, window, cx),
            Err(reason) => notify(window, cx, unavailable_text(action_label(action), &reason)),
        }
    }
    /// The claim under `subject`, from the subject's own cluster as it is now.
    pub(crate) fn claim_of(
        &self,
        subject: &ClusterObject,
        cx: &App,
    ) -> Option<PersistentVolumeClaimSummary> {
        let live = self.slot_live(&subject.cluster, cx)?;
        match &live.row_of(&subject.key)?.object {
            KindObject::PersistentVolumeClaim(claim) => Some(claim.clone()),
            _ => None,
        }
    }

    /// Expand on the cursor row: the one popover that the menu, the key, and the palette share. A
    /// claim the state refuses says why instead.
    pub(crate) fn open_expand_popover(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::ExpandClaim);
        let Some(claim) = self.claim_of(subject, cx) else {
            let text = unavailable_text(label, "the object is no longer listed");
            notify(window, cx, text);
            return;
        };
        if let Some(reason) = self.claim_refusal(subject, &claim, cx) {
            notify(window, cx, unavailable_text(label, &reason));
            return;
        }
        let (shell, subject) = (cx.weak_entity(), subject.clone());
        let popover = cx.new(|cx| ValuePopover::expand_one(shell, subject, claim, window, cx));
        self.set_value_popover(popover, cx);
    }

    /// Why `claim` cannot be expanded now: its state, and its class when the StorageClasses list is
    /// loaded in the claim's cluster.
    fn claim_refusal(
        &self,
        subject: &ClusterObject,
        claim: &PersistentVolumeClaimSummary,
        cx: &App,
    ) -> Option<SharedString> {
        let live = self.slot_live(&subject.cluster, cx)?;
        claim_block(claim, &live.loaded_storage_classes())
    }

    /// The Expand popover for the ticked claims, empty: one size for all of them.
    pub(crate) fn open_bulk_expand_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.kind_table.read(cx).delegate().checked_rows(cx).len();
        let shell = cx.weak_entity();
        let popover = cx.new(|cx| ValuePopover::expand_ticked(shell, count, window, cx));
        self.set_value_popover(popover, cx);
    }

    /// Expand of the popover: the popover closes, and the change goes to the confirm dialog.
    pub(crate) fn submit_expand(
        &mut self,
        subject: &ClusterObject,
        storage: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_value_popover(cx);
        self.start_expand(subject, storage, window, cx);
    }

    /// The intent of growing `subject` to `storage`, on the row's own cluster. The row is read again
    /// so the label, the floor, and the warnings name what is there now, and a claim that is no
    /// longer bound, or whose size already passed `storage`, is refused here.
    fn start_expand(
        &mut self,
        subject: &ClusterObject,
        storage: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::ExpandClaim);
        let Some(claim) = self.claim_of(subject, cx) else {
            let text = unavailable_text(label, "the object is no longer listed");
            notify(window, cx, text);
            return;
        };
        if let Some(reason) = self.claim_refusal(subject, &claim, cx) {
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
            expand_intent(&scope, &claim, storage)
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => {
                let reason = match storage_input(storage, claim_floor(&claim)) {
                    StorageInput::Refused(reason) => reason,
                    StorageInput::Incomplete | StorageInput::Set(_) => {
                        "the object name is not valid".to_owned()
                    }
                };
                notify(window, cx, unavailable_text(label, &reason));
            }
        }
    }

    /// Expand of the bulk popover: the popover closes, and the batch goes to the list dialog.
    pub(crate) fn submit_bulk_expand(
        &mut self,
        storage: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_value_popover(cx);
        let action = ResourceAction::ExpandClaim;
        match self.bulk_batch(action, BulkValue::Storage(storage), cx) {
            Ok(intent) => self.start_batch(intent, window, cx),
            Err(reason) => notify(window, cx, unavailable_text(action_label(action), &reason)),
        }
    }
    /// Set as default on the class under `subject`: the menu item, the key, the palette, the
    /// selection bar, and the Retry share it. The plan comes from the StorageClasses list on screen,
    /// read from the subject's own cluster now, so the dialog names the classes as they are.
    pub(crate) fn start_set_default(
        &mut self,
        subject: &ClusterObject,
        check: RowCheck,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::SetDefaultStorageClass);
        let ResourceKey::Kind { name, .. } = &subject.key else {
            return;
        };
        let intent = {
            let (Some(guard), Some(live)) = (
                self.guard_for(&subject.cluster, cx),
                self.slot_live(&subject.cluster, cx),
            ) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            let classes = live.loaded_storage_classes();
            let Some(target) = classes.iter().copied().find(|class| class.name == *name) else {
                let text = unavailable_text(label, "the object is no longer listed");
                notify(window, cx, text);
                return;
            };
            // A Retry bypasses this: the target is the default by then, and the unsets remain.
            if check == RowCheck::Enforced
                && let Some(reason) = class_block(target)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let scope = WorkloadScope {
                cluster: &subject.cluster,
                cluster_name: guard.display_name(),
            };
            default_class_intent(&scope, target, &classes)
        };
        match intent {
            Some(intent) => self.start_batch(intent, window, cx),
            None => notify(
                window,
                cx,
                unavailable_text(label, "there is nothing to change"),
            ),
        }
    }

    /// The Retry of a stopped batch: the same action on the same object, planned again from the
    /// data of now. The gate still applies; the state of the row does not.
    pub(crate) fn retry_batch(
        &mut self,
        action: ResourceAction,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action == ResourceAction::SetDefaultStorageClass {
            self.start_set_default(subject, RowCheck::Bypassed, window, cx);
        }
    }
}

/// The fixed pictures of the `--screen` fixtures of this spec (screenshot builds only): fixed
/// objects of a fixed cluster, no connection call, and a confirm button that can never send.
#[cfg(feature = "screenshot")]
mod fixtures {
    use cluster::{ControllerRef, HorizontalPodAutoscalerSummary, StorageClassSummary};

    use std::rc::Rc;

    use super::*;
    use crate::cluster_registry::ClusterRef;
    use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
    use crate::environment::Environment;
    use crate::resource_kind::ResourceKind;
    use crate::write_guard::{ActionRisk, ConfirmMode, confirm_step};

    fn fixture_cluster(context: &str) -> ClusterRef {
        ClusterRef {
            kubeconfig: std::path::PathBuf::from("fixture.yaml"),
            context: context.to_owned(),
        }
    }

    /// 3 to 20 replicas, 9 now.
    fn fixture_hpa() -> HorizontalPodAutoscalerSummary {
        HorizontalPodAutoscalerSummary {
            namespace: "web".to_owned(),
            name: "frontend-hpa".to_owned(),
            created_at: None,
            labels: Vec::new(),
            target: ControllerRef {
                kind: "Deployment".to_owned(),
                name: "frontend".to_owned(),
            },
            min_replicas: 3,
            max_replicas: 20,
            current_replicas: 9,
            desired_replicas: 9,
            metrics: Vec::new(),
            conditions: Vec::new(),
            last_scaled_at: None,
        }
    }

    impl AppShell {
        /// `--screen hpa-range-popover`: the Edit min / max popover of a fixed HPA, with the max cut
        /// to 5 so the scale-down warning shows. It needs no cluster and can never send.
        pub(in crate::app_shell) fn open_hpa_range_fixture(
            &mut self,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let hpa = fixture_hpa();
            let key = ResourceKey::Kind {
                kind: ResourceKind::HorizontalPodAutoscalers,
                namespace: Some(hpa.namespace.clone()),
                name: hpa.name.clone(),
            };
            let subject = ClusterObject::new(fixture_cluster("stg-eu-1"), key);
            let shell = cx.weak_entity();
            let popover = cx.new(|cx| {
                let mut popover = ValuePopover::hpa_range_one(shell, subject, hpa, window, cx);
                popover.type_range("3", "5", window, cx);
                popover
            });
            self.set_value_popover(popover, cx);
        }
    }

    /// A bound claim of 100Gi on a class that expands.
    fn fixture_claim() -> PersistentVolumeClaimSummary {
        PersistentVolumeClaimSummary {
            namespace: "kafka".to_owned(),
            name: "data-kafka-0".to_owned(),
            created_at: None,
            labels: Vec::new(),
            phase: "Bound".to_owned(),
            is_terminating: false,
            volume: Some("pvc-0a1b2c".to_owned()),
            capacity: Some("100Gi".to_owned()),
            requested: Some("100Gi".to_owned()),
            access_modes: vec!["ReadWriteOnce".to_owned()],
            storage_class: Some("gp3".to_owned()),
            volume_mode: Some("Filesystem".to_owned()),
            conditions: Vec::new(),
        }
    }

    impl AppShell {
        /// Opens `kind` as a dialog of a fixed cluster of `environment`: the dry-run has passed, the
        /// buttons are dead, and no guard or connection is read. Its tier follows the environment.
        pub(in crate::app_shell) fn open_fixed_dialog(
            &mut self,
            kind: DialogKind,
            environment: Environment,
            (risk, expected): (ActionRisk, &str),
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let confirm = confirm_step(ConfirmMode::for_environment(environment), risk, expected);
            let inputs = DialogInputs {
                shell: cx.weak_entity(),
                kind,
                confirm,
                environment,
                generation: 0,
            };
            let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
            dialog.update(cx, |dialog, _| dialog.show_fixture());
            ConfirmDialog::open(&dialog, window, cx);
        }

        /// `--screen expand-confirm`: the Expand dialog of a fixed claim (100Gi to 150Gi) of a
        /// fixed Production cluster, so it asks for the cluster name. It needs no cluster and can
        /// never send.
        pub(in crate::app_shell) fn open_expand_fixture(
            &mut self,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let cluster = fixture_cluster("prod-eu-1");
            let scope = WorkloadScope {
                cluster: &cluster,
                cluster_name: "prod-eu-1",
            };
            let Some(intent) = expand_intent(&scope, &fixture_claim(), "150Gi") else {
                return;
            };
            let (risk, expected) = (intent.risk, intent.expected().to_owned());
            self.open_fixed_dialog(
                DialogKind::Write(Rc::new(intent)),
                Environment::Production,
                (risk, &expected),
                window,
                cx,
            );
        }
    }

    /// A storage class of the fixed cluster; the picture shows the plan, so only the default flag
    /// matters.
    fn fixture_class(name: &str, is_default: bool) -> StorageClassSummary {
        StorageClassSummary {
            name: name.to_owned(),
            created_at: None,
            labels: Vec::new(),
            provisioner: "ebs.csi.aws.com".to_owned(),
            reclaim_policy: "Delete".to_owned(),
            binding_mode: "WaitForFirstConsumer".to_owned(),
            allows_expansion: true,
            is_default,
            parameters: Vec::new(),
            mount_options: Vec::new(),
        }
    }

    impl AppShell {
        /// `--screen default-class-confirm`: the Set default dialog of a fixed cluster of Staging
        /// that makes `gp3` the default and unsets `io2`. It needs no cluster and can never send.
        pub(in crate::app_shell) fn open_default_class_fixture(
            &mut self,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let cluster = fixture_cluster("stg-eu-1");
            let scope = WorkloadScope {
                cluster: &cluster,
                cluster_name: "stg-eu-1",
            };
            let (gp3, io2) = (fixture_class("gp3", false), fixture_class("io2", true));
            let Some(batch) = default_class_intent(&scope, &gp3, &[&gp3, &io2]) else {
                return;
            };
            let (risk, expected) = (batch.risk, batch.expected().to_owned());
            self.open_fixed_dialog(
                DialogKind::Batch(Rc::new(batch)),
                Environment::Staging,
                (risk, &expected),
                window,
                cx,
            );
        }
    }
}
