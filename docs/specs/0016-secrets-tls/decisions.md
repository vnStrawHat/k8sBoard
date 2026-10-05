# 0016 · Decisions

[Back to index](README.md). Architect defaults; the user asked not to stop for questions; amended after the security review (HEAD `6de733c`, items M1–M3, S1–S6, N2–N7). 0005, 0007, 0012–0015 decisions apply unless replaced here.

## Data and fetching

| # | Decision | Rationale |
|---|---|---|
| 1 | The Secrets table uses a **full typed watch, summarized at once** (`watch_secrets`) | W7 needs Type and Keys; C1 allows summaries with names, sizes, type. **Rejected**: kube 4.2 `metadata_watcher` (no type or keys, and metadata still carries `last-applied-configuration`, which embeds the base64 `data` of kubectl-applied Secrets); a custom deserializer that skips `data` values (~150 lines of serde visitor, and it must still read `tls.crt` and `.dockerconfigjson`, so values pass through it anyway); server-side Table format (not supported by the kube watcher) |
| 2 | Summaries keep **type, key names, sizes, binary flag**, and type details (9–12). No value, no annotation except decision 12 | C1; same rule as `ConfigMapSummary` |
| 3 | Opening a drawer makes **no** request: Data is built from the summary. Values come from **one GET per Reveal or Copy**, never cached | C1 "fetched only on demand" |
| 4 | Reveal is **per key** or **all keys of the open drawer**; each value hides 30 s after it was shown. W7's list-level "Reveal all" is not rendered | C1: per drawer, not per list |
| 5 | **Copy works without reveal**, for text values only; binary values: Copy disabled ("Binary value") | copying base64 needs a new direct dependency and a mode; binary keys are rare |
| 6 | Owned plaintext lives in `Zeroizing` (`SecretValue`, the GET body) | `zeroize` is already locked; [secret-safety.md](secret-safety.md) states the ceiling |
| 7 | Reveal and Copy are gated by the **GET result**, not an SSAR: 403 → "Not permitted: get secrets" inline | same as the YAML tab (0007) |
| 8 | A Reveal or Copy fetches all keys and drops the unrequested ones at once | the API has no per-key read |
| 29 | `secret_values` builds the GET with `Request::get` and decodes `request_text` itself; never `Api::get` or `Client::request` (M1b); its call site is a named exception in the 0030 clippy table (`secret.rs`, write-path.md) | kube-client 4.2 logs the whole body at `warn` on a decode failure; owning the text also lets us wipe it |
| 30 | The app adds the fixed directive `kube_client::client=error` after the env filter (M1a) | the same log path serves every `Api::list` and watcher initial list, Secrets included; `RUST_LOG` must not reopen it |
| 25 | Both secret watches use `ListSemantic::MostRecent` with `page_size(50)` from step 1 (M3) | the watch-cache list (`resourceVersion=0`) ignores `limit` and returns every Secret in scope in one body; paging caps transient plaintext and peak memory at 50 objects |
| 26 | **Copy is private and auto-cleared** (0043 adds no setting for the 30 s: it stays fixed) (M2): Windows write adds `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory=0`, `CanUploadToCloudClipboard=0` in the same clipboard session; the clipboard is cleared after 30 s if it still holds our value (keyed hash), on by default | coordinator decision; settles the C1 open question; [secret-clipboard.md](secret-clipboard.md); macOS/Linux managers stay a ceiling |

## Type details

| # | Decision | Rationale |
|---|---|---|
| 9 | TLS: `tls.crt` is parsed **inside the summarizer**; only subject, issuer, SANs, validity are kept; `tls.key` is never read there | certificates are public; one parse serves table, drawer, Ingresses |
| 10 | Parser: **`x509-cert` 0.3** (`default-features = false`) plus **`pem` 3** (locked); chain cut at 10 | maintained pure-Rust parser for untrusted input; 5 small new packages. Rejected: `x509-parser` (nom, asn1-rs, more packages), a hand DER walker, `rustls-webpki` (no field accessors) |
| 11 | Docker types: **registry hosts only** | hosts are not secret; `auth`, `password` never copied |
| 12 | Token secrets: account from annotation **`kubernetes.io/service-account.name`** (second allowlisted annotation after 0014 decision 4); `token` masked like any value | the only link; it holds a name |
| 13 | Expiry is the **leaf's not-after** everywhere; an intermediate that expires earlier gets a separate Warn field (S2). Warn at ≤ 14 days, Bad when expired, Warn when not yet valid | the leaf date is what operators renew; an early intermediate is real but different news |
| 14 | The X.509 issuer DN is shown; cert-manager facts (issuer resource, "has not renewed") are not | no annotations; needs CRDs (0018) |
| 24 | Parse with the **default RFC 5280 profile**; a failure reads "could not be parsed" (S4), not "unreadable" or "invalid" | the `hazmat` Raw profile is outside x509-cert's semver promise; strict-profile rejections (e.g. serials over 20 octets) are rare in current CAs; the text no longer claims the certificate is broken |
| 27 | `SecretDetails::{Certificate { chain }, NoCertificate(issue)}` instead of `Tls(Result<..>)` (N5) | a non-empty chain is a type guarantee; readers match once |

## App

| # | Decision | Rationale |
|---|---|---|
| 15 | **Used by** = pods (env, env from, volume, projected, image pull) + ingresses (`tls`) + the token's account; the Secrets screen runs an **Ingresses companion** | TLS users are ingresses, not pods |
| 16 | A secret nobody uses reads `none found` (muted), the same as a ConfigMap, once pods and the companion are Ready; the wording claims only what was checked (UX fix: "unused" overclaimed). `may_be_unused` and its eligibility rules are gone | Gateway API, Istio, CronJob templates, and cert-manager reference secrets without a running pod; the drawer note says so |
| 17 | Ingress **TLS column replaces Ports** (W7), fed by a TLS-secrets companion | the column needs every referenced certificate |
| 18 | Expiry text and tone are **computed at paint time** | days left change while the screen is open |
| 19 | The Secrets **Age cell is not toned** (W7 `@warn 84d`); the Secrets list has **no expiry signal until 0020/0021** (N4) | a build-time tone goes stale |
| 20 | Masked values show a fixed `••••••••••` plus the size | leaks nothing beyond the size |
| 21 | Revealed text shows at most **4 KiB** | large values stall layout |
| 22 | **One screenshot gate**: `value_access(&LaunchOptions)` sets `AppShell.secret_value_access` once; the view and the menus read only that field (S6) | two checks could drift |
| 23 | No W7 `meta` line, no list buttons (0013 convention); menu: Reveal values (30s), Copy value ▸, Edit (disabled), View YAML, Delete secret… (disabled) | Reveal/Copy are reads |
| 28 | Helm release secrets are plain rows here; **0017 owns their decoding** (N7) | scope |

## Ceilings

- Watch plaintext **transits** memory (≤ 50 objects per page): every Secret in scope on the Secrets screen, every TLS private key in scope on the Ingresses screen; transport buffers are freed, not wiped.
- "unused" checks running pods and ingresses in scope only (drawer note says so).
- The TLS companion's field selector skips Opaque secrets holding `tls.crt`; an ingress naming one reads "no TLS secret" (Warn).
- `MostRecent` lists are quorum reads from etcd: heavier on the API server than cache reads, once per screen open.

## UAT probe (`--secrets`, filled in step 1 by the coder)

| Item | Result |
|---|---|
| `list secrets` / watch line | allowed (0005 probe) / `watch secrets: 1 snapshots, last 42 items, 0 failures`; `watch tls secrets`: last 8 items, 0 failures |
| `get secrets` (`secret values` line) | allowed: `secret values argocd/argocd-tls: 2 keys, 8454 bytes` (counts only) |
| Secrets by type; TLS parsed / earliest leaf not-after | 42 secrets: Opaque 33, kubernetes.io/tls 8, service-account-token 1; no docker or Helm types. TLS 8/8 parsed, earliest leaf not-after 2026-12-26T23:59:59Z (argocd/argocd-tls) |
| First TLS secret (screenshot filter) | `argocd/argocd-tls` (metadata only) |

## Cargo.lock (step 1)

Six packages were added: `x509-cert` 0.3.0, `der` 0.8.2, `der_derive` 0.8.0, `spki` 0.8.0, `flagset` 0.4.7, and `base64ct` 1.8.3. `base64ct` is a weak optional dependency of `spki` (feature `base64`): Cargo records it in the lock file although it is never compiled here (`cargo tree -i base64ct` finds nothing; cargo issue #10801). Accepted by the security review.

## AC 7 live check

The live Reveal and Copy check (reveal hides after 30 s and on drawer close; Copy fills the clipboard, stays out of Win+V history, and is cleared after 30 s) is a **manual step for the user** ([secret-safety.md](secret-safety.md)). Agents never touch the real clipboard and never reveal a real UAT value: tests use fixtures and a fake `ClipboardPort`, and screenshot runs block Reveal and Copy.
