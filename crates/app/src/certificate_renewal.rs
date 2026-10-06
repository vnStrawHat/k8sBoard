//! Certificate "Renew now" (spec 0018 step 6): the intent of the one write to a custom resource, its
//! success notice, and the start from the cursor row. The write itself is `RenewCertificate` in the
//! cluster crate; the confirm dialog, the dry-run, the lock, the audit line, and the commit are the
//! shared 0030 flow, so a held Enter never confirms and a reconnect blocks the commit as for any
//! other change.

use cluster::{CustomResourceType, ObjectRef, WriteOperation, WriteRequest};
use gpui_kit::{App, Context, SharedString, Window};

use super::AppShell;
use super::write_flow::{WriteIntent, notify};
use crate::resource_actions::{
    KeyAvailability, ResourceAction, RowAction, action_label, action_risk, key_availability,
    kind_block, unavailable_text,
};
use crate::resource_kind::ResourceKind;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::workload_actions::WorkloadScope;

/// The first context line: ACME issuers count a re-issuance against their limits.
pub(crate) const RATE_LIMIT_WARNING: &str = "cert-manager requests a new certificate now; ACME issuers count it against their rate limits (Let's Encrypt: 5 duplicate certificates per week)";
/// The second one: the key is re-created with the certificate unless the Certificate pins it.
pub(crate) const PRIVATE_KEY_WARNING: &str =
    "The private key changes too unless privateKey.rotationPolicy is Never";

/// The intent of renewing the Certificate `namespace`/`name` of `resource`; `None` when `resource` is
/// not the cert-manager Certificate, or the name or namespace cannot form a request path.
pub(crate) fn renew_intent(
    scope: &WorkloadScope<'_>,
    resource: &CustomResourceType,
    namespace: &str,
    name: &str,
    now: jiff::Timestamp,
) -> Option<WriteIntent> {
    let target = ObjectRef::custom(
        resource.clone(),
        Some(namespace.to_owned()),
        name.to_owned(),
    )?;
    // Whole seconds, so the dry-run and the commit send the same body.
    let requested_at = jiff::Timestamp::from_second(now.as_second()).ok()?;
    let request = WriteRequest::new(target, WriteOperation::RenewCertificate { requested_at })?;
    let action = ResourceAction::RenewCertificate;
    Some(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!("Renew certificate {namespace}/{name}").into(),
        button: "Renew".into(),
        request,
        risk: action_risk(action),
        warnings: vec![RATE_LIMIT_WARNING.into(), PRIVATE_KEY_WARNING.into()],
    })
}

/// What the user reads after the commit went through.
pub(crate) fn renewal_notice(target: &ObjectRef) -> String {
    match target.namespace() {
        Some(namespace) => format!("Renewal requested for {namespace}/{}", target.name()),
        None => format!("Renewal requested for {}", target.name()),
    }
}

impl AppShell {
    /// Renew now on the cursor Certificate `subject`, in its own cluster: the row is read again, so
    /// the intent names the object as it is listed now, and the kind is checked again because a menu
    /// or a key may be a moment old.
    pub(crate) fn start_renew_certificate(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = ResourceAction::RenewCertificate;
        let label = action_label(action);
        let intent = {
            let (Some(guard), Some(live)) = (
                self.guard_for(&subject.cluster, cx),
                self.live_of(&subject.cluster, cx),
            ) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            let ResourceKey::Kind {
                kind: kind @ ResourceKind::Custom(custom),
                namespace: Some(namespace),
                name,
            } = &subject.key
            else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the object is not a certificate"),
                );
                return;
            };
            if let Some(reason) = kind_block(action, *kind) {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            if live.row_of(&subject.key).is_none() {
                let text = unavailable_text(label, "the object is no longer listed");
                notify(window, cx, text);
                return;
            }
            let scope = WorkloadScope {
                cluster: &subject.cluster,
                cluster_name: guard.display_name(),
            };
            renew_intent(
                &scope,
                custom.resource(),
                namespace,
                name,
                jiff::Timestamp::now(),
            )
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => notify(
                window,
                cx,
                unavailable_text(
                    label,
                    "the object is not a certificate or its name is not valid",
                ),
            ),
        }
    }

    /// What the Renew button of the Certificates header does: `Ok` when it acts on the cursor row,
    /// else why it is off. It reads the gate of the key, so the two never disagree.
    pub(crate) fn renew_header_state(
        &self,
        kind: ResourceKind,
        cx: &App,
    ) -> Result<(), SharedString> {
        const SELECT: &str = "Select a certificate";
        let subject = self
            .selected
            .as_ref()
            .filter(|subject| matches!(&subject.key, ResourceKey::Kind { kind: shown, .. } if *shown == kind))
            .ok_or(SELECT)?;
        let (Some(live), Some(guard)) = (
            self.live_of(&subject.cluster, cx),
            self.guard_for(&subject.cluster, cx),
        ) else {
            return Err("Not connected".into());
        };
        match key_availability(RowAction::RenewCertificate, &subject.key, live, &guard) {
            KeyAvailability::Run(_) => Ok(()),
            KeyAvailability::Disabled { reason } => Err(reason),
            KeyAvailability::NotOffered => Err(SELECT.into()),
        }
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen renew-confirm`: the Renew now dialog of a fixed Certificate of a fixed Production
    /// cluster, so it asks for the cluster name. It needs no cluster and can never send.
    pub(in crate::app_shell) fn open_renew_fixture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use std::rc::Rc;

        use cluster::ResourceScope;

        use crate::cluster_registry::ClusterRef;
        use crate::confirm_dialog::DialogKind;
        use crate::environment::Environment;

        let cluster = ClusterRef {
            kubeconfig: std::path::PathBuf::from("fixture.yaml"),
            context: "prod-eu-1".to_owned(),
        };
        let scope = WorkloadScope {
            cluster: &cluster,
            cluster_name: "prod-eu-1",
        };
        let resource = CustomResourceType {
            group: "cert-manager.io".to_owned(),
            version: "v1".to_owned(),
            kind: "Certificate".to_owned(),
            plural: "certificates".to_owned(),
            scope: ResourceScope::Namespaced,
        };
        let Some(intent) = renew_intent(
            &scope,
            &resource,
            "shop",
            "shop-tls",
            jiff::Timestamp::UNIX_EPOCH,
        ) else {
            return;
        };
        let (risk, expected) = (intent.risk, intent.expected().to_owned());
        self.open_fixed_dialog(
            DialogKind::Write(Rc::new(intent)),
            Environment::PRODUCTION,
            (risk, &expected),
            window,
            cx,
        );
    }
}
