# 0023 · Cluster crate: RBAC snapshot, evaluator, rules review

[Back to index](README.md) · Step 1a · 0001/0002 rules apply (no kube types in public signatures, no spawned task). Reuses 0015 `RoleSummary`, `RbacRule`, `BindingSummary`, `RoleRef`, `RoleKind`, `Subject`, `SubjectKind` and their summarizers (made `pub(crate)` where private).

## Requests and identities (`rbac_evaluation.rs`)

```rust
pub struct AccessRequest { pub verb: String, pub target: RequestTarget }
pub enum RequestTarget { Resource(ResourceRequest), NonResource { path: String } }
pub struct ResourceRequest { pub group: String /* "" core */, pub resource: String,
    pub subresource: Option<String>, pub name: Option<String>, pub namespace: Option<String> /* None: cluster-wide */ }
pub struct Identity { user: Option<String>, groups: Vec<String> }
impl Identity {
    pub fn service_account(namespace: &str, name: &str) -> Self; // decision 10
    pub fn user(name: &str) -> Self;   // + system:authenticated; system:anonymous → system:unauthenticated instead
    pub fn group(name: &str) -> Self;
}
```

`Identity::applies_to(&Subject) -> bool` (private): `User` → name == user; `Group` → groups contain name; `ServiceAccount` → `system:serviceaccount:{ns}:{name}` == user (a subject without namespace never matches; 0015 already defaults RoleBinding subjects).

## Rule matching (private, the authorizer's `RuleAllows`)

`fn rule_match(rule: &RbacRule, request: &AccessRequest) -> Option<GrantNames>`

| Part | Matches when |
|---|---|
| verb | rule verbs contain `*` or the verb |
| group | rule groups contain `*` or the group |
| resource | rule resource is `*`; or equals `resource` (no subresource) / `resource/sub`; or is `*/sub` for a request with subresource `sub` (`pods/*` is not a wildcard) |
| names | `resourceNames` empty → `Any`; request name in the list → `Any`; request without name → `Only(names)`; else no match |
| non-resource | rule verbs as above and a URL equals the path, is `*`, or ends in `*` and the path starts with it after trimming **every** trailing `*` (`/apis/**` → prefix `/apis/`) |

A resource rule never matches a non-resource request and the reverse.

```rust
pub enum GrantNames { Any, Only(Vec<String>) }   // Only: "only for named objects" (decision 8)
```

## Snapshot and evaluation (`rbac_snapshot.rs`, `rbac_evaluation.rs`)

```rust
pub struct RbacSnapshot { pub roles: Vec<RoleSummary>, pub cluster_roles: Vec<RoleSummary>,
    pub role_bindings: Vec<BindingSummary>, pub cluster_role_bindings: Vec<BindingSummary>,
    pub coverage: RbacCoverage }
pub struct RbacCoverage { pub cluster_roles: bool /* listed */, pub cluster_bindings: bool,
    pub roles: NamespaceCoverage, pub role_bindings: NamespaceCoverage }
pub enum NamespaceCoverage { AllNamespaces, Namespaces(Vec<String>) /* those listed */ }
impl RbacCoverage { pub fn covers(&self, namespace: &str) -> bool; } // roles AND role_bindings cover it (the narrower)
pub struct Grant<'a> { pub subject: &'a Subject, pub binding: &'a BindingSummary, pub names: GrantNames }
pub struct EffectiveRule<'a> { pub rule: &'a RbacRule, pub binding: &'a BindingSummary, pub subject: &'a Subject }
impl RbacSnapshot {
    pub fn who_can(&self, request: &AccessRequest) -> Vec<Grant<'_>>;
    pub fn rules_of(&self, identity: &Identity, namespace: Option<&str>) -> Vec<EffectiveRule<'_>>;
    pub fn decide(&self, identity: &Identity, request: &AccessRequest) -> Vec<Grant<'_>>;
}
```

- **Applicable bindings** for request namespace `ns`: every ClusterRoleBinding; plus RoleBindings with `namespace == ns` only when the request is a resource request with `Some(ns)`.
- **Role resolution**: `Role` → the Role `(binding namespace, name)`; `ClusterRole` → by name; `Other` or missing → grants nothing (skipped silently, ceiling). For RoleBindings, non-resource rules of the resolved role are ignored.
- `who_can`: for each applicable binding whose role has a matching rule, one `Grant` per subject. Several matching rules of one binding merge: `Any` wins, else `Only` with the sorted union of names. Order: ClusterRoleBindings, then RoleBindings, snapshot order; subjects in binding order.
- `rules_of(identity, ns)`: every rule of every applicable binding (namespace `ns`, resource requests assumed) with a subject the identity `applies_to`; `subject` is the first subject in binding order that applies (as `appliesTo` scans). `None` → ClusterRoleBindings only.
- `decide`: `who_can` filtered to subjects the identity `applies_to` and `names == Any`; empty = RBAC grants nothing.
- Pure and allocation-light: one role lookup map (`HashMap<(Option<&str>, &str), &RoleSummary>`) built per call. The evaluator ignores coverage; the app words the gaps.

## Snapshot fetch (`rbac_snapshot.rs`)

```rust
impl ClusterConnection {
    /// `fallback`: the namespaces to list one by one when a cluster-wide Roles or RoleBindings list
    /// is Forbidden (the app passes the session's namespaces list, else the scope's namespaces).
    pub async fn read_rbac(&self, fallback: &[String]) -> Result<RbacSnapshot, ClusterError>;
}
```

| List | Request | `ClusterError::Forbidden` | Other error |
|---|---|---|---|
| ClusterRoles | one `list_all` | empty, `cluster_roles = false` | fails the call |
| ClusterRoleBindings | one `list_all` | empty, `cluster_bindings = false` | fails the call |
| Roles, RoleBindings (each on its own) | cluster-wide `list_all` → `AllNamespaces` | one list per `fallback` namespace (concurrent); a Forbidden namespace is skipped; `Namespaces(listed)` | fails the call |

- The four lists run concurrently (`try_join!`); lists keep API order. Actions: `listing roles`, `listing cluster roles`, `listing role bindings`, `listing cluster role bindings`.
- An empty `fallback` with a Forbidden cluster-wide list gives `Namespaces(vec![])`, not an error.
- The folding is pure and tested offline: `fn cluster_list<T>(Result<Vec<T>, ClusterError>) -> Result<(Vec<T>, bool), ClusterError>` and `fn namespace_lists<T>(Vec<(String, Result<Vec<T>, ClusterError>)>) -> Result<(Vec<T>, NamespaceCoverage), ClusterError>`.

## Rules review and request SSAR (`access_review.rs`)

```rust
pub struct RulesReview { pub rules: Vec<RbacRule>, pub is_incomplete: bool, pub evaluation_error: Option<String> }
impl ClusterConnection {
    pub async fn review_rules(&self, namespace: &str) -> Result<RulesReview, ClusterError>;
    pub async fn review_request(&self, request: &AccessRequest) -> Result<AccessDecision, ClusterError>;
}
```

- `review_rules`: one `Api::<SelfSubjectRulesReview>::all(..).create` with `spec.namespace`; action `reviewing rules`. `resourceRules` → `RbacRule` (urls empty), then `nonResourceRules` → `RbacRule` (groups and resources empty). `incomplete` → `is_incomplete`; empty `evaluationError` → `None`. Duplicate rules (one per binding) are kept; `permission_table` merges them.
- `review_request`: SSAR with `ResourceAttributes { group, resource, subresource, name, namespace, verb }` or `NonResourceAttributes { path, verb }`; decision through the existing `access_decision`. `review_one` builds its attributes through the same helper (no second builder).
- Both are non-mutating review creates (project rule); the 0001 grep allow-list names both.

## Probe (`examples/probe.rs`)

`--analysis`: after the access table prints (counts only, no names):
`rbac: cluster roles listed|denied · cluster bindings listed|denied · roles all|{n} namespaces · role bindings all|{n} namespaces · roles {n} · cluster roles {n} · role bindings {n} · cluster role bindings {n}`,
`rules review {ns}: {n} rules{, incomplete}`, `who can get secrets in {ns}: {n} grants ({m} only named)`,
`network policies {ns}: {n}` (added in step 1b); `{ns}` = the first `--namespace` or the context default. The `read_rbac` fallback is the namespaces the probe already lists. Errors print `error: …` lines and continue. Update `USAGE` and 0001 `probe-example.md`.
