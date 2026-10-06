# 0013 · HPAs, ResourceQuotas, Namespace Quota (step 3)

[Back to index](README.md) · Modules: `policy_rows.rs` (+ tests), `live_sections.rs`, `kind_diagnosis.rs`, `namespace_rows.rs`, `resource_actions.rs`. Formatting reuses 0010 `Measure` and `format_percent`. W7 `meta` lines and top buttons are not rendered ([app-model.md](app-model.md)).

## HPAs

### Metric text and state (pure, `policy_rows.rs`)

```rust
fn value_text(value: &MetricValue) -> String;            // Utilization "74%", else the quantity as written
fn metric_text(metric: &HpaMetric) -> String;            // cell and bar text parts
fn is_above_target(metric: &HpaMetric) -> Option<bool>;  // None: current unknown or of another variant
fn is_at_max(hpa: &HorizontalPodAutoscalerSummary) -> bool; // condition ScalingLimited true, reason TooManyReplicas
fn is_scaling_disabled(hpa: &HorizontalPodAutoscalerSummary) -> bool; // ScalingActive false, reason ScalingDisabled
```

| Target | `metric_text` | Above target |
|---|---|---|
| `Utilization(t)` | `{name} {c}% / {t}%`; container source: `{name} ({container}) …` | current `Utilization(c)` and `c > t` |
| `AverageValue(t)` / `Value(t)` | `{name} {c} / {t}` (quantities as written) | current of the same variant and `quantity_ratio(c, t) > 1.0` |
| current unknown | `{name} <unknown> / {target}` | `None` |

`is_at_max` uses the controller's own verdict (decision 17): `TooManyReplicas` means the metrics asked for more than `maxReplicas`.

### Columns

| Column | Width | Cell |
|---|---|---|
| Target | 200, grows 1, up to 320 | `Text("{kind lowercased}/{name}")` (0005 owner format); capped so Name keeps the spare width |
| Min / Max | 90 | `Text("{min} / {max}")` |
| Replicas | 80 r | `current`; `Toned(Bad)` when `is_at_max`, `Toned(Warn)` when `current >= max` whatever the conditions say, else `Text` |
| Status | 135 | `Toned(hpa_status)`: `Scaling inactive`, `At max replicas`, `3 replicas`, and the others of the table below |
| Metrics | 170, grows 1, up to 320 | first metric's text, plus ` +{n}` for more; Bad when at max, Warn when above target or unknown, else `Text`; no metrics → `Absent` |
| Age | 70 r | |

### Status (first match)

| Case | Status |
|---|---|
| `is_scaling_disabled` (target scaled to 0 replicas) | Done "Scaled to zero" |
| condition `ScalingActive` false | Bad "Scaling inactive" |
| condition `AbleToScale` false | Bad "Cannot scale" |
| `is_at_max` | Bad "At max replicas" |
| desired > current | Warn "Scaling up" |
| desired < current | Info "Scaling down" |
| otherwise | Ok "{current} replicas" |

### WHY (first match; `kind_diagnosis.rs`; no box when `is_scaling_disabled`)

| Title | When (a false condition's reason) | Text |
|---|---|---|
| METRICS UNAVAILABLE (Bad) | `ScalingActive` or `AbleToScale` false with reason `FailedGet…Metric` (prefix `FailedGet`, suffix `Metric`) or `InvalidMetricSourceType` | `{reason}: {message}` |
| CANNOT SCALE (Bad) | reason `FailedGetScale` or `FailedUpdateScale` | `{reason}: {message}` |
| SCALING INACTIVE (Bad) | `ScalingActive` false, any other reason | `{reason}: {message}` |
| CANNOT SCALE (Bad) | `AbleToScale` false, any other reason | `{reason}: {message}` |
| AT MAX REPLICAS (Bad) | `is_at_max` | `Running {current} of max {max} replicas and the metrics ask for more.` + ` {metric_text} is above target.` for the first metric above target + ` Raise maxReplicas or reduce the load.` |
### Sections

| Section | Rows |
|---|---|
| WHY | above |
| Scaling | Target (`Link` via `of_owner`, else `Text`), Min replicas, Max replicas, Current, Desired, Last scaled (`Age`, or "Never") |
| Metrics | per metric with a known current: `Bar { label, percent, text, tone }`: `Utilization` → label `CPU utilization`/`Memory utilization` for cpu/memory Resource sources, else `{name}`; `percent` = `c` clamped; text `{c}% / target {t}%`. Value targets → `percent = ratio·100` clamped; text `{c} / target {t}`. Tone Bad at max and above, Warn above, else none. Unknown current → `Field { name, "current value unknown" }` Warn |
| Scaling events | `Live(ScalingEvents)` |
| Conditions | 0005 rows with message |
| Labels | |

`ScalingEvents`: object events with reason `SuccessfulRescale`, newest `last_seen` first, max 10; row `Field { label: run_label(last_seen) (0012), value: first message line cut at 120 chars }`. Empty / loading / failed: "No scaling events kept" / "Loading events…" / "Events are unavailable".

## ResourceQuotas

### Items (pure, `policy_rows.rs`)

```rust
enum QuotaMeasure { Cpu, Bytes, Count }
fn quota_measure(resource: &str) -> QuotaMeasure; // ends with "cpu" → Cpu; contains "hugepages", or ends with "memory" or "storage" → Bytes; else Count
fn quota_ratio(item: &QuotaItem) -> Option<f64>;  // quantity_ratio(used, hard); used None → None
fn quota_text(item: &QuotaItem) -> String;        // Cpu/Bytes: Measure::format_pair(used, hard, " / "); Count: "{used} / {hard}"; unparsable: as written
fn quota_tone(ratio: f64) -> Option<StatusTone>;  // decision 16
```

### Columns

| Column | Width | Item | Cell |
|---|---|---|---|
| CPU req | 130 r | `requests.cpu`, else `cpu` | `Quantity { quota_text, permille of ratio, quota_tone }`; no ratio → `Text(quota_text)`; missing → `Absent` |
| Memory req | 150 r | `requests.memory`, else `memory` | same |
| Pods | 100 r | `pods` | same |
| Age | 70 r | | |

Status: the item with the highest ratio: ≥ 1 → Bad "{short} at quota"; ≥ 0.9 → Warn "{pct} {short} used" (`format_percent`); else Ok "Within quota"; no items → Done "No limits". `{short}`: CPU for cpu items, memory for memory items, else the resource name.

WHY **AT QUOTA** (Bad), the fullest item (highest ratio), when its ratio is ≥ 1, so the status and the box name the same item: `{resource} is at its limit ({quota_text}). New objects that need it are rejected; see Blocked creations.`

### Sections

| Section | Rows |
|---|---|
| WHY | above |
| Usage | per item, key order: `Bar { label: resource, percent, text: quota_text, tone: quota_tone }`; no ratio → `Field { resource, quota_text }` |
| Scopes | `Chips(scopes)`; omitted when empty |
| Blocked creations | `Live(BlockedCreations)` |
| Labels | |

`BlockedCreations`: related events whose message contains `exceeded quota: {quota}` followed by `,` or the end, or `failed quota: {quota}:`; newest first, max 20. Row: `Link { label: run_label(last_seen), text: "{kind lowercased}/{name}" of the involved object, target: of_object(..) }` (plain `Field` when there is no screen), then `Note(message cut at 160 chars)`. Empty: "No creations blocked recently"; always ends with the muted note "From FailedCreate events of controllers that the API server still keeps; a pod created directly is rejected without an event".

## Namespace drawer: Quota section

`namespace_row` appends section **Quota** = `Live(NamespaceQuotas)` before Labels. Rows: one `Link { label: quota name, text: "{resource} {quota_text} ({pct})" of the highest-ratio item, target: the quota }` per quota (links carry no tone; the quota drawer has the colored bars); none → "No ResourceQuota"; loading / failed / denied: "Loading quotas…" / "Quotas are unavailable" / "Not permitted: list resourcequotas".

### Implementation notes

- The Metrics cell is Warn when any metric is above target or has no current value (not only the first).
- Event times in Scaling events and Blocked creations use `run_label` in the system time zone.
- A denied related watch is not started: the shell filters the subject with `denied_related_check`, and the section paints "Not permitted: {check}".
- A quota item with no `used` yet reads `— / {hard}` and has no bar (a plain field).
