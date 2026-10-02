//! One consistent snapshot of the RBAC objects (Roles, ClusterRoles, and both binding kinds),
//! listed once and evaluated client-side (`rbac_evaluation.rs`).

use futures::stream::{self, StreamExt};
use futures::try_join;
use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use k8s_openapi::serde::de::DeserializeOwned;
use kube::Api;

use crate::connection::{ClusterConnection, ClusterError};
use crate::role::{RoleSummary, cluster_role_summary, role_summary};
use crate::role_binding::{BindingSummary, cluster_role_binding_summary, role_binding_summary};

/// At most this many per-namespace fallback lists run at once; `buffered` keeps fallback order.
const FALLBACK_CONCURRENCY: usize = 8;

/// Every RBAC object the account may list, with what could not be listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RbacSnapshot {
    pub roles: Vec<RoleSummary>,
    pub cluster_roles: Vec<RoleSummary>,
    pub role_bindings: Vec<BindingSummary>,
    pub cluster_role_bindings: Vec<BindingSummary>,
    pub coverage: RbacCoverage,
}

/// What a restricted account could not see. A gap is data, never a failed snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RbacCoverage {
    /// Whether the ClusterRoles list was allowed.
    pub cluster_roles: bool,
    /// Whether the ClusterRoleBindings list was allowed.
    pub cluster_bindings: bool,
    pub roles: NamespaceCoverage,
    pub role_bindings: NamespaceCoverage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamespaceCoverage {
    AllNamespaces,
    /// Only these namespaces were listed (the cluster-wide list was not permitted).
    Namespaces(Vec<String>),
}

impl NamespaceCoverage {
    fn covers(&self, namespace: &str) -> bool {
        match self {
            Self::AllNamespaces => true,
            Self::Namespaces(listed) => listed.iter().any(|name| name == namespace),
        }
    }
}

impl RbacCoverage {
    /// Whether Roles and RoleBindings of `namespace` were both listed: the narrower of the two.
    pub fn covers(&self, namespace: &str) -> bool {
        self.roles.covers(namespace) && self.role_bindings.covers(namespace)
    }
}

impl ClusterConnection {
    /// Lists the four RBAC kinds concurrently. A Forbidden ClusterRoles or ClusterRoleBindings
    /// list becomes an empty list plus a coverage flag. A Forbidden cluster-wide Roles or
    /// RoleBindings list falls back to one list per `fallback` namespace (the namespaces the
    /// caller knows), skipping Forbidden ones. Any other error fails the call.
    pub async fn read_rbac(&self, fallback: &[String]) -> Result<RbacSnapshot, ClusterError> {
        let client = self.client();
        let (cluster_roles, cluster_bindings, roles, role_bindings) = try_join!(
            async {
                let api = Api::<ClusterRole>::all(client.clone());
                cluster_list(self.list_all(api, "listing cluster roles").await)
            },
            async {
                let api = Api::<ClusterRoleBinding>::all(client.clone());
                cluster_list(self.list_all(api, "listing cluster role bindings").await)
            },
            self.list_namespaced::<Role>(fallback, "listing roles"),
            self.list_namespaced::<RoleBinding>(fallback, "listing role bindings"),
        )?;
        let (cluster_roles, has_cluster_roles) = cluster_roles;
        let (cluster_bindings, has_cluster_bindings) = cluster_bindings;
        let (roles, roles_coverage) = roles;
        let (role_bindings, bindings_coverage) = role_bindings;
        Ok(RbacSnapshot {
            roles: roles.iter().map(role_summary).collect(),
            cluster_roles: cluster_roles.iter().map(cluster_role_summary).collect(),
            role_bindings: role_bindings.iter().map(role_binding_summary).collect(),
            cluster_role_bindings: cluster_bindings
                .iter()
                .map(cluster_role_binding_summary)
                .collect(),
            coverage: RbacCoverage {
                cluster_roles: has_cluster_roles,
                cluster_bindings: has_cluster_bindings,
                roles: roles_coverage,
                role_bindings: bindings_coverage,
            },
        })
    }

    async fn list_namespaced<K>(
        &self,
        fallback: &[String],
        action: &'static str,
    ) -> Result<(Vec<K>, NamespaceCoverage), ClusterError>
    where
        K: kube::Resource<Scope = NamespaceResourceScope>
            + Clone
            + DeserializeOwned
            + std::fmt::Debug,
        K::DynamicType: Default,
    {
        let client = self.client();
        match self.list_all(Api::<K>::all(client.clone()), action).await {
            Ok(items) => Ok((items, NamespaceCoverage::AllNamespaces)),
            Err(ClusterError::Forbidden { .. }) => {
                // Owned namespaces: a closure over `&String` makes the future not `Send` for
                // every lifetime once it runs on the runtime.
                let lists = stream::iter(fallback.iter().cloned().map(|namespace| async move {
                    let api = Api::<K>::namespaced(client.clone(), &namespace);
                    let result = self.list_all(api, action).await;
                    (namespace, result)
                }))
                .buffered(FALLBACK_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;
                namespace_lists(lists)
            }
            Err(error) => Err(error),
        }
    }
}

/// A Forbidden list is empty and unlisted; any other error stays an error.
fn cluster_list<T>(result: Result<Vec<T>, ClusterError>) -> Result<(Vec<T>, bool), ClusterError> {
    match result {
        Ok(items) => Ok((items, true)),
        Err(ClusterError::Forbidden { .. }) => Ok((Vec::new(), false)),
        Err(error) => Err(error),
    }
}

/// Folds the per-namespace fallback lists: a Forbidden namespace is skipped, any other error
/// fails. An empty fallback gives no namespaces, not an error.
fn namespace_lists<T>(
    lists: Vec<(String, Result<Vec<T>, ClusterError>)>,
) -> Result<(Vec<T>, NamespaceCoverage), ClusterError> {
    let mut items = Vec::new();
    let mut listed = Vec::new();
    for (namespace, result) in lists {
        match result {
            Ok(mut namespace_items) => {
                items.append(&mut namespace_items);
                listed.push(namespace);
            }
            Err(ClusterError::Forbidden { .. }) => {}
            Err(error) => return Err(error),
        }
    }
    Ok((items, NamespaceCoverage::Namespaces(listed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forbidden() -> ClusterError {
        ClusterError::Forbidden {
            context: "ctx".to_owned(),
            action: "listing roles",
            message: "no".to_owned(),
        }
    }

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn coverage(roles: NamespaceCoverage, bindings: NamespaceCoverage) -> RbacCoverage {
        RbacCoverage {
            cluster_roles: true,
            cluster_bindings: true,
            roles,
            role_bindings: bindings,
        }
    }

    #[test]
    fn coverage_covers_all_or_listed_namespaces() {
        let all = coverage(
            NamespaceCoverage::AllNamespaces,
            NamespaceCoverage::AllNamespaces,
        );
        assert!(all.covers("anything"));
        let some = NamespaceCoverage::Namespaces(names(&["a"]));
        let listed = coverage(some.clone(), some);
        assert!(listed.covers("a"));
        assert!(!listed.covers("b"));
    }

    #[test]
    fn covers_uses_narrower_of_roles_and_bindings() {
        let roles = NamespaceCoverage::Namespaces(names(&["a", "b"]));
        let bindings = NamespaceCoverage::Namespaces(names(&["b", "c"]));
        let narrow = coverage(roles, bindings);
        assert!(narrow.covers("b"));
        assert!(!narrow.covers("a"));
        assert!(!narrow.covers("c"));
        let mixed = coverage(
            NamespaceCoverage::AllNamespaces,
            NamespaceCoverage::Namespaces(names(&["a"])),
        );
        assert!(mixed.covers("a"));
        assert!(!mixed.covers("b"));
    }

    #[test]
    fn forbidden_cluster_list_gives_empty_and_flag() {
        let (items, is_listed) = cluster_list::<u8>(Err(forbidden())).expect("folded");
        assert!(items.is_empty());
        assert!(!is_listed);
        let (items, is_listed) = cluster_list(Ok(vec![1_u8])).expect("listed");
        assert_eq!(items, [1]);
        assert!(is_listed);
        let failed = ClusterError::TimedOut {
            context: "ctx".to_owned(),
            action: "listing roles",
        };
        assert!(cluster_list::<u8>(Err(failed)).is_err());
    }

    #[test]
    fn forbidden_namespace_is_skipped_in_fallback() {
        let lists = vec![
            ("a".to_owned(), Ok(vec![1_u8])),
            ("b".to_owned(), Err(forbidden())),
            ("c".to_owned(), Ok(vec![2_u8, 3])),
        ];
        let (items, coverage) = namespace_lists(lists).expect("folded");
        assert_eq!(items, [1, 2, 3]);
        assert_eq!(coverage, NamespaceCoverage::Namespaces(names(&["a", "c"])));
    }

    #[test]
    fn other_fallback_errors_fail_the_fold() {
        let failed = ClusterError::TimedOut {
            context: "ctx".to_owned(),
            action: "listing roles",
        };
        assert!(namespace_lists::<u8>(vec![("a".to_owned(), Err(failed))]).is_err());
    }

    #[test]
    fn empty_fallback_gives_no_namespaces() {
        let (items, coverage) = namespace_lists::<u8>(Vec::new()).expect("folded");
        assert!(items.is_empty());
        assert_eq!(coverage, NamespaceCoverage::Namespaces(Vec::new()));
    }
}
