# 0018 · App model: custom kinds, session, sidebar, launch

[Back to index](README.md) · Steps 2a (`KindApi`, Crds kind, session CRD watch) and 2b (everything custom) · Modules: `custom_kind.rs` (new, 2b) + `custom_kind_tests.rs`, `resource_kind.rs`, `cluster_session.rs` (+ tests), `app_shell.rs`, `navigation.rs`, `launch_options.rs` (+ tests), `screenshot.rs`, `yaml_view.rs`, `kind_table.rs`

## `CustomKind` (`custom_kind.rs`, step 2b, decisions 16, 18, 35)

```rust
/// A served custom resource kind. `PartialEq` checks `ptr::eq` first, then the source; `Hash` hashes the source.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CustomKind(&'static CustomKindSpec);
pub(crate) struct CustomKindSpec { source: CustomKindSource, rules: Vec<ColumnRule>, spec: KindSpec } // manual PartialEq/Eq/Hash
#[derive(Clone, PartialEq, Eq, Hash)]
struct CustomKindSource { crd_name: String, resource: CustomResourceType, printer_columns: Vec<PrinterColumn> } // built-ins included
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColumnRule { Plain, ConditionStatus, Expiry }   // one per printer column
impl CustomKind {
    pub(crate) fn crd_name(self) -> &'static str;
    pub(crate) fn resource(self) -> &'static CustomResourceType;
    pub(crate) fn printer_columns(self) -> &'static [PrinterColumn];   // passed to the objects watch
    pub(crate) fn column_rules(self) -> &'static [ColumnRule];
    pub(crate) fn spec(self) -> &'static KindSpec;
}
impl fmt::Debug for CustomKind { /* `CustomKind(certificates.cert-manager.io)` */ }
/// Every definition ever built, keyed by source. Owned by the session, moved to the next one.
#[derive(Default)] pub(crate) struct CustomKindCache { kinds: HashMap<CustomKindSource, CustomKind> }
/// Established CRDs with a preferred served version, sorted by (group, label). A cached source
/// is reused; only definitions never seen before are leaked.
pub(crate) fn custom_kinds(crds: &[CrdSummary], cache: &mut CustomKindCache) -> Vec<CustomKind>;
pub(crate) fn kind_label(kind: &str, plural: &str) -> String;   // decision 19
pub(crate) fn kind_badge(kind: &str) -> String;                 // decision 19
struct BuiltInColumn { crd_name: &'static str, name: &'static str, column_type: ColumnType, json_path: &'static str, rule: ColumnRule }
static BUILT_IN_COLUMNS: [BuiltInColumn; 1] = [BuiltInColumn { crd_name: "certificates.cert-manager.io",
    name: "Expires", column_type: ColumnType::Date, json_path: ".status.notAfter", rule: ColumnRule::Expiry }];
```

- Leaking: `Box::leak` for the spec, `String::leak` for derived texts, `Vec::leak` for columns; one `// ponytail:` comment: bounded by distinct definitions ever seen; evict unused definitions if it grows.
- **Cache lifetime** (decision 16): `ClusterSession::new` takes a `CustomKindCache`; `AppShell::start_session` moves it out of the old session (`ClusterSession::take_custom_kind_cache`) into the new one, so a context switch reuses every definition already seen. The first session gets `CustomKindCache::default()`.
- **Built-in columns**: for a CRD whose name matches, each entry becomes `PrinterColumn::new(name, column_type, json_path, false)`, inserted before the column named `Age` of type `date`, else appended. A built-in column whose name already exists is skipped.
- `ColumnRule`: built-in entry → its `rule`; `is_condition_status` → `ConditionStatus`; else `Plain`.
- `kind_label`: `lower = kind.to_ascii_lowercase()`; plural starts with `lower` → `kind + suffix` (`Certificate`+`s`, `Ingress`+`es`); `lower` ends in `y` and plural is `lower[..-1] + "ies"` → `kind[..-1] + "ies"`; else the plural with its first letter uppercased.
- `kind_badge`: first char, then the next ASCII uppercase char lowercased, else the second char (`Certificate` → `Ce`, `KafkaTopic` → `Kt`, `X` → `X`).
- Spec: `label` = `kind_label`, `singular`, `plural`, `badge`, `is_namespaced` from scope, `NameColumn::Flexible`, `has_labels: true`, `read_only_actions: &[]`, `delete_label` = `Delete {singular}…`, no port-forward, `api: KindApi::Custom`.
- Columns per printer column: named `Age` with type `date` → `AGE_COLUMN`; `date` 90 r; `integer`/`number` 90 r; `boolean` 80; condition-status 80; `string` 160. No printer columns → `[AGE_COLUMN]` (decision 15).

## `ResourceKind` (`resource_kind.rs`; `KindApi`, `Crds` in 2a, `Custom` in 2b)

```rust
pub(crate) enum ResourceKind { /* builtins, 0013–0017 kinds … */ Crds, Custom(CustomKind) }
pub(crate) struct KindSpec { /* today's fields, now pub(crate), minus object and access_check */ pub(crate) api: KindApi }
#[derive(Clone, Copy)]
pub(crate) enum KindApi { Builtin { object: ObjectKind, access_check: AccessCheck }, Custom }
impl ResourceKind {
    pub(crate) fn access_check(self) -> Option<AccessCheck>;        // None for Custom
    pub(crate) fn builtin_object(self) -> Option<ObjectKind>;      // replaces object(); None for Custom
    pub(crate) fn object_kind(self) -> &'static str;               // Custom → resource().kind
    pub(crate) fn object_ref(self, namespace: Option<String>, name: String) -> Option<ObjectRef>;
    pub(crate) fn custom(self) -> Option<CustomKind>;
    /// `None` for Crds: its list is fed by the session's CRD watch (decision 29).
    pub(crate) fn watch_rows(self, …) -> Option<BoxStream<'static, WatchUpdate<KindRow>>>;
}
```

- `CRDS` static: label `CRDs`, object `ObjectKind::CustomResourceDefinition`, access `ListCustomResourceDefinitions`, singular/plural `customresourcedefinition`/`customresourcedefinitions`, badge `Cd`, cluster-scoped, `has_labels: true`, delete `Delete CRD…`; columns in [tables-and-drawers.md](tables-and-drawers.md). Appended last to `ALL`. Custom kinds are never in `ALL`.
- Every builtin static: `object: X, access_check: Y` → `api: KindApi::Builtin { object: X, access_check: Y }`.
- `Custom` arm of `watch_rows`: `watch_custom_objects(custom.resource(), custom.printer_columns(), scope)` mapped by `rows(update, |summary| custom_object_row(custom, summary))`; `rows` takes `impl Fn(&T) -> KindRow`.
- Callers: `yaml_view::object_ref` → `kind.object_ref(..)`; navigation, menus, and 0012 counts use `access_check()` as `Option` (custom kinds: see the gate); counts use `builtin_object()`.

## Session (`cluster_session.rs`; CRD watch and Crds feed 2a, gate, cache, remap 2b)

```rust
pub(crate) struct CrdWatch { pub(crate) list: LiveList<CrdSummary>, pub(crate) kinds: Vec<CustomKind>, _subscription: WatchSubscription }
pub(crate) enum CustomGate { Checking { _task: Task<()> }, Allowed, Denied { reason: String } }
// LiveCluster gains:
pub(crate) crds: Option<CrdWatch>,                     // decision 4 (2a; `kinds` added in 2b)
// ClusterSession gains `custom_kind_cache: CustomKindCache` (2b) and `take_custom_kind_cache(&mut self)`
pub(crate) custom_gates: HashMap<CustomKind, CustomGate>, // cleared on scope change (decision 21)
// KindList: `_subscription: Option<WatchSubscription>`; `None` for Crds and gated custom lists
```

- **CRD watch start**: in `finish_access_review` (and on a failed review) when `crds` is `None` and the report does not deny `ListCustomResourceDefinitions`. Never restarted by a scope change (cluster-scoped).
- **CRD snapshot**: apply; (2b) `kinds = custom_kinds(items, &mut self.custom_kind_cache)`; when the explorer is `Crds`, its list becomes `Ready` with `crd_row` per item (failures pass through); one notify. Problems: the CRD list counts in `any_list_has_problem` once started.
- **Custom explorer** (`set_explorer_kind(Some(Custom(k)))`): gate `None` → insert `Checking`, spawn `review_custom_access(k.resource(), &scope)` (`list` only, decision 21) on the cluster runtime, list `Loading`, no subscription. On completion (explorer still `k`): `Allowed` → `KindList::start`; `Err(_)` → start too, nothing cached; `Denied` → cache, list `Failed { message: custom_denied_reason(k, &scope) }`. Gate `Allowed` → start at once; `Denied` → the failed list at once.
- `custom_denied_reason`: `Not permitted: list {plural}.{group}`, plus ` in all namespaces` or ` in {names}` for namespaced kinds, like `kind_availability`.
- **Remap** (`AppShell::follow_custom_kinds`, called from the session observer, over a pure `remapped_screen(screen, kinds) -> Option<Screen>`): the shown `Kind(Custom(old))` missing from `kinds` → the kind with the same `crd_name` (`show_screen`), else `show_screen(Kind(Crds))`.
- **Watches** (`OpenWatches`): `crds: bool` adds 1; the explorer counts 0 without a subscription, else N for a namespaced custom kind and 1 for a cluster-scoped one. Bound `3N + 5`; the `open_watch_count` test asserts it at N = 5.

## Sidebar (`navigation.rs`, 2b)

- `item` splits into `screen_item(name: &'static str, screen: Option<Screen>, …)`; `item(name)` = `screen_item(name, screen_of(name), …)`. "CRDs" resolves through `from_label`.
- The Custom Resources section's children: its static items, then one submenu per API group of `live.crds.kinds` (in order): `SidebarMenuItem::new(group).click_to_toggle(true).default_open(group of the shown kind)`, children `screen_item(kind.label(), Some(Screen::Kind(Custom(kind))), …)`.
- `kind_availability(Custom(k))`: gate `Denied { reason }` → `Denied`; else `Enabled`. `Crds` uses its access check as today.
- No CRD watch, a `Loading` or failed CRD list → no group submenus.

## Launch and screenshots (`launch_options.rs`, `screenshot.rs`; Crds slugs 2a, `custom:` 2b)

- `--screen customresourcedefinitions[-drawer|-events|-yaml]` works through `from_plural`.
- New `LaunchScreen::Custom { crd_name: String, tab: Option<DrawerTab> }` from `custom:<crd-name>` with an optional `-drawer|-events|-yaml` suffix (stripped first). Resolved when `crds.list` is `Ready`: the kind with that `crd_name` → `show_screen`, then the first row as `KindDrawer` does; none → the run fails with `no Established CRD named {name}`.
- Settle: `is_loading` also waits for the CRD list (Crds and custom targets), the custom gate, and the fields related watch.
- Screenshot slugs: `crds`, `crds-drawer`, `custom`, `custom-drawer`, `custom-yaml` (custom targets from the probe's first Established CRD, recorded in [decisions.md](decisions.md)).
