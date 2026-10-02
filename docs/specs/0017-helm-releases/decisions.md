# 0017 · Decisions

[Back to index](README.md). Architect defaults, amended after the advisor review (HEAD `ef69712`: M1–M3, S1–S8, nice-to-haves). 0005, 0007, 0012–0016 decisions apply unless replaced here.

## Data and fetching

| # | Decision | Rationale |
|---|---|---|
| 1 | The table watches **current revisions only**: typed Secret watch, labels `owner=helm,status!=superseded`, field `type=helm.sh/release.v1`. Every status shows (deployed, failed, pending-*, uninstalling, uninstalled, unknown), like **`helm list --all`** (S4) | labels carry name, version, status but not the chart; superseded revisions (up to 10 per release by default) are most of the bytes. A label change to `superseded` arrives as a watch DELETED event |
| 2 | The summarizer **decodes each current revision once** (base64 → gzip → JSON) and keeps only chart name, chart version, app version, `info.last_deployed`, `info.description` | W7 needs Chart and App version in the table. Rejected: metadata watch plus one GET per release (same bytes, a fetch queue and a cache) |
| 3 | One decode path for summary and detail: `release_json` returns the JSON in a wiped buffer; serde-derived structs read only the needed fields and drop the buffer at once | one function to test. Derive (N1): serde skips `config`, `manifest`, `chart.templates` without building them, and a typed struct fails loudly on a changed shape; a `Value` would allocate the whole release |
| 4 | Release watch: `ListSemantic::MostRecent` with **`page_size(10)`** | a release Secret can be ~1 MiB; 10 per page caps transient plaintext near 10 MiB (0016 decision 25 uses 50) |
| 5 | **History** = a related watch (0012 pattern) of **metadata only** (`metadata_watcher`), labels `owner=helm,name={release}` | revision, status, `modifiedAt`, creation time are labels and metadata; no payload is downloaded; Helm-created Secrets carry no `last-applied` annotation |
| 6 | Values, manifest, notes, and diffs come from **GETs on demand** (`request_text`, decoded in the crate), only while a drawer of that release is open | C1 "fetched only on demand"; C7 |
| 7 | Rows group revisions by `(namespace, name)`: the row is the highest revision; `deployed_revision` names an older revision still `deployed` | a failed upgrade keeps the previous revision deployed |
| 8 | A Secret without valid `name` and `version` labels is skipped; a payload that does not decode gives a row with `chart: None` | labels are the identity; a bad payload must not hide a release |
| 9 | Decompressed size limit **64 MiB** (`RELEASE_SIZE_LIMIT`); the output buffer is reserved once from the gzip **ISIZE** trailer, capped at the limit (S2) | gzip-bomb guard; one allocation, so no outgrown buffer is freed unwiped |
| 10 | Plain-JSON payloads (no gzip magic `1f 8b`) are accepted | Helm's decoder accepts both |

## Masking and display (C1)

| # | Decision | Rationale |
|---|---|---|
| 11 | Values are **secret-capable**: in masked YAML every **string and number** leaf reads `<hidden>`; keys, booleans, `null`, empty `{}`/`[]` stay. The masked Document is a **structure view**; **Reveal is the normal path** to read values (S7) | charts take passwords and tokens as values; structure and switches stay readable |
| 12 | **Reveal (30s)** is per view: user values, computed values, notes, and the diff together; hidden at the first revealed arrival + 30 s; dropped on any tab or subject change | C1 "Reveal all is per drawer"; per-key reveal in a code editor is not feasible |
| 13 | Manifest: per YAML document, 0007 rules 2–4; parsed with a raised budget (S3: `max_nodes` 2,000,000, `max_events` 8,000,000, `max_depth` 128); **never revealed** | 0007 decision 11; the Secrets kind is where values are revealed |
| 14 | **Notes are masked** (M1): the tab shows "Notes may contain rendered passwords." and the line count; text only through Reveal (one GET, `HelmRevealed`) | charts render passwords into NOTES.txt |
| 15 | **Computed values** = Helm `CoalesceValues(chart.values, config)`: maps merge recursively, user wins, a user `null` removes the key, arrays and scalars replace | equals `helm get values --all` for a stored release |
| 16 | **Values diff** = a path diff over flattened JSON leaves, masked like decision 11. Cost (S8): **2 GETs** for the masked diff, **2 more** on Reveal | a changed secret still shows as changed (`<hidden> → <hidden>`); a line diff of masked YAML would hide it. No `similar` (C6 row stays for 0031) |
| 17 | Diff compares a revision with the **highest earlier revision in History** | W7 "Values changed in rev 4"; covers pruned history |
| 18 | Diff list at most **500** changes, values cut at **200** chars | large charts change hundreds of defaults on upgrade |
| 19 | One screenshot gate: `AppShell.secret_value_access` (0016 decision 22) disables every Helm Reveal, so values and notes stay masked | one source |
| 26 | **Revealed copies are private** (M2): while revealed, the editor's Copy and Cut go through 0016 `write_private_text` and arm the 30 s clear; no clipboard → nothing copied (fail closed). Revealed diff and notes are not selectable | C1; same clipboard rules as Secrets |

## App

| # | Decision | Rationale |
|---|---|---|
| 20 | Tabs **Overview · Values · Manifest · Notes**; no YAML (masked blob), no Events | the wireframe's "View values" and "View manifest" need homes |
| 21 | Overview shows W7's **"Values changed in rev N"** (latest vs the previous revision, user values, masked, with Reveal) **and** History rows keep a **Diff** button (S5, coordinator decision). Rendering is a **path list, not a line diff**: accepted deviation from W7's `- data / + old-feature-x` | follows the wireframe; costs 2 GETs per drawer open after the 250 ms debounce |
| 22 | `KindSpec.object = ObjectKind::Secret`; `object_ref` and `event_subject` return `None` for Releases; `ALL` lists Releases after Secrets | the row key is the release; `from_object_kind("Secret")` keeps finding Secrets first |
| 23 | **No one-shot sidebar count** (C11) for Releases | a `limit=1` count counts revisions, not releases |
| 24 | Roll back…, Uninstall release… disabled "Read-only mode"; no list-level Rollback; no Roll back on history rows | C12/0038; 0013 convention |
| 25 | `drawer.helm_revision` resets on subject change; Overview always uses the latest revision | a revision belongs to one release |
| 27 | `HelmReleaseDetail.chart` is kept | the shown revision can be older than the row; its header names that revision's chart version |
| 28 | Status box title from the description prefix (S1): `Upgrade "` → UPGRADE FAILED, `Rollback "` → ROLLBACK FAILED, else INSTALL FAILED | Helm writes these prefixes; install failures have several texts |

## Ceilings

- The release watch downloads every current revision's payload (≤ 10 per page), freed not wiped (0016 ceiling). **Failed revisions stay current** (Helm never supersedes them), so each one up to the history limit is downloaded again at every screen open (S4).
- A decompressed release can reach 64 MiB in memory during one decode. serde builds `Option<String>` fields (manifest, notes) through scratch buffers that are freed, not wiped (S2); a wrong ISIZE makes the buffer grow and free unwiped copies.
- The manifest parse budget is `max_nodes` 2,000,000 (decision 13): a document that large builds a transient tree of about 100-200 MiB before it is masked, freed not wiped.
- `# Source: ` lines of the manifest are kept verbatim. Helm writes them from chart paths, but a template could echo a value there.
- The gzip output reservation is also bounded by 1032 times the compressed length (the DEFLATE maximum ratio), so a lying ISIZE cannot make a tiny Secret reserve 64 MiB on every watch event.
- Masked YAML still shows key names and **array lengths** (e.g. how many hosts or users) (S7).
- Values masked in Values can appear in rendered non-Secret manifests (ConfigMap data, args).
- `info.description` is Helm's text; a Kubernetes validation error inside it can quote a field value (helm-safety rule 1).
- Revealed text also lives in the editor rope and GPUI caches (not wiped); Copy and Cut are private, but other ways out of the window (screen readers, OS screenshots) are not controlled.
- Large manifests (several MiB) may render slowly in the code editor.

## UAT probe (`--helm`, filled by coder-lite in step 1)

| Item | Result |
|---|---|
| Release Secrets found / decoded | 0 / 0 (`helm releases 0`, `payload decoded 0/0`) |
| Status mix | none |
| First release (screenshot filter) | none: the UAT cluster has no `helm.sh/release.v1` Secret, so screenshots cover the empty state only |
| `get secrets` on a release revision | not probed: no release exists; the GET path is `secret_text`, shared with 0016 `secret_values` (live-proven there) |

## As built (app steps)

- The **Release** section is built statically in `helm_rows.rs` (a `Live(HelmRelease)` placeholder had nothing to compute at paint time); `LiveContent` has `HelmHistory` and `HelmValuesChange` only.
- A release row has `created_at: None`: the Updated cell and field carry the last deploy, so the subtitle does not say "created".
- The Overview section's static title is `Values changed`; the drawer skips its heading and the view draws `Values changed in rev N` itself, because section titles are `&'static str`.
- The History model and its tests live in `helm_rows.rs`; `can_diff` is false only for the oldest revision of a history that is not cut at 50.
- `open_helm_values` always stores the chosen revision (`helm_revision`), even when it equals the latest; the view's Latest button compares revisions itself.
- `view_tab_item` serves View YAML, View values, and View manifest; Releases get no View YAML item (`object_ref` is `None`).
- The History has three states in the view (`HistoryState`: loading, failed, loaded). A failed History reads "The history could not be read.", disables Diff with that reason, and counts as settled for screenshots. A failed Reveal fetch still starts the 30 s countdown, so the ticker drops the Reveal and its alert. The History row of the shown revision is marked "shown" also when it is shown implicitly (the latest).
