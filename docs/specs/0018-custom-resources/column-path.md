# 0018 · Printer-column JSONPath subset

[Back to index](README.md) · Step 1a · Module: `crates/cluster/src/column_path.rs` (new, private) + `column_path_tests.rs`. Pure; no I/O.

## API

```rust
/// A compiled printer-column path. Only the subset below parses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ColumnPath { root: PathRoot, steps: Vec<Step> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathRoot { Metadata, Data }   // first field `metadata` → Metadata; anything else → Data
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step { Field(String), Index(usize), Wildcard, Filter { path: Vec<String>, literal: String } }
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UnsupportedPath;   // no payload: the column is simply `—`

impl ColumnPath {
    pub(crate) fn parse(text: &str) -> Result<Self, UnsupportedPath>;
    /// The first value the path reaches, or `None`.
    pub(crate) fn first_match<'a>(&self, metadata: &'a Value, data: &'a Value) -> Option<&'a Value>;
    pub(crate) fn reads_metadata(&self) -> bool;
    /// `…conditions[?(@.type == "X")].status`: the cell is toned.
    pub(crate) fn is_condition_status(&self) -> bool;
    /// The last `Field`, for the secret-key rule.
    pub(crate) fn last_field(&self) -> Option<&str>;
}
/// `column` carries the type and the password format (decision 36).
pub(crate) fn column_value(column: &PrinterColumn, path: &ColumnPath, found: Option<&Value>) -> ColumnValue;
```

`PrinterColumn::new` parses the path once and stores `is_supported` and `is_condition_status`; the objects watch parses again (cheap) to evaluate.

## Grammar (whitespace allowed inside `[…]` and around `==`)

| Form | Example | Step |
|---|---|---|
| `.name` (`[A-Za-z0-9_-]+`); the path starts with `.` | `.spec.issuerRef.name` | `Field` |
| `['key']` or `["key"]` (any chars; `\'` `\"` `\\` escapes) | `.metadata.annotations['a.b/c']` | `Field` |
| `[n]` (non-negative) | `.status.ingress[0].ip` | `Index` |
| `[*]` | `.spec.hosts[*]` | `Wildcard` |
| `[?(@.a.b == "lit")]`; the literal in `"…"` or `'…'` | `[?(@.type == "Ready")]` | `Filter` |

Anything else is `UnsupportedPath`: a path not starting with `.` (`{.spec.x}`, `$.spec.x`, `spec.x`), `.*`, recursive descent `..`, slices `[a:b]`, unions `[a,b]`, negative indexes, `!=`, `<`, `>`, `&&`, `||`, `=~`, integer or boolean literals, functions, `range`/`end`, a filter on `@` itself, an empty path.

## Evaluation (`first_match`)

- The root is `metadata` (`serde_json::to_value(&object.metadata)`, built once per object and only when some column `reads_metadata`) or `data` (`DynamicObject.data`: spec, status, and other top-level fields). `.metadata.x` skips the leading `metadata` field on the metadata root.
- Depth-first over the steps; returns the first value that completes the path, which is what the API server uses for a printer column.
- `Field` reads an object key; `Index` an array item; `Wildcard` iterates array items or object values in order; `Filter` iterates the items of an array (not objects) and keeps those whose `@` path value is a JSON string equal to the literal; a missing `@` path never matches.
- No allocation besides the metadata `Value`; recursion depth is the step count.

## Typing (`column_value`)

| Column | Found value → cell |
|---|---|
| any | `None` → `Absent`; `format == "password"` and a value exists → `Hidden`; `last_field` passes `is_secret_key` and a value exists → `Hidden` |
| `string` | string → `Text` (cut at 200 chars, `…`); number, bool → `Text` of the JSON text; null, array, object → `Absent` (shown `—`; kubectl prints the map) |
| `integer` | JSON integer (`as_i64`) → `Integer`; else `Absent` |
| `number` | JSON number → `Number` (its JSON text); else `Absent` |
| `boolean` | bool → `Boolean`; else `Absent` |
| `date` | string parsing as `jiff::Timestamp` (RFC 3339) → `Date`; else `Absent`. The app paints future dates `in 6d` (kubectl: `<invalid>`) |

Unsupported columns get `Absent` in every summary (the watch skips evaluation for them).

## Coverage examples (fixtures in tests)

| CRD | Path | Result |
|---|---|---|
| cert-manager Certificate | `.status.conditions[?(@.type == "Ready")].status` | `True` (toned) |
| cert-manager Certificate | `.spec.issuerRef.name`, `.metadata.creationTimestamp` | text, date |
| cert-manager Certificate (built-in) | `.status.notAfter` | date (Expires) |
| Argo CD Application | `.status.sync.status`, `.status.health.status` | text |
| Prometheus Operator Prometheus | `.spec.replicas`, `.status.availableReplicas` | integer |
| Strimzi KafkaTopic | `.spec.partitions`, `.status.conditions[?(@.type=="Ready")].status` | integer, toned |
| Flux Kustomization | `.status.conditions[?(@.type=="Ready")].message` | text |
| any | `.spec.containers[*].image` | first image |
| any | `.items..name`, `.spec.ports[0:2]`, `{.spec.x}` | unsupported → `—` |
