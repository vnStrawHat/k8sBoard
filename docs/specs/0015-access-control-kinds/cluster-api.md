# 0015 · Cluster crate: RBAC and service-account summaries, access, probe

[Back to index](README.md) · Step 1 · The 0001/0002 rules apply. Common fields as 0005; `namespace: Option<String>` on the shared Role and Binding types (`None` for the cluster-scoped kind). The only annotations read anywhere in this spec are the three cloud-identity keys below.

## ServiceAccount (`service_account.rs`, core/v1)

```rust
pub struct ServiceAccountSummary { /* common */ pub secrets: Vec<String> /* secrets[].name */,
    pub image_pull_secrets: Vec<String> /* imagePullSecrets[].name */, pub automount_token: Option<bool>,
    pub cloud_identities: Vec<CloudIdentity> }
pub struct CloudIdentity { pub provider: CloudProvider, pub value: String }
pub enum CloudProvider { Aws, Gcp, Azure }   // + label(): "IAM role", "GCP service account", "Azure client ID"
impl ClusterConnection { pub fn watch_service_accounts(&self, scope: NamespaceScope) -> impl Stream<..>; }
```

- Only names are copied from the object references; empty names are dropped.
- `cloud_identities`, in provider order, from exactly these keys (decision 13); empty values skipped; every other annotation is never read:

| Key | Provider |
|---|---|
| `eks.amazonaws.com/role-arn` | `Aws` |
| `iam.gke.io/gcp-service-account` | `Gcp` |
| `azure.workload.identity/client-id` | `Azure` |

- The module imports no Secret type and makes no request other than its watch. Action text `watching service accounts`.

## Roles (`role.rs`, rbac.authorization.k8s.io/v1)

```rust
pub struct RoleSummary { pub namespace: Option<String>, pub name: String, pub created_at: Option<jiff::Timestamp>,
    pub labels: Vec<String>, pub rules: Vec<RbacRule>,
    pub aggregation: Vec<Selector> /* ClusterRole aggregationRule.clusterRoleSelectors; always empty for Roles */ }
pub struct RbacRule { pub api_groups: Vec<String>, pub resources: Vec<String>, pub resource_names: Vec<String>,
    pub verbs: Vec<String>, pub non_resource_urls: Vec<String> }
impl RbacRule { pub fn grants_everything(&self) -> bool; }   // verbs, resources, api_groups each contain "*" and resource_names is empty
impl RoleSummary { pub fn grants_everything(&self) -> bool; pub fn is_aggregated(&self) -> bool;
    pub fn is_built_in(&self) -> bool; /* label kubernetes.io/bootstrapping=rbac-defaults */ }
impl ClusterConnection {
    pub fn watch_roles(&self, scope: NamespaceScope) -> impl Stream<..>;
    pub fn watch_cluster_roles(&self) -> impl Stream<..>;
}
```

Lists keep API order. `Selector` is 0012's (`Selector::of`).

## Bindings (`role_binding.rs`)

```rust
pub struct BindingSummary { pub namespace: Option<String>, pub name: String, pub created_at: Option<jiff::Timestamp>,
    pub labels: Vec<String>, pub role: RoleRef, pub subjects: Vec<Subject> }
pub struct RoleRef { pub kind: RoleKind, pub name: String }
pub enum RoleKind { Role, ClusterRole, Other(String) }      // + Display: "Role", "ClusterRole", the text as written
pub struct Subject { pub kind: SubjectKind, pub name: String, pub namespace: Option<String> }
pub enum SubjectKind { User, Group, ServiceAccount }
pub enum SubjectMatch { Direct, Group(String) }            // how a binding reaches one service account
pub enum BroadGroup { Authenticated, Unauthenticated, AllServiceAccounts, NamespaceServiceAccounts(String) }
impl Subject { pub fn broad_group(&self) -> Option<BroadGroup>; } // Group subjects only
impl BindingSummary {
    /// The authorizer's `appliesTo` for one service account (decision 4); groups system:authenticated
    /// and system:unauthenticated are ignored here (decision 5).
    pub fn binds_service_account(&self, namespace: &str, name: &str) -> Option<SubjectMatch>;
    pub fn has_service_account_subject(&self) -> bool;
}
impl ClusterConnection {
    pub fn watch_role_bindings(&self, scope: NamespaceScope) -> impl Stream<..>;
    pub fn watch_cluster_role_bindings(&self) -> impl Stream<..>;
}
```

- `roleRef.kind`: `"Role"` → `Role`, `"ClusterRole"` → `ClusterRole`, anything else → `Other(text)` (decision 9).
- Subject kinds other than the three are dropped. A `ServiceAccount` subject without a namespace gets the binding's namespace (RoleBinding); for a ClusterRoleBinding it stays `None` and never matches.
- `binds_service_account(ns, name)`: a `ServiceAccount` subject with that namespace and name → `Direct`; else a `Group` subject `system:serviceaccounts` or `system:serviceaccounts:{ns}` → `Group(name)`; else `None`.
- `broad_group`: `system:authenticated`, `system:unauthenticated`, `system:serviceaccounts`, `system:serviceaccounts:{ns}`; other groups → `None`.

## Other additions

| Item | Change |
|---|---|
| `access_review.rs` | `ListServiceAccounts` ("", ns), `ListRoles` (rbac.authorization.k8s.io, ns), `ListClusterRoles` (rbac…, cluster), `ListRoleBindings` (rbac…, ns), `ListClusterRoleBindings` (rbac…, cluster); appended to `ALL` |
| `object_yaml.rs` | `ObjectKind::{ServiceAccount, Role, ClusterRole, RoleBinding, ClusterRoleBinding}`; `ClusterRole` and `ClusterRoleBinding` cluster-scoped; `api_resource` arms |
| `object_count.rs` (0012) | five arms |
| `lib.rs` | modules; export `ServiceAccountSummary`, `CloudIdentity`, `CloudProvider`, `RoleSummary`, `RbacRule`, `BindingSummary`, `RoleRef`, `RoleKind`, `Subject`, `SubjectKind`, `SubjectMatch`, `BroadGroup` |
| `examples/probe.rs` | `--watch-seconds`: `service accounts`, `roles`, `cluster roles`, `role bindings`, `cluster role bindings` after the 0014 lines; `USAGE`; 0001 `probe-example.md` |

The YAML view needs no new masking: these objects hold no secret values; the 0007 annotation masking still applies.
