# 0018 · Custom object safety (C1)

[Back to index](README.md) · Steps 1a (matcher), 1b (masking in the cluster crate), 3 (drawer) · Modules: `object_yaml.rs`, `custom_object.rs`, `column_path.rs`, `storage_class.rs`

## Why not mask everything, why not mask nothing

| Option | Verdict |
|---|---|
| Treat every custom object like a Secret (mask all string leaves, as 0017 Helm values) | rejected: hides `secretName`, issuer names, sync status, hosts; kubectl shows these objects to the same user |
| Treat custom objects like ConfigMaps (0007 decision 8: shown) | the base rule: operators keep credentials in Secret objects and reference them |
| Base rule plus heuristics for inline credentials | **chosen**: inline `password:`, `clientSecret:`, `apiKey:`, Secret-like kinds with plain `data`, DSNs with userinfo, `format: password` columns are caught without hiding normal fields |

## Secret-key matcher (`object_yaml.rs`, step 1a; replaces 0014 `is_secret_parameter`)

`pub(crate) fn is_secret_key(key: &str) -> bool`: lowercase the key and drop `-`, `_`, `.`, `/`.

1. Keys ending in `secretname`, `secretnamespace`, `ref`, or `refs` are **never** secret (references: `csi.storage.k8s.io/provisioner-secret-name`, `secretRef`, `passwordSecretRef`, `imagePullSecretRefs`).
2. Otherwise secret when the key contains `secret`, `password`, `passwd`, `token`, `credential`, `accesskey`, `userkey`, `privatekey`, `apikey`, `passphrase`, `bearer`, `clientkey`, `kubeconfig`, or `connectionstring` (`secretkey` and `restuserkey` stay covered; `clientKeySecretRef` and `clientKeyRef` stay readable through rule 1).
3. StorageClass results stay correct: `restuserkey`, `adminPassword`, `access-key` hidden; `kmsKeyId`, `type`, `fsType`, `*-secret-name`, `*-secret-namespace` shown. 0014's tests move with the function unchanged.

## Rules (one private pass `mask_custom_object(object: &mut Value, kind: &str) -> usize` in `object_yaml.rs`, step 1b)

Applied, in this order, to: the YAML of an `ObjectTarget::Custom` object (after 0007 rules 1–4, before sorting), the `object_fields` input (a clone of `spec` and `status`), and printer column values. Every hidden value counts toward the YAML header `# k8sBoard hid {n} values as <hidden>.`

| # | Rule | Scope |
|---|---|---|
| S1 | 0007 rules 1, 2, 4 already apply to any kind: `managedFields` removed, manifest annotations hidden at any depth, `env[].value` literals hidden under `spec` | YAML |
| S2 | **Secret-like kind**: the kind, ASCII-lowercased, contains `secret`, or `is_secret_key(kind)` → every string and number leaf outside `metadata`, `apiVersion`, `kind`, and the **whole `status` subtree** becomes `<hidden>`; keys, booleans, `null`, empty `{}`/`[]` stay (0017 decision 11 shape) | YAML; fields (`spec` side `Text` → `Hidden`); columns whose path does not start with `.status` or `.metadata`, other than `date` columns → `Hidden` |
| S3 | **Secret-like key**: a string or number leaf whose own key passes `is_secret_key` → `<hidden>`. Objects and arrays under any key are walked, not hidden; a scalar in an array counts under the array's key. An env-style `{name, value}` pair whose string `name` passes `is_secret_key` hides its string or number `value`, wherever the pair sits (outside container `env` lists too; columns read the pair through the path's parent) | YAML, fields, columns (`last_field`) |
| S4 | **URL userinfo**: in any remaining string leaf, each `scheme://userinfo@host` has `userinfo` replaced by `<hidden>`; the authority ends at the first `/`, `?`, `#`, or whitespace after `://`, and only an `@` inside it counts (the last one) | YAML, fields, column text |
| S5 | **Password columns**: a printer column with `format: password` reads `Hidden` (decision 36) | columns |

- Leaves under `metadata` are never touched by S2–S4.
- `status` of a Secret-like kind stays readable (conditions, sync state) but S3 and S4 still apply inside it.
- No reveal for custom objects (0007 decision 11). Copy copies masked text.

## What leaves the cluster crate

| Data | Holds |
|---|---|
| `CrdSummary` | names, versions, printer column definitions, schema field names and types, conditions; no instance data |
| `CustomObjectSummary` | metadata, masked typed column values, ≤ 10 conditions (message ≤ 300 chars), `phase` |
| `CustomObjectFields` | masked flattened `spec`/`status` leaves (≤ 60 per side, ≤ 200 chars each) |
| `ObjectYaml` | masked text (no `Debug`, 0007) |

## Rules for code

1. No `tracing::` call in `custom_object.rs`, `column_path.rs`, or `custom_resource_definition.rs` except the shared watch error path (kind and error text only).
2. The raw `DynamicObject`, its `Value`, and the metadata `Value` are dropped inside the summarizer; nothing else keeps them.
3. Serializer and conversion failures use the fixed 0007 message; `UnsupportedPath` carries no text.
4. Condition messages get S4 (URL userinfo) before the 300-character cut, and are otherwise kept as the API reports them (ceiling: a validation error can quote a field value, like 0017's Helm description).
5. Screenshots show masked data only; there is nothing to reveal.

## Review checklist (advisor, steps 1a, 1b, 3)

- [ ] `mask_custom_object` runs for every custom YAML fetch and every `object_fields` call; builtin kinds' YAML is unchanged (existing `object_yaml_tests.rs` pass untouched).
- [ ] Matcher tests: `client_secret_and_api_key_are_hidden` (`clientSecret`, `apiKey`, `passphrase`, `bearerToken`), `secret_refs_stay_readable` (`secretRef`, `passwordSecretRef`, `tokenSecretRefs`, `secretName`), StorageClass examples unchanged.
- [ ] Tests cover S2 (`ClusterSecret` `data` hidden, its `status` readable), S3, S4 (`postgres://u:p@db:5432/x` → `postgres://<hidden>@db:5432/x`; `https://example.com/a@b` unchanged), S5 (`password_format_columns_are_hidden`).
- [ ] `Debug` of a summary built from a fixture holding a distinctive secret value does not contain it (S2, S3, S4, S5).
- [ ] The probe prints counts and names only.
