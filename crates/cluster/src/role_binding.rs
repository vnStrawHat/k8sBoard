use std::fmt;

use futures::Stream;
use k8s_openapi::api::rbac::v1::{
    ClusterRoleBinding, RoleBinding, RoleRef as ApiRoleRef, Subject as ApiSubject,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Api;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::label_terms;

pub(crate) const SERVICE_ACCOUNTS_GROUP: &str = "system:serviceaccounts";
pub(crate) const SERVICE_ACCOUNTS_GROUP_PREFIX: &str = "system:serviceaccounts:";
pub(crate) const AUTHENTICATED_GROUP: &str = "system:authenticated";
pub(crate) const UNAUTHENTICATED_GROUP: &str = "system:unauthenticated";

/// A RoleBinding (`namespace: Some`) or a ClusterRoleBinding (`namespace: None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingSummary {
    pub namespace: Option<String>,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    pub role: RoleRef,
    pub subjects: Vec<Subject>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleRef {
    pub kind: RoleKind,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleKind {
    Role,
    ClusterRole,
    /// The text as written; the API rejects other kinds today.
    Other(String),
}

impl fmt::Display for RoleKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Role => formatter.write_str("Role"),
            Self::ClusterRole => formatter.write_str("ClusterRole"),
            Self::Other(text) => formatter.write_str(text),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    pub kind: SubjectKind,
    pub name: String,
    /// Set for service accounts; a RoleBinding subject without one gets the binding namespace.
    pub namespace: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubjectKind {
    User,
    Group,
    ServiceAccount,
}

/// How a binding reaches one service account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubjectMatch {
    Direct,
    /// Through the named service-account group.
    Group(String),
}

/// A group that hands a role to many callers at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BroadGroup {
    Authenticated,
    Unauthenticated,
    AllServiceAccounts,
    NamespaceServiceAccounts(String),
}

impl Subject {
    /// `Some` for the four broad groups; every other subject, `system:masters` included, is `None`.
    pub fn broad_group(&self) -> Option<BroadGroup> {
        if self.kind != SubjectKind::Group {
            return None;
        }
        match self.name.as_str() {
            AUTHENTICATED_GROUP => Some(BroadGroup::Authenticated),
            UNAUTHENTICATED_GROUP => Some(BroadGroup::Unauthenticated),
            SERVICE_ACCOUNTS_GROUP => Some(BroadGroup::AllServiceAccounts),
            name => name
                .strip_prefix(SERVICE_ACCOUNTS_GROUP_PREFIX)
                .filter(|namespace| !namespace.is_empty())
                .map(|namespace| BroadGroup::NamespaceServiceAccounts(namespace.to_owned())),
        }
    }
}

impl BindingSummary {
    /// The authorizer's `appliesTo` rule for one service account. The groups
    /// `system:authenticated` and `system:unauthenticated` are ignored here: they bind
    /// everyone, and listing them on every account is noise.
    pub fn binds_service_account(&self, namespace: &str, name: &str) -> Option<SubjectMatch> {
        let is_direct = self.subjects.iter().any(|subject| {
            subject.kind == SubjectKind::ServiceAccount
                && subject.name == name
                && subject.namespace.as_deref() == Some(namespace)
        });
        if is_direct {
            return Some(SubjectMatch::Direct);
        }
        self.subjects
            .iter()
            .find(|subject| match subject.broad_group() {
                Some(BroadGroup::AllServiceAccounts) => true,
                Some(BroadGroup::NamespaceServiceAccounts(group_namespace)) => {
                    group_namespace == namespace
                }
                _ => false,
            })
            .map(|subject| SubjectMatch::Group(subject.name.clone()))
    }

    pub fn has_service_account_subject(&self) -> bool {
        self.subjects
            .iter()
            .any(|subject| subject.kind == SubjectKind::ServiceAccount)
    }
}

impl ClusterConnection {
    /// Watches role bindings in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_role_bindings(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<BindingSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching role bindings",
            role_binding_summary,
        )
    }

    /// Watches cluster role bindings. Yields batched snapshots ordered by name.
    pub fn watch_cluster_role_bindings(
        &self,
    ) -> impl Stream<Item = WatchUpdate<BindingSummary>> + Send + 'static {
        let api = Api::<ClusterRoleBinding>::all(self.client().clone());
        summary_watch(
            self,
            vec![(None, api)],
            "watching cluster role bindings",
            cluster_role_binding_summary,
        )
    }
}

pub(crate) fn role_binding_summary(binding: &RoleBinding) -> BindingSummary {
    let namespace = binding.metadata.namespace.clone();
    summary_of(
        &binding.metadata,
        namespace.as_deref(),
        &binding.role_ref,
        binding.subjects.as_deref(),
    )
}

pub(crate) fn cluster_role_binding_summary(binding: &ClusterRoleBinding) -> BindingSummary {
    summary_of(
        &binding.metadata,
        None,
        &binding.role_ref,
        binding.subjects.as_deref(),
    )
}

/// `default_namespace` is the namespace of the binding itself: a RoleBinding service-account subject without a
/// namespace means that one, and a ClusterRoleBinding subject without one never matches.
fn summary_of(
    metadata: &ObjectMeta,
    default_namespace: Option<&str>,
    role_ref: &ApiRoleRef,
    subjects: Option<&[ApiSubject]>,
) -> BindingSummary {
    BindingSummary {
        namespace: default_namespace.map(str::to_owned),
        name: metadata.name.clone().unwrap_or_default(),
        created_at: metadata.creation_timestamp.as_ref().map(|time| time.0),
        labels: label_terms(metadata),
        role: RoleRef {
            kind: match role_ref.kind.as_str() {
                "Role" => RoleKind::Role,
                "ClusterRole" => RoleKind::ClusterRole,
                other => RoleKind::Other(other.to_owned()),
            },
            name: role_ref.name.clone(),
        },
        subjects: subjects
            .unwrap_or_default()
            .iter()
            .filter_map(|subject| subject_of(subject, default_namespace))
            .collect(),
    }
}

fn subject_of(subject: &ApiSubject, default_namespace: Option<&str>) -> Option<Subject> {
    let (kind, namespace) = match subject.kind.as_str() {
        "User" => (SubjectKind::User, None),
        "Group" => (SubjectKind::Group, None),
        "ServiceAccount" => (
            SubjectKind::ServiceAccount,
            subject
                .namespace
                .clone()
                .filter(|namespace| !namespace.is_empty())
                .or_else(|| default_namespace.map(str::to_owned)),
        ),
        _ => return None,
    };
    Some(Subject {
        kind,
        name: subject.name.clone(),
        namespace,
    })
}

#[cfg(test)]
#[path = "role_binding_tests.rs"]
mod role_binding_tests;
