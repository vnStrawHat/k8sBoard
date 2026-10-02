//! Pure RBAC helpers: how roles, bindings, and subjects read as text, which screen row each one
//! is, and the index that joins roles with the bindings that name them. The session's binding
//! lists are passed in; nothing here reads live state.

use std::collections::HashMap;

use cluster::{
    AccessCheck, BindingSummary, BroadGroup, PodSummary, RoleKind, RoleRef, RoleSummary, Subject,
    SubjectKind,
};

use crate::cluster_session::{AccessState, CompanionLists, LiveList, denied_binding_checks};
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

const CLUSTER_ADMIN: &str = "cluster-admin";
const SERVICE_ACCOUNTS_GROUP: &str = "system:serviceaccounts";
const SERVICE_ACCOUNTS_GROUP_PREFIX: &str = "system:serviceaccounts:";

/// `role/x`, `clusterrole/x`, or `{kind lowercased}/x` for a kind the API should not accept.
pub(crate) fn role_text(role: &RoleRef) -> String {
    format!("{}/{}", role.kind.to_string().to_lowercase(), role.name)
}

/// `sa {ns}/{name}`, `user {name}`, `group {name}`.
pub(crate) fn subject_text(subject: &Subject) -> String {
    match subject.kind {
        SubjectKind::ServiceAccount => format!("sa {}", service_account_text(subject)),
        SubjectKind::User => format!("user {}", subject.name),
        SubjectKind::Group => format!("group {}", subject.name),
    }
}

/// `{ns}/{name}`; the name alone for a cluster binding subject that has no namespace.
pub(crate) fn service_account_text(subject: &Subject) -> String {
    match &subject.namespace {
        Some(namespace) => format!("{namespace}/{}", subject.name),
        None => subject.name.clone(),
    }
}

/// `rolebinding/x` or `clusterrolebinding/x`.
pub(crate) fn binding_text(binding: &BindingSummary) -> String {
    let prefix = if binding.namespace.is_some() {
        "rolebinding"
    } else {
        "clusterrolebinding"
    };
    format!("{prefix}/{}", binding.name)
}

/// The row of the binding on its own screen.
pub(crate) fn binding_key(binding: &BindingSummary) -> ResourceKey {
    let kind = if binding.namespace.is_some() {
        ResourceKind::RoleBindings
    } else {
        ResourceKind::ClusterRoleBindings
    };
    ResourceKey::Kind {
        kind,
        namespace: binding.namespace.clone(),
        name: binding.name.clone(),
    }
}

/// The role the binding names: a Role lives in the binding namespace, a ClusterRole is cluster
/// wide; any other kind has no screen.
pub(crate) fn role_key(binding: &BindingSummary) -> Option<ResourceKey> {
    match binding.role.kind {
        RoleKind::Role => Some(ResourceKey::Kind {
            kind: ResourceKind::Roles,
            namespace: Some(binding.namespace.clone()?),
            name: binding.role.name.clone(),
        }),
        RoleKind::ClusterRole => Some(ResourceKey::Kind {
            kind: ResourceKind::ClusterRoles,
            namespace: None,
            name: binding.role.name.clone(),
        }),
        RoleKind::Other(_) => None,
    }
}

/// Who gets a role that grants everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BroadAdmin {
    /// `system:authenticated` or `system:unauthenticated`: callers nobody vetted.
    Everyone,
    /// A service account, or a group of them.
    ServiceAccounts,
}

/// cluster-admin is detected by name: a binding carries only the role name.
pub(crate) fn is_cluster_admin(role: &RoleRef) -> bool {
    role.kind == RoleKind::ClusterRole && role.name == CLUSTER_ADMIN
}

/// `Some` when the binding hands cluster-admin to a service account or a broad group.
pub(crate) fn broad_admin(binding: &BindingSummary) -> Option<BroadAdmin> {
    if !is_cluster_admin(&binding.role) {
        return None;
    }
    let is_everyone = binding.subjects.iter().any(|subject| {
        matches!(
            subject.broad_group(),
            Some(BroadGroup::Authenticated | BroadGroup::Unauthenticated)
        )
    });
    if is_everyone {
        return Some(BroadAdmin::Everyone);
    }
    let reaches_service_accounts = binding.subjects.iter().any(|subject| {
        subject.kind == SubjectKind::ServiceAccount || subject.broad_group().is_some()
    });
    reaches_service_accounts.then_some(BroadAdmin::ServiceAccounts)
}

/// The two binding lists of a companion; an empty slice is a list that is not needed.
pub(crate) struct BindingLists<'a> {
    pub(crate) role_bindings: &'a [BindingSummary],
    pub(crate) cluster_role_bindings: &'a [BindingSummary],
}

/// What the Bindings companion of an explorer kind can offer right now.
pub(crate) enum BindingsStatus<'a> {
    Ready(BindingLists<'a>),
    Loading,
    /// A list failed before its first snapshot.
    Failed(&'a str),
    /// The access report denies these lists, so the companion never starts.
    Denied(Vec<AccessCheck>),
}

/// The state of the Bindings companion of `kind` (Roles, ClusterRoles, ServiceAccounts).
pub(crate) fn bindings_status<'a>(
    kind: ResourceKind,
    access: &AccessState,
    companion: Option<&'a CompanionLists>,
) -> BindingsStatus<'a> {
    let denied = denied_binding_checks(kind, access);
    if !denied.is_empty() {
        return BindingsStatus::Denied(denied);
    }
    let Some(CompanionLists::Bindings {
        role_bindings,
        cluster_role_bindings,
    }) = companion
    else {
        return BindingsStatus::Loading;
    };
    for list in std::iter::once(role_bindings).chain(cluster_role_bindings) {
        if let LiveList::Failed { message } = list {
            return BindingsStatus::Failed(message);
        }
    }
    match ready_binding_lists(companion) {
        Some(lists) => BindingsStatus::Ready(lists),
        None => BindingsStatus::Loading,
    }
}

/// The binding lists of the companion once every list it runs has loaded, for joins that do not
/// tell why they are not ready. A list its kind does not need is empty.
pub(crate) fn ready_binding_lists(companion: Option<&CompanionLists>) -> Option<BindingLists<'_>> {
    let CompanionLists::Bindings {
        role_bindings,
        cluster_role_bindings,
    } = companion?
    else {
        return None;
    };
    let cluster_role_bindings = match cluster_role_bindings {
        Some(list) => list.ready_items()?,
        None => &[],
    };
    Some(BindingLists {
        role_bindings: role_bindings.ready_items()?,
        cluster_role_bindings,
    })
}

/// One role a service account holds, and the binding that gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoundRole {
    pub(crate) role: RoleRef,
    /// `None` for a role kind k8sBoard has no screen for.
    pub(crate) role_key: Option<ResourceKey>,
    pub(crate) binding: ResourceKey,
    pub(crate) binding_text: String,
    /// The group the binding names, when the account is reached through one.
    pub(crate) group: Option<String>,
}

impl BoundRole {
    /// The binding namespace; `None` for a ClusterRoleBinding.
    pub(crate) fn binding_namespace(&self) -> Option<&str> {
        match &self.binding {
            ResourceKey::Kind { namespace, .. } => namespace.as_deref(),
            ResourceKey::Pod { .. } | ResourceKey::Node { .. } => None,
        }
    }
}

/// Roles joined with the bindings that name them, and service accounts with the roles they hold.
/// Built once per join call or paint.
pub(crate) struct BindingIndex<'a> {
    /// Keyed on the role name; the role scope is checked at lookup, since few bindings share a name.
    by_role_name: HashMap<&'a str, Vec<&'a BindingSummary>>,
    /// Direct `ServiceAccount` subjects, by name, each with its namespace.
    by_account: HashMap<&'a str, Vec<(&'a str, BoundRole)>>,
    /// Bindings of `system:serviceaccounts` and `system:serviceaccounts:{ns}`, by group name.
    by_group: HashMap<&'a str, Vec<BoundRole>>,
    /// Bindings of `system:authenticated`: every account is authenticated.
    everyone: Vec<BoundRole>,
}

impl<'a> BindingIndex<'a> {
    /// One pass over both lists. A role kind without a screen is left out of `bindings_of_role`
    /// but still shows in an account Bound roles. The service-account rules are the cluster
    /// crate `BindingSummary::binds_service_account`: a direct subject, or the two groups.
    /// `system:authenticated` bindings are kept apart (`everyone_roles`): they reach every account
    /// but only matter for cluster-admin (decision 5). `system:unauthenticated` binds no account.
    pub(crate) fn build(lists: &BindingLists<'a>) -> Self {
        let mut by_role_name: HashMap<&str, Vec<&BindingSummary>> = HashMap::new();
        let mut by_account: HashMap<&str, Vec<(&str, BoundRole)>> = HashMap::new();
        let mut by_group: HashMap<&str, Vec<BoundRole>> = HashMap::new();
        let mut everyone: Vec<BoundRole> = Vec::new();
        for binding in lists
            .role_bindings
            .iter()
            .chain(lists.cluster_role_bindings)
        {
            if !matches!(binding.role.kind, RoleKind::Other(_)) {
                by_role_name
                    .entry(binding.role.name.as_str())
                    .or_default()
                    .push(binding);
            }
            for subject in &binding.subjects {
                match (subject.kind, subject.broad_group()) {
                    (SubjectKind::ServiceAccount, _) => {
                        let Some(namespace) = subject.namespace.as_deref() else {
                            continue;
                        };
                        by_account
                            .entry(subject.name.as_str())
                            .or_default()
                            .push((namespace, bound_role(binding, None)));
                    }
                    (
                        SubjectKind::Group,
                        Some(
                            BroadGroup::AllServiceAccounts
                            | BroadGroup::NamespaceServiceAccounts(_),
                        ),
                    ) => by_group
                        .entry(subject.name.as_str())
                        .or_default()
                        .push(bound_role(binding, Some(subject.name.clone()))),
                    (SubjectKind::Group, Some(BroadGroup::Authenticated)) => {
                        everyone.push(bound_role(binding, Some(subject.name.clone())));
                    }
                    _ => {}
                }
            }
        }
        Self {
            by_role_name,
            by_account,
            by_group,
            everyone,
        }
    }

    /// The roles `namespace/name` holds: direct bindings, then those of `system:serviceaccounts`
    /// and `system:serviceaccounts:{namespace}`; each run sorted by role text, then binding.
    pub(crate) fn bound_roles(&self, namespace: &str, name: &str) -> Vec<&BoundRole> {
        let group = format!("{SERVICE_ACCOUNTS_GROUP_PREFIX}{namespace}");
        let direct = self
            .by_account
            .get(name)
            .into_iter()
            .flatten()
            .filter(|(account_namespace, _)| *account_namespace == namespace)
            .map(|(_, bound)| bound);
        let all = self.by_group.get(SERVICE_ACCOUNTS_GROUP);
        let own = self.by_group.get(group.as_str());
        let mut roles: Vec<&BoundRole> = direct
            .chain(all.into_iter().flatten())
            .chain(own.into_iter().flatten())
            .collect();
        roles.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        roles
    }

    /// The roles every authenticated caller holds through `system:authenticated`.
    pub(crate) fn everyone_roles(&self) -> &[BoundRole] {
        &self.everyone
    }

    /// What an account holds for the Bound roles list, the cell, and the boxes: `bound_roles`,
    /// then the cluster-admin grants to `system:authenticated`. The stock `system:basic-user`
    /// style grants to everyone are left out, or they would sit on every account.
    pub(crate) fn roles_held(&self, namespace: &str, name: &str) -> Vec<&BoundRole> {
        let mut roles = self.bound_roles(namespace, name);
        roles.extend(
            self.everyone_roles()
                .iter()
                .filter(|bound| is_cluster_admin(&bound.role)),
        );
        roles
    }

    /// The bindings that name `role`, in list order. A Role is named only by RoleBindings of its
    /// namespace; a ClusterRole by ClusterRoleBindings and RoleBindings of any namespace.
    pub(crate) fn bindings_of_role(&self, role: &RoleSummary) -> Vec<&'a BindingSummary> {
        let named = self.by_role_name.get(role.name.as_str());
        named
            .into_iter()
            .flatten()
            .copied()
            .filter(|binding| match (&binding.role.kind, &role.namespace) {
                (RoleKind::Role, Some(namespace)) => binding.namespace.as_ref() == Some(namespace),
                (RoleKind::ClusterRole, None) => true,
                _ => false,
            })
            .collect()
    }
}

fn bound_role(binding: &BindingSummary, group: Option<String>) -> BoundRole {
    BoundRole {
        role: binding.role.clone(),
        role_key: role_key(binding),
        binding: binding_key(binding),
        binding_text: binding_text(binding),
        group,
    }
}

/// Direct first, then group; each by role kind, name, and binding.
fn sort_key(bound: &BoundRole) -> (bool, u8, &str, &str) {
    let kind_rank = match bound.role.kind {
        RoleKind::ClusterRole => 0,
        RoleKind::Role => 1,
        RoleKind::Other(_) => 2,
    };
    (
        bound.group.is_some(),
        kind_rank,
        bound.role.name.as_str(),
        bound.binding_text.as_str(),
    )
}

/// The service account a pod runs as; the API defaults it to `default`.
pub(crate) fn pod_account(pod: &PodSummary) -> &str {
    pod.service_account.as_deref().unwrap_or("default")
}

/// One subject of a ClusterRole drawer Bound to list, with the binding that gives it the role.
pub(crate) struct RoleSubject<'a> {
    pub(crate) text: String,
    pub(crate) binding: &'a BindingSummary,
    pub(crate) is_service_account: bool,
}

/// Every subject of `bindings`, service accounts first, then by text.
pub(crate) fn role_subjects<'a>(bindings: &[&'a BindingSummary]) -> Vec<RoleSubject<'a>> {
    let mut subjects: Vec<RoleSubject> = bindings
        .iter()
        .flat_map(|&binding| {
            binding.subjects.iter().map(move |subject| RoleSubject {
                text: subject_text(subject),
                binding,
                is_service_account: subject.kind == SubjectKind::ServiceAccount,
            })
        })
        .collect();
    subjects.sort_by(|a, b| {
        b.is_service_account
            .cmp(&a.is_service_account)
            .then_with(|| a.text.cmp(&b.text))
            .then_with(|| a.binding.name.cmp(&b.binding.name))
    });
    subjects
}

#[cfg(test)]
#[path = "access_bindings_tests.rs"]
mod access_bindings_tests;
