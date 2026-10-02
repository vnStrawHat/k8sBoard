# 0023 · Dialogs, texts, caveats, Can do

[Back to index](README.md) · Steps 2–4. Kit `Dialog` 760 px wide, body in a vertical scroll (max 520 px); kit `Input`, `Select` (searchable for pods), `Button`; colors from the theme, tones via `tone_color` (0003 AC4). Links are mono, styled like drawer links. Every dialog ends with a muted **source line** and a muted **caveats** paragraph (AC 6).

## Snapshot states (Who can, Check permissions for others, Can do)

| `RbacState` | Shows |
|---|---|
| Idle / Loading | muted `Listing RBAC objects…` |
| Failed | Bad `{error}` + `Retry` button (`refresh_rbac`) |
| Ready | results; source line `Computed from RBAC objects listed at {HH:MM:SS}` + `Refresh`; coverage Warn lines (below) |

Coverage Warn lines (pure `coverage_notes(&RbacCoverage, Option<&str /* request ns */>) -> Vec<SharedString>`, in `who_can_view.rs`), one per gap:

| Gap | Text |
|---|---|
| `!cluster_bindings` | `ClusterRoleBindings were not listed; only namespace grants are shown.` |
| `!cluster_roles` | `ClusterRoles were not listed; grants through them are not shown.` |
| roles or role bindings `Namespaces(..)` | `Roles and RoleBindings were listed in {n} namespaces only (not permitted cluster-wide).` |
| request namespace not `covers(ns)` | `RoleBindings of {ns} were not listed; only cluster-wide grants are shown.` |

**RBAC caveats**: `Not covered: other authorizers (Node, webhook), admission, impersonation, and groups assigned at sign-in. Bindings to missing roles grant nothing.`

## Who can… (step 2)

| Row | Content |
|---|---|
| query | `Input` placeholder `verb resource[.group][/subresource] [name]   or   verb /url   (e.g. list pods.metrics.k8s.io)`; Namespace `Select` (namespaces list, plus `All namespaces (cluster-wide grants)`); `Check` (also Enter) |
| hint | `AssumedCoreGroup` → muted `Unknown resource: assumed the core group; write resource.group`; `ClusterScoped` → muted `Cluster-scoped resource: namespace ignored`; `QueryError` → Bad text (below) |
| headline | `{n} subjects can {verb} {target}{ in {ns} | cluster-wide}` ("1 subject"); Warn when a 0015 `broad_group()` subject is among them |
| fixed row | muted `group system:masters` · `always allowed (bypasses RBAC)` |
| subjects | ordered: broad groups, users, groups, service accounts; then name. Subject text (0015 `subject_text`, mono; service account → link to its row; broad groups toned Bad for authenticated/unauthenticated, else Warn). Under it one line per grant: `via {binding_text} → {role_text}` (both links via 0015 `binding_key`, `role_key`) · muted `cluster-wide` or `in {ns}` |
| only named | sub-heading `Only for named objects`, same rows with ` · only {names}` (max 5 names, `+{n}`) |
| empty | `No RBAC binding grants this.` |

Query errors: `Empty` "Type a verb and a resource, for example get pods"; `MissingTarget` "Add a resource or a /url after the verb"; `EmptyPart` "Resource, group, and subresource cannot be empty (for example deployments.apps, pods/log)"; `NameWithUrl` "A URL takes no object name"; `TooManyWords` "Use: verb resource [name]"; `InvalidSubject` "Write sa ns/name, user name, or group name".

## Check permissions (step 3)

| Row | Content |
|---|---|
| subject | `Input` placeholder `You — or sa ns/name, user name, group name`; Namespace `Select` (You: namespaces only, from the namespaces list, else the scope's namespaces when that list failed, pure `you_namespaces`; others also `All namespaces (cluster-wide grants)`); `Check` |
| ask | `Can` + request `Input` + `?` + `Ask`; answer: Ok `Yes` / Bad `No`. You (SSAR): the denial reason when given. Others (`decide`): `via {binding_text} → {role_text}` of the first grant, or `No RBAC binding grants this.` |
| table | header `Resource` · 8 verb columns (`TABLE_VERBS`, short: get list watch create update patch delete delcol) · `Other`. Cells: `✓` (All), `names` (Names, tooltip lists them), empty. Resource cell `{resource}` with the muted `.{group}` stacked under it (long group names do not fit beside it at 760 px); everything rows say `all` in each cell, Warn. Then URL rows `{url}` · verbs. `… {n} more rows` when capped. Empty → `No permissions in {ns}.` |
| granted by | others only: one row per contributing binding `{binding_text} → {role_text}` (links) · muted `via {subject_text}` |
| source | You: `From SelfSubjectRulesReview for {ns}`; `is_incomplete` → Warn `Incomplete: rules shown are granted; others may be missing ({evaluation_error})` (no error text → without the parentheses). Others: the snapshot source line |
| caveats | You: `The API server's own answer for your credentials. Webhook authorizers can leave it incomplete.` Others: RBAC caveats; a user adds `Only bindings to this user name and system:authenticated count.` |

Table source for others: `permission_table(rules_of(identity, ns).map(|r| r.rule))`; for You: `permission_table(&review.rules)`.

## ServiceAccount drawer: Can do (step 3, `LiveContent::CanDo`)

- Section `Can do` after Cloud identity, before Used by (W7 order), for every ServiceAccount.
- Ready: toned chips from `can_do_chips(permission_table(rules))`, `rules` = `rules_of(Identity::service_account(ns, name), Some(ns))` without grants whose subject is group `system:authenticated` (decision 17); then `+{more} more` chip; then Note `In {ns} and cluster-wide · computed from RBAC objects. Check permissions shows the full table.` No rules → Note `No permissions from RBAC bindings`.
- Other states as the table above (one muted line). Coverage miss → the Warn note.

## Test traffic (step 4)

```rust
pub(crate) struct TrafficForm { pub(crate) source: Option<ResourceKey>, pub(crate) destination: Option<ResourceKey>, pub(crate) port: String }
pub(crate) fn traffic_defaults(pods: &[PodSummary], policy: Option<&NetworkPolicySummary>) -> TrafficForm;
```

`traffic_defaults` (pure): destination = first pod (namespace, name order) the policy selects, else the first pod; source = the first other pod in that namespace, else the first other pod; port = the destination's first main-container port number, else `80`.

| Row | Content |
|---|---|
| source | segmented `Pod` / `Labels` / `IP address`; Pod → searchable `Select` of `{ns}/{name}`; Labels → Namespace `Select` + `Input` `app=web,tier=front` (sorted into terms); IP → `Input` (invalid → Bad `Not an IP address`) |
| destination | searchable pod `Select` |
| port | `Input` (number or name) · Protocol `Select` TCP/UDP/SCTP · `Check` |
| verdict | large Ok `Allowed` / Bad `Denied`; port names add muted `Port {name} is {n}` |
| Egress from {source} | `NotIsolated` "No policy limits egress from the source." · `NotApplicable` "The source is outside the cluster; egress is not checked." · `Allowed` one row per rule `{policy} · egress rule {i+1}: {peer_text}, {ports_text}` (policy link) · `Denied` "Isolated by {policies}; no egress rule allows {destination} on {port}/{protocol}." |
| Ingress to {destination} | same wording with ingress and `{source}` |
| warnings (Warn) | host-network pod (from `PodSummary.host_network`): `{pod} uses the host network; most network plugins do not apply NetworkPolicy to it.`; source or destination without an IP: `{endpoint} has no IP; ipBlock rules were treated as not matching.`; an allowing egress rule with `named_port`: `Egress rule {i+1} of {policy} matched by port name {x}, resolved on the destination; some network plugins do not support named egress ports.`; each unknown namespace: `Labels of namespace {x} are unknown; its namespace selectors were treated as not matching.` |
| source | `Computed from NetworkPolicies of {ns}{ and {ns2}} listed at {HH:MM:SS}` |
| caveats | `Not covered: CNI-specific policies (Calico, Cilium), AdminNetworkPolicy, Services, and whether the pod listens on the port. ipBlock rules against pod IPs depend on the network plugin.` |

`peer_text` and `ports_text` are 0013's (`network_policy_rows.rs`, made `pub(crate)`); "any source"/"any destination" for a rule with no peers. `TrafficError::UnknownPortName` → Bad `The destination has no port named {x}`.
