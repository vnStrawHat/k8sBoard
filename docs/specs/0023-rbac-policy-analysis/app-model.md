# 0023 · App: session state, async contract, query parser, permission table, entry points

[Back to index](README.md) · Steps 2–4 · Layout and texts are in [dialogs.md](dialogs.md).

## RBAC snapshot state (step 2, `cluster_session.rs`)

```rust
pub(crate) enum RbacState { Idle, Loading { _task: Task<()> },
    Ready { snapshot: Rc<RbacSnapshot>, listed_at: jiff::Timestamp }, Failed(String) }
impl LiveCluster { pub(crate) fn rbac(&self) -> &RbacState; }   // new field, starts Idle
impl ClusterSession {
    pub(crate) fn request_rbac(&mut self, cx: &mut Context<Self>); // Idle | Failed → Loading; else no-op
    pub(crate) fn refresh_rbac(&mut self, cx: &mut Context<Self>); // Ready | Failed → Loading; Loading → no-op
}
```

- Fallback list: the ready namespaces list (`live.namespaces`) names, else `scope.namespaces()`; passed as `read_rbac(&fallback)`.
- Async: `ClusterRuntime::spawn(connection.read_rbac(&fallback))` → `RuntimeTask`; `cx.spawn` awaits it, sets `Ready` (`Rc::new`, `listed_at = now`) or `Failed(error.to_string())`, one `cx.notify()`. Dropping `Loading` drops the GPUI task, which drops the `RuntimeTask` (abort).
- Both methods go through pure `fn starts_fetch(state: &RbacState, trigger: RbacTrigger) -> bool` (`RbacTrigger::{Request, Refresh}`; the rules above), tested without a window.
- Snapshot is immutable once `Ready` (`Rc<RbacSnapshot>`, never mutated; Refresh replaces it). Reset to `Idle` when the session scope changes (fallback coverage may differ). Not counted in `open_watch_count` (no watch).
- No tracing of snapshot contents; errors trace as today's one-shot requests do.

## Dialog views and their one-shot requests

| View (module) | Owns | Request on Check |
|---|---|---|
| `WhoCanView` (`who_can_view.rs`) | query `InputState`, namespace `Select`, `result: Option<WhoCanResult>` | none: `request_rbac` then evaluates synchronously when the snapshot is `Ready` (`cx.observe(session)`); `// ponytail: who_can runs on the GPUI thread, O(bindings × rules); move to the runtime if a Check stalls a frame` |
| `PermissionsView` (`permissions_view.rs`) | subject input, namespace `Select`, "can …?" input, `table: RequestState<PermissionTable>`, `answer: RequestState<Answer>` | You: `review_rules(ns)` and `review_request(&req)` via runtime + `cx.spawn`; others: snapshot evaluation |
| `TrafficTestView` (`traffic_test_view.rs`) | source mode, pod `Select`s (searchable), namespace `Select`, labels/IP/port inputs, protocol `Select`, `verdict: RequestState<TrafficOutcome>` | `read_network_policies` for the 1–2 namespaces (`try_join`), then `evaluate_traffic` on the GPUI thread |

- **View lifetime**: `AppShell::open_*` creates the view once with `cx.new(...)` (starting any prefilled Check there), then calls `window.open_dialog(cx, move |dialog, _, _| dialog.title(..).w(px(760.)).child(view.clone()))`. The builder runs every frame and only clones the entity handle; it never creates a view, input state, or request.
- `enum RequestState<T> { Idle, Loading { _task: Task<()> }, Ready(T), Failed(String) }` (in `permissions_view.rs`, reused by the traffic view). A new Check replaces the state, dropping (aborting) the previous request; closing the dialog drops the view and its tasks.
- `who_can_view.rs` pure: `fn who_can_groups(grants: &[Grant]) -> WhoCanGroups { full: Vec<SubjectGroup>, named_only: Vec<SubjectGroup> }`, `SubjectGroup { subject_text, tone, account: Option<ResourceKey>, lines: Vec<GrantLine> }` (order and texts in [dialogs.md](dialogs.md)).
- Views hold `WeakEntity<ClusterSession>` and `WeakEntity<AppShell>`; links call `window.close_dialog(cx)` then `AppShell::reveal(key)`.
- Pods, namespaces, and pod ports are read from `LiveCluster` at Check time; a pod that vanished → `Failed("Pod {ns}/{name} no longer exists")`.

## Query parser (step 2, `access_query.rs`, pure, tests in module)

```rust
pub(crate) struct ParsedRequest { pub(crate) request: AccessRequest, pub(crate) hint: Option<QueryHint> }
pub(crate) enum QueryHint { AssumedCoreGroup, ClusterScoped }
pub(crate) enum QueryError { Empty, MissingTarget, EmptyPart, NameWithUrl, TooManyWords, InvalidSubject }
pub(crate) fn parse_request(text: &str, namespace: Option<&str>) -> Result<ParsedRequest, QueryError>;
pub(crate) enum SubjectQuery { You, Other { text: String /* 0015 subject_text */, identity: Identity, account: Option<ResourceKey> } }
pub(crate) fn parse_subject(text: &str) -> Result<SubjectQuery, QueryError>;
pub(crate) fn who_can_prefill(rules: &[RbacRule]) -> Option<String>;
const BUILT_IN_RESOURCES: &[(&str /* plural */, &str /* group */, bool /* namespaced */)];
```

- Request: trim, then whitespace words `verb target [name]`; the verb is lowercased. Target `/…` → `NonResource` (a name → `NameWithUrl`). Else split the first `/` into subresource, then the first `.` into resource and group; an empty resource, subresource, or group part (`.apps`, `pods/`, `pods.`) → `EmptyPart`. Resource `*` without a group → group `*`. No group: table lookup → its group; unknown → `""` + `AssumedCoreGroup`. Cluster-scoped (table) → namespace `None` + `ClusterScoped`. Resource and group are kept as typed.
- Table (v1.29): core `pods services endpoints configmaps secrets serviceaccounts persistentvolumeclaims events limitranges resourcequotas replicationcontrollers podtemplates` (ns), `nodes namespaces persistentvolumes` (cluster); `apps` deployments statefulsets daemonsets replicasets controllerrevisions; `batch` jobs cronjobs; `autoscaling` horizontalpodautoscalers; `policy` poddisruptionbudgets; `networking.k8s.io` ingresses networkpolicies (ns), ingressclasses (cluster); `discovery.k8s.io` endpointslices; `rbac.authorization.k8s.io` roles rolebindings (ns), clusterroles clusterrolebindings (cluster); `coordination.k8s.io` leases; `storage.k8s.io` storageclasses volumeattachments csidrivers csinodes (cluster), csistoragecapacities (ns); `apiextensions.k8s.io` customresourcedefinitions; `admissionregistration.k8s.io` mutatingwebhookconfigurations validatingwebhookconfigurations; `certificates.k8s.io` certificatesigningrequests; `scheduling.k8s.io` priorityclasses; `node.k8s.io` runtimeclasses; `admissionregistration.k8s.io` also validatingadmissionpolicies validatingadmissionpolicybindings; `apiregistration.k8s.io` apiservices; `flowcontrol.apiserver.k8s.io` flowschemas prioritylevelconfigurations; `authentication.k8s.io` tokenreviews selfsubjectreviews; `authorization.k8s.io` selfsubjectaccessreviews selfsubjectrulesreviews subjectaccessreviews (all cluster), localsubjectaccessreviews (ns). Anything else (for example `pods.metrics.k8s.io`) names its group.
- Subject: empty or `you` → `You`; `sa ns/name`, `serviceaccount ns/name`, `system:serviceaccount:ns:name` → service account (with `ResourceKey` of its row); `user x`; `group x`; else `InvalidSubject`.
- `who_can_prefill`: first rule with resources → `{first verb} {first resource}` + `.{first group}` when the group is not `""`; none → `None`.

## Permission table (step 3, `permission_table.rs`, pure, tests in module)

```rust
pub(crate) const TABLE_VERBS: [&str; 8] = ["get", "list", "watch", "create", "update", "patch", "delete", "deletecollection"];
pub(crate) enum VerbCell { Empty, All, Names(Vec<String>) }
pub(crate) struct PermissionRow { pub(crate) group: String, pub(crate) resource: String,
    pub(crate) cells: [VerbCell; 8], pub(crate) other_verbs: Vec<String>, pub(crate) is_everything: bool }
pub(crate) struct UrlRow { pub(crate) url: String, pub(crate) verbs: Vec<String> }
pub(crate) struct PermissionTable { pub(crate) rows: Vec<PermissionRow>, pub(crate) url_rows: Vec<UrlRow>, pub(crate) hidden_rows: usize }
pub(crate) fn permission_table<'a>(rules: impl IntoIterator<Item = &'a RbacRule>) -> PermissionTable;
pub(crate) fn can_do_chips(table: &PermissionTable) -> CanDoChips; // { chips: Vec<(SharedString, Option<StatusTone>)>, more: usize }
```

- Each rule expands to groups × resources rows (key `(group, resource)`, subresources as `pods/log`). Verb `*` fills all 8 cells and adds `*` to `other_verbs`; other verbs not in `TABLE_VERBS` go to `other_verbs` (sorted, unique). `resourceNames` → `Names` (union); `All` wins over `Names`.
- `is_everything`: group, resource, and a verb all `*`. Order: everything rows, then group (`""` first), then resource. At most 300 rows (`hidden_rows` counts the rest). URL rules merge by URL into `url_rows`.
- `can_do_chips`: per row `{verbs} {resource}[.{group}]` (verbs: filled cells in column order then other verbs, `, `; all 8 → `all verbs`; resource `*` → `all resources`); ` (named)` when no cell is `All`; `is_everything` → `everything` Warn. Max 12 chips, `more` = rest.

## Entry points (`workspace.rs`, `resource_actions.rs`, `app_shell.rs`)

| Where | Item | Opens |
|---|---|---|
| Roles, ClusterRoles header | `Who can…` (ClusterRoles: before Hide system) | Who can, empty query, namespace = first scope namespace (`All` → cluster-wide) |
| Role / ClusterRole menu (first item) | `Who can…` | prefilled by `who_can_prefill`; Role namespace, ClusterRole cluster-wide |
| ServiceAccounts header | `Check permissions` | the open drawer's account, else You |
| ServiceAccount menu (first item) | `Check permissions` | that account, its namespace |
| NetworkPolicies header / menu (first item) | `Test traffic` / `Test traffic…` | `traffic_defaults` (menu: destination = first pod the policy selects) |

`AppShell::{open_who_can, open_permissions, open_traffic_test}(…, window, cx)` build the view and call `window.open_dialog`. Buttons are disabled with "Not connected" while there is no live session. SA drawer subject → `session.request_rbac(cx)` (Can do).

## Launch screens (`launch_options.rs`, `screenshot.rs`)

`--screen who-can` (ClusterRoles; query `get secrets`, auto Check), `check-permissions` (ServiceAccounts; You), `account-permissions` (ServiceAccounts, first row after `--filter`, its account), `test-traffic` (NetworkPolicies, first row after `--filter` if any; defaults; auto Check). The capture waits until the dialog result leaves Loading, under the existing first-rows timeout.
