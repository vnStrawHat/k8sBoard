# 0018 · Decisions

[Back to index](README.md). Architect defaults, amended after the advisor review (must-fix 1–2, should-fix 3–10, nice-to-haves 11–16). 0005, 0007, 0012–0016 decisions apply unless replaced here.

## Discovery and data

| # | Decision | Rationale |
|---|---|---|
| 1 | Discover through an always-on **CRD watch** (`apiextensions.k8s.io/v1`), not the Discovery API | it carries printer columns, schema, scope, and Established; it updates the sidebar live; Discovery has no printer columns and cannot tell CRDs from aggregated APIs |
| 2 | **Lean CRD decoding**: `kube::core::Object<CrdSpecHead, CrdStatusHead>` with serde structs that read names, scope, versions (served, storage, deprecation, printer columns, two schema levels) and conditions only | no `JSONSchemaProps` tree is built (the typed k8s-openapi CRD builds it whole); metadata and the page body are still fully materialized transiently |
| 3 | CRD watch: `ListSemantic::MostRecent`, **`page_size(20)`** | bounds the transient page body near 20 × the largest CRD |
| 4 | The CRD watch starts after the access review: not started when `ListCustomResourceDefinitions` is `Known` denied; started when allowed, unknown, or failed | a denied watch would retry a 403 for the whole session |
| 5 | **One sidebar item per CRD**, at its preferred served version (`kube::core::Version::priority`) | kubectl's default; all versions are the same objects; the CRD drawer lists every version |
| 6 | Only Established CRDs with a served version become items; the CRDs table shows every CRD | a non-established CRD cannot be listed |
| 7 | Object watch: `DynamicObject`, only while its screen is shown, `MostRecent`, **`page_size(50)`** | unknown object sizes; same bound as 0016 Secrets |
| 8 | The list summary keeps metadata, one typed value per column, at most 10 conditions (message ≤ 300 chars), and `status.phase` | C7 and memory |
| 9 | Overview **Spec and Status fields come from a related watch** of the selected object (0012 infra, field selector `metadata.name`) | live, one object at a time, never in list memory |
| 10 | Summary watch helpers take `F: Fn(&K) -> T + Clone + Send + 'static` instead of `fn(&K) -> T` | custom summaries need runtime columns; `fn` pointers still satisfy the bound |

## Printer columns

| # | Decision | Rationale |
|---|---|---|
| 11 | Own **JSONPath subset** ([column-path.md](column-path.md)): `.field` chains starting with `.`, `['key']`, `[n]`, `[*]`, `[?(@.a.b == "lit")]` (string literal only); **first match only** | the API server uses the first result for printer columns; covers cert-manager, Argo CD, Prometheus Operator, Strimzi, Flux; no dependency. Rejected: server-side `as=Table` (a hand-written watch), JSONPath crates (C6) |
| 12 | Unsupported syntax → `—` in every row and `(unsupported)` in the CRD drawer | never a wrong value |
| 13 | Values typed by the column type; a mismatch is `—`; `string` accepts any scalar, object values show `—` (kubectl prints the map); `date` paints `5d` past and `in 6d` future (kubectl shows `<invalid>` for the future) | matches kubectl for well-formed CRDs; W7 `in 6d` |
| 14 | Every printer column is shown; `priority` is not read; Columns ▾ (0009) hides them | W7 shows Issuer (priority 1) |
| 15 | No printer columns → Name and Age; otherwise Name plus the printer columns (and built-in columns, decision 35) | kubectl behavior |
| 36 | A printer column with **`format: password`** reads `<hidden>` in every row | the CRD author marked it secret |

## App model and navigation

| # | Decision | Rationale |
|---|---|---|
| 16 | `ResourceKind::Custom(CustomKind)`; `CustomKind` wraps a **leaked** `&'static CustomKindSpec`; a `CustomKindCache` reuses every definition seen and **moves from session to session** through `AppShell::start_session` | keeps `ResourceKind: Copy` and every `&'static` accessor; the leak is bounded by distinct definitions ever seen. Rejected: `Rc` (no `Copy`, ~200 call sites), id + registry |
| 17 | `KindSpec.object` and `access_check` merge into `api: KindApi { Builtin { object, access_check }, Custom }` | custom kinds have neither; no dummy values |
| 18 | `CustomKind` equality and hash compare the source (CRD name, resource, columns), with a `ptr::eq` fast path; a changed definition is a new kind and the shown screen is remapped; a removed CRD returns to CRDs | views and keys never mix two column sets |
| 19 | Label = kind plus the plural's suffix (`Certificates`); badge = first letter plus the next capital, else the second letter (`Ce`, `Kt`) | readable, W7 `Ce` |
| 20 | Sidebar: Custom Resources › CRDs, then one **collapsed submenu per API group**, kinds alphabetical; the shown kind's group starts open. No search, no pinning | **accepted deviation** from W7's flat list: 300 CRDs stay scannable; 0029 covers search |
| 21 | **Per-resource SSAR, `list` only** (per namespace for `Several`) on the first navigation per scope, cached for the scope; the watch starts after Allowed or a failed review; Denied → table text and sidebar lock | consistent with the built-in kinds' `list` checks; no 403 retry loop |

## Status, drawers, safety

| # | Decision | Rationale |
|---|---|---|
| 22 | Row status: `Ready`, else `Available`; a failing condition raises Warn; else `status.phase` (Info); else Info `No status` | the common operator convention; W7 "Renewal failing" |
| 23 | A string column whose path filters `conditions` and ends in `.status` is a toned cell | W7 `@ok True` |
| 24 | Tabs Overview · YAML · Events; Overview built at render (C7): box, Conditions, Status, Spec, Labels | W7 Certificates drawer |
| 25 | Fields: depth ≤ 3, scalar arrays joined (5, then `+n`), object arrays `n items`, deeper `n fields`, 60 per side, 200 chars | bounded; the YAML tab has the rest |
| 26 | A string field whose key is `secretName` links to the Secrets row of the same namespace | generic W7 "Go to secret" |
| 27 | The object drawer shows values, not schema descriptions; the outline lives in the CRD drawer | accepted deviation: values are what triage needs |
| 28 | C1: custom objects are **non-secret like ConfigMaps** plus heuristics ([custom-object-safety.md](custom-object-safety.md)); no reveal | kubectl shows them; inline credentials are the exception |
| 29 | The CRDs explorer list is **fed from the session's CRD watch** | no second multi-MB download |
| 30 | CRDs Name column is the full CRD name; Group column kept | **accepted deviation** (W7 shows the plural): the row name is the object name (YAML, events) |
| 31 | Instances = one-shot **cluster-wide** count per CRD (one `limit=1` list, whatever the scope) when CRDs is shown, ≥ 30 s apart, 4 at a time; also fill custom sidebar counts | C11; one request per CRD |
| 32 | STUCK = Terminating longer than 5 min; Remaining resources parsed from deletion conditions | the API server already names remaining types and finalizers |
| 33 | Launch `--screen custom:<crd-name>[-drawer|-events|-yaml]`, resolved once the CRD list is ready | custom kinds are unknown at parse time |
| 34 | Watch bound `3N + 5` (the CRD watch adds one) | 0012's bound plus one always-on watch |
| 35 | **`BUILT_IN_COLUMNS`** (`custom_kind.rs`): `certificates.cert-manager.io` gains **Expires** (`date`, `.status.notAfter`) before Age; Warn within 14 days, Bad when expired (0016 `expiry_label` thresholds) | coordinator decision: follow W7's Expires column; cert-manager has no NotAfter printer column |
| 37 | A string printer column whose name contains Status, Ready, Health, Sync, or Phase reads its value through a fixed table, ASCII case-insensitive and exact. Ok: Healthy, Synced, True, Ready, Running, Succeeded, Completed, Bound, Available. Bad: Degraded, Failed, Error, False, Missing. Warn: OutOfSync, Progressing, Pending, Unknown, Suspended, Terminating. Any other value has no tone | Argo CD and Strimzi states read at a glance (ui-verifier M1); an unfamiliar word is never painted as good or bad. The row status of a phase uses the same table (Info for other words) |
| 38 | An object with no `status.phase` and no `Ready`/`Available` condition uses a top-level string `status.status` as its `phase` (ClickHouse style), masked and capped like the phase | the drawer subtitle agrees with a table that shows `Completed` (ui-verifier M2) |
| 39 | Namespace deletion-condition messages are masked for URL userinfo (S4) before the 500-character cut; a Remaining resources section whose entries are all unparsed shows the "no remaining content" note, because the STUCK box already shows the message | a discovery failure can quote an aggregated API URL; no message twice (it also drops them during the first 5 minutes, before the box appears) |

## Ceilings

- Leak: one `CustomKindSpec` (~1 KiB) per distinct CRD definition ever seen in the process. `ponytail:` comment; upgrade path: evict definitions no session uses.
- The CRD watch downloads every CRD once per session; Crossplane-scale clusters (1,000+ CRDs) can transfer 100+ MiB. Upgrade path: start on first use, or Discovery plus one GET per opened CRD.
- First-match JSONPath: a column meant to join several values shows the first.
- The failing-condition rule matches `fail`/`error` substrings, so a reason such as `NoErrors` or `ErrorsCleared` reads as failing.
- Events match custom objects by `kind` only; a custom kind named like a builtin (`Service` in another group) shares event matches, and `from_object_kind` routes its Go to object to the builtin screen.
- Cluster-wide counts need cluster-wide `list`; a namespace-limited user gets no Instances.
- Masking heuristics miss credentials under innocuous keys (`config: "user=a password=b"`).
- `is_secret_kind` over-hides reference-only kinds such as `ExternalSecret` and `SecretStore`: every scalar outside `status` reads `<hidden>`, including the names and keys they only reference.
- `is_secret_key` false negatives remain: keys such as `auth`, `webhookUrl`, `signingKey`, `encryptionKey`, `sshKey`, `hmacKey`; DSNs without a scheme; credentials in `?password=` query strings (S4 hides userinfo only).

## UAT probe (`--crds`, filled by coder-lite in 1a and 1b)

| Item | Result |
|---|---|
| `list customresourcedefinitions` | allowed (access line; `count customresourcedefinitions` 72) |
| CRDs / Established | 72 / 72 (every CRD has a served version) |
| Unsupported printer columns | 0 of 183 columns in 72 CRDs (names only; none to list) |
| First CRD with instances: access, count, watch, yaml | `applications.argoproj.io`: access allowed, count 10, watch 10 objects with 31 of 40 column values filled, yaml 253 lines, masked yes. The first ten CRDs were all listable (10 allowed); counts 10, 0, 1, 1, 0, 0, 4, 0, 7, 0 |
| Namespaces STUCK (step 5) | `coroot` was Terminating for 3 days with a `NamespaceDeletionDiscoveryFailure` condition (stale `external.metrics.k8s.io/v1beta1`): the Bad STUCK box and the Remaining resources note render live (`v43-namespaces-stuck-*.png`); no remaining resource or finalizer was reported, so those parse paths are fixture-tested |
| Certificates (cert-manager) | not installed on UAT (72 CRDs, none of cert-manager): the worked example is covered by fixture tests, and Argo CD Applications exercise the same custom path live |
