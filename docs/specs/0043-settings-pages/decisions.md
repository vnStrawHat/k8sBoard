# 0043 · Decisions

[Back to index](README.md). Proposed by the architect (the user asked for a proposal); amended after the opus review (2 must-fix, 9 should-fix, nits) and the coordinator's density decision, 2026-10-03.

| # | Decision | Rationale |
|---|---|---|
| 1 | A setting exists only when it changes behaviour the app already has (a constant or a per-tab default today) or W2 / Tokens draw it | ponytail rule from the request; W2 draws no General, Logs, or Terminal content |
| 2 | One settings section per page (`general`, `appearance`, `logs`, `terminal`); two 0024 reserved names move into `general` | a key is found where its page is; nothing read the reserved names yet, so no migration |
| 3 | Dropdowns over number inputs for every numeric setting | no free-text validation, no out-of-range state from the UI; hand edits are clamped by accessors |
| 4 | Log and terminal settings are **defaults for new tabs**; tab toggles never write back | one source of truth per tab; no surprise when one tab's toggle changes the next |
| 5 | **No clipboard-clear setting**: 0016 decision 26 (fixed 30 s) stays | a C1 secret-safety decision is not relaxed without a user request (review nit) |
| 6 | TLS Secrets watch is opt-out and shows as `Not checked: certificates (off in Settings)` | 0020 decision 11 promised the opt-out; coverage must not claim "no problems" for what it does not watch |
| 7 | Row density default **Compact 28 px** (today the kit's 32 px), conditional on the ui-verifier 28 px check; clipping is fixed in the cell, never by the default | coordinator decision per Tokens ("default for power users"); Comfortable 36 px for touch and projectors |
| 8 | Density applies to `DataTable`s only, header included; `workspace.rs` observes `AppSettings` for the live change | the token names table rows; the kit uses one size for header and rows |
| 9 | Search reuses the switcher's `search_text` + `normalize_query` | one matching rule for both cluster lists |
| 10 | Order is the order of `registry.clusters`; a move registers every row of the group and moves the group's entries to the end in the new order | no new key; `cluster_groups` already sorts by entry position; other groups keep their order |
| 11 | Drag only inside an environment group, off while searching; `Alt ↑ / Alt ↓` move the selected row | the environment decides the group; a drag-only control is not keyboard accessible |
| 12 | Six swatches from theme tokens (`danger`, `warning`, `info`, `magenta`, `cyan`, `muted_foreground`); picking the environment's own colour stores `None` | project rule: no colour literals; a cluster on its environment colour keeps following it |
| 13 | Colour drives the title-bar top border only; the environment badge keeps the environment colour | the badge is the risk signal (PROD stays red); after 0046 no table carries a cluster label |
| 14 | Proxy is per cluster (W2 Connection) with three modes: from kubeconfig (default), none, custom URL | W2 draws it per cluster; the default keeps today's behaviour |
| 15 | Settings proxy URLs refuse userinfo; HTTP proxy auth lives in the kubeconfig `proxy-url`; keychain deferred | never store credentials in plain settings (C1, W2 note 5); no keychain code exists |
| 16 | Turn on `kube` `http-proxy` and `socks5`; `HTTPS_PROXY` / `NO_PROXY` stay ignored by the client (exec plugins still inherit them; the hint says so) | no new package; kube does not read `NO_PROXY`, so honouring the variable could route a local cluster through a proxy |
| 17 | Settings accept `http://` and `socks5://` only; the scheme is stored lowercase | kube builds `https://` proxy TLS from the cluster's rustls config (cluster CA only, pinned `tls-server-name`, client cert offered, skip-verify shared); kube matches `"socks5"` case-sensitively |
| 18 | `ProxyChoice::Url(ProxyUrl)` is parsed when the profile is built; an unparsable stored URL is `Err` and fails `open_cluster` with `InvalidProxy`, never direct | invalid states are unrepresentable past the profile; traffic must not silently bypass a proxy the user asked for |
| 19 | The Custom URL input commits on Enter or blur; `Not applied` shows until a valid URL is committed; the form shows only `ProxyUrl::display()` | a half-typed URL never applies; a stored value with userinfo is never echoed |
| 20 | A proxy change applies to the next connection; a running session keeps its client | like 0025 decision 24: no surprise disconnect |
| 21 | Watched folders load each file standalone; files are not added to `registry.kubeconfigs` | the folder is the source of truth; deleting a file on disk is enough |
| 22 | **A folder file never starts a session on its own**: only an exact `last_used` match; `--context`, `current-context`, and the first-file fallback see chain and registry files only | a dropped file must not run its exec plugin or send credentials at start (must-fix 1) |
| 23 | **Folder rows are never probed automatically**, whatever their auth kind | a dropped file can aim `tokenFile` at a real token and `server` at any host (must-fix 2) |
| 24 | Folder files are read bounded (`take(1 MiB + 1)`) and parsed with `Kubeconfig::parse_file`, which applies kube's relative-path rule against the file's folder | kube's `read_from` reads without a bound; relative paths must not resolve against the app's working directory |
| 25 | Rescan 500 ms after the last event, at least every 2 s during a burst, then one batched `notify` | editors write in bursts; a never-ending stream (a log file) still gets rescans |
| 26 | The catalog task reads a channel; tests inject events and use the fake clock | deterministic tests; real notify only in live checks |
| 27 | A vanished file drops its rows but not its running session; a broken rewrite keeps the last good version | the 0025 rule for removed files; a half-written save must not drop clusters |
| 28 | A missing folder raises a notice and is checked again at the next start; Stop watching never deletes | read-only by construction; no polling loop |
| 29 | Caps: 50 candidate files per folder, 1 MiB per file, fixed extensions | a large folder must not stall the app or parse unrelated files |
| 30 | `notify` 7 from `Cargo.lock` (via `gpui-component`), default features | an installed dependency over a hand-written poller; no new package |
| 31 | Scrollback max 10,000 lines | about 16 MB per tab, 128 MB for 8 tabs, by the 0036 decision 15 arithmetic |
| 32 | Left out: Metrics and Extensions pages, global proxy, recursive watch, clipboard-clear time, log buffer size, cursor style, copy on select, paste-confirm toggle, reopen-last toggle | speculative or a safety rule; add on request |
