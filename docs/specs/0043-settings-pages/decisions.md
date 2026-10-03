# 0043 · Decisions

[Back to index](README.md). Picked without a user round-trip (the user asked the architect to propose the content); each can be revisited in review.

| # | Decision | Rationale |
|---|---|---|
| 1 | A setting exists only when it changes behaviour the app already has (a constant or a per-tab default today) or W2 / Tokens draw it | ponytail rule from the request; W2 draws no General, Logs, or Terminal content |
| 2 | One settings section per page (`general`, `appearance`, `logs`, `terminal`); three 0024 reserved names move into `general` | a key is found where its page is; nothing read the reserved names yet, so no migration |
| 3 | Dropdowns over number inputs for every numeric setting | no free-text validation, no out-of-range state from the UI; hand edits are clamped by accessors |
| 4 | Log and terminal settings are **defaults for new tabs**; tab toggles never write back | one source of truth per tab; no surprise when one tab's toggle changes the next |
| 5 | Clipboard clear offers 15 s–2 min, no "never" | C1: a copied secret must not stay on the clipboard |
| 6 | TLS Secrets watch is opt-out and shows as `Not checked: certificates (off in Settings)` | 0020 decision 11 promised the opt-out; coverage must not claim "no problems" for what it does not watch |
| 7 | Row density default **Compact 28 px** (today the kit's 32 px) | Tokens: compact is the default "for power users"; Comfortable 36 px for touch and projectors |
| 8 | Density applies to `DataTable`s only | the token names table rows; other lists have their own layouts |
| 9 | Search reuses the switcher's `search_text` + `normalize_query` | one matching rule for both cluster lists |
| 10 | Order is the order of `registry.clusters`; a move registers every row of the group and moves the group's entries to the end in the new order | no new key; `cluster_groups` already sorts by entry position; other groups keep their order |
| 11 | Drag only inside an environment group, off while searching; `Alt ↑ / Alt ↓` move the selected row | the environment decides the group; a drag-only control is not keyboard accessible |
| 12 | Six swatches from theme tokens (`danger`, `warning`, `info`, `magenta`, `cyan`, `muted_foreground`); picking the environment's own colour stores `None` | project rule: no colour literals; a cluster on its environment colour keeps following it |
| 13 | Colour drives the title-bar top border only; the environment badge keeps the environment colour | the badge is the risk signal (PROD stays red); after 0046 no table carries a cluster label |
| 14 | Proxy is per cluster (W2 Connection) with three modes: from kubeconfig (default), none, custom URL | W2 draws it per cluster; the default keeps today's behaviour |
| 15 | Settings proxy URLs refuse userinfo; proxy auth lives in the kubeconfig `proxy-url`; keychain deferred | never store credentials in plain settings (C1, W2 note 5); no keychain code exists, a new dependency needs its own review |
| 16 | Turn on `kube` `http-proxy` and `socks5`; `HTTPS_PROXY` / `NO_PROXY` stay ignored | no new package; kube does not read `NO_PROXY`, so honouring the variable could route a local cluster through a proxy |
| 17 | An invalid stored proxy URL fails the connection with `InvalidConfig`, never falls back to direct | traffic must not silently bypass a proxy the user asked for |
| 18 | A proxy change applies to the next connection; a running session keeps its client | like 0025 decision 24: no surprise disconnect |
| 19 | Watched folders load each file standalone; files are not added to `registry.kubeconfigs` | the folder is the source of truth; deleting a file on disk is enough |
| 20 | Rescan after 500 ms without events, per folder, then one batched `notify` | editors write in bursts (temp file, rename); one reload per burst |
| 21 | A vanished file drops its rows but not its running session; a broken rewrite keeps the last good version | the 0025 rule for removed files; a half-written save must not drop clusters |
| 22 | A missing folder raises a notice and is checked again at the next start; Stop watching never deletes | read-only by construction; no polling loop |
| 23 | Caps: 50 candidate files per folder, 1 MiB per file, fixed extensions | a user picking a large folder must not stall the app or parse unrelated files |
| 24 | `notify` 7 from `Cargo.lock` (via `gpui-component`), default features | an installed dependency over a hand-written poller; no new package |
| 25 | Left out: Metrics and Extensions pages, global proxy, recursive watch, log buffer size, cursor style, copy on select, paste-confirm toggle, reopen-last toggle | speculative or a safety rule; add on request |
