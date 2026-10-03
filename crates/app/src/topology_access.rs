//! The pure part of the Topology RBAC layer (spec 0022, rbac-layer.md): which bindings the graph
//! reads, and the cluster-admin grant of a drawn ServiceAccount. Read-only: it joins summaries
//! that the feeds already hold, and it never sees a Secret.

use cluster::{BindingSummary, BroadGroup, SubjectKind};

use crate::access_bindings::{BindingIndex, BindingLists, is_cluster_admin};
use crate::kind_row::{KindObject, KindRow};
use crate::table_selection::ResourceKey;
use crate::topology_graph::{NodeId, TopologyKind};

/// Whether `binding` reaches an account of `namespace`: it names one directly, or names a group
/// that holds every account (`system:serviceaccounts`, `system:serviceaccounts:{namespace}`,
/// `system:authenticated`). The same rule decides which cluster-wide bindings the build keeps.
pub(crate) fn names_namespace_account(binding: &BindingSummary, namespace: &str) -> bool {
    binding.subjects.iter().any(|subject| match subject.kind {
        SubjectKind::ServiceAccount => subject.namespace.as_deref() == Some(namespace),
        SubjectKind::Group => match subject.broad_group() {
            Some(BroadGroup::AllServiceAccounts | BroadGroup::Authenticated) => true,
            Some(BroadGroup::NamespaceServiceAccounts(group_namespace)) => {
                group_namespace == namespace
            }
            Some(BroadGroup::Unauthenticated) | None => false,
        },
        SubjectKind::User => false,
    })
}

/// Cloned bindings for `BindingIndex::build`: the feeds hold `KindRow`s, and the index borrows
/// plain summaries. The copy is bounded by `RAW_LIMIT`, because only the cluster-wide bindings that
/// name an account of the namespace are kept. `counted_rows` in `topology_graph.rs` counts exactly
/// the rows this keeps, by the same `names_namespace_account` rule: change them together.
#[derive(Default)]
pub(crate) struct AccessBindings {
    role_bindings: Vec<BindingSummary>,
    cluster_role_bindings: Vec<BindingSummary>,
}

impl AccessBindings {
    pub(crate) fn collect<'a>(
        rows: impl Iterator<Item = (TopologyKind, &'a KindRow)>,
        namespace: &str,
    ) -> Self {
        let mut bindings = Self::default();
        for (kind, row) in rows {
            let KindObject::Binding(binding) = &row.object else {
                continue;
            };
            match kind {
                TopologyKind::RoleBinding => bindings.role_bindings.push(binding.clone()),
                TopologyKind::ClusterRoleBinding if names_namespace_account(binding, namespace) => {
                    bindings.cluster_role_bindings.push(binding.clone());
                }
                _ => {}
            }
        }
        bindings
    }

    pub(crate) fn lists(&self) -> BindingLists<'_> {
        BindingLists {
            role_bindings: &self.role_bindings,
            cluster_role_bindings: &self.cluster_role_bindings,
        }
    }
}

/// An account that holds cluster-admin, and the binding that hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClusterAdminGrant {
    /// The drawn node of the account.
    pub(crate) account: NodeId,
    pub(crate) account_name: String,
    /// The binding node when the account is a direct subject of it; `None` for a group grant.
    pub(crate) binding: Option<NodeId>,
    /// `clusterrolebinding/x` or `rolebinding/x`.
    pub(crate) binding_text: String,
    /// The namespace a RoleBinding limits the grant to; `None` for a ClusterRoleBinding, whose
    /// grant is cluster wide.
    pub(crate) binding_namespace: Option<String>,
    /// The group the binding names, for a grant the account holds through one.
    pub(crate) group: Option<String>,
}

/// The cluster-admin role `namespace/name` holds, as `roles_held` reads it: direct bindings
/// first, then the service-account groups, then `system:authenticated`.
pub(crate) fn cluster_admin_grant(
    index: &BindingIndex,
    namespace: &str,
    account: &NodeId,
    name: &str,
) -> Option<ClusterAdminGrant> {
    let held = index.roles_held(namespace, name);
    let bound = held
        .into_iter()
        .find(|bound| is_cluster_admin(&bound.role))?;
    let binding = match bound.group {
        Some(_) => None,
        None => binding_node(&bound.binding),
    };
    Some(ClusterAdminGrant {
        account: account.clone(),
        account_name: name.to_owned(),
        binding,
        binding_text: bound.binding_text.clone(),
        binding_namespace: bound.binding_namespace().map(str::to_owned),
        group: bound.group.clone(),
    })
}

/// The graph node of a RoleBinding or ClusterRoleBinding key.
pub(crate) fn binding_node(key: &ResourceKey) -> Option<NodeId> {
    let ResourceKey::Kind { kind, name, .. } = key else {
        return None;
    };
    let kind = TopologyKind::of_resource_kind(*kind).filter(|kind| {
        matches!(
            kind,
            TopologyKind::RoleBinding | TopologyKind::ClusterRoleBinding
        )
    })?;
    Some(NodeId::Object {
        kind,
        name: name.clone(),
    })
}

#[cfg(test)]
#[path = "topology_access_tests.rs"]
mod topology_access_tests;
