# 0019 · Decisions

[Back to index](README.md). Architect defaults (the user asked not to stop for questions). 0004 decisions apply unless replaced here.

## Workload logs

| # | Decision | Rationale |
|---|---|---|
| 1 | Subjects = kinds whose `KindRow.related_pods` is set: Deployment, StatefulSet, DaemonSet, ReplicaSet, Job; reuse `PodOwner` / `owns_pod` | membership logic already exists and is tested (0012); no selector query |
| 2 | Members come from the session's **pods watch** (the tab observes `ClusterSession`); no new watch or list call | zero extra API load; churn arrives with the existing 100 ms batches |
| 3 | When the namespace scope no longer covers the workload, or the pods list is not ready, membership **freezes** (streams keep running) and the legend says so | a scope change must not end streams the user opened |
| 4 | Caps: **10 pods** and **20 live streams** per tab; pod limit = `clamp(20 / containers, 1, 10)`. Every live `TabStream` counts, member or leaver; joins wait for a free slot | bounds HTTP streams and staging memory even during churn; W8b shows 3 pods |
| 5 | Ranking: ready first, then newest `created_at`, then name. Membership is **sticky**: listed members stay; free slots take the best newcomers | no stream churn when readiness flaps |
| 6 | A pod that leaves the list keeps its streams for a **10 s grace** (the server usually closes them first with the final lines), then they are dropped; its chip shows `deleted` | no lost tail lines on rollout, and leavers cannot hold slots forever |
| 7 | One stream per (pod, selected container); default selection = `default_container` of the first member; multi-select picker | W8b "Containers: api, worker ▾" |
| 8 | **Merge:** a 2 s staging window at every (re)start, stable-sorted by kubelet timestamp, then pushed once; afterwards arrival order. Staging flushes early above 8 MiB. Late joiners request a **50-line tail** (`LogRequest.tail_lines`) and append on arrival | initial tails arrive pod by pod; sorting them is the visible part; a short late tail limits out-of-order history. Ceiling: README open item 1 (incl. cross-node clock skew) |
| 9 | The 0004 buffer caps (10k lines / 8 MiB) apply per tab across streams; eviction after the sorted merge keeps the newest lines overall. **Fairness ceiling:** a chatty pod can evict a quiet pod's lines | one memory bound per tab; per-source quotas are the upgrade (README open item 3) |
| 10 | Screen prefix `{short}/{container}`; `short` = last `-` segment (StatefulSet: the pod name without the owner name, `-0`; a pod with no suffix keeps its whole name cut from the start); color = `chart_1..chart_5` by the pod name's slot. Export uses the full prefix (decision 32) | W8b `x2k4q/api`; ordinals need the name; theme tokens only |
| 11 | Menu label "View logs (all pods)", Job "View logs" (W7 acts); opens through `AppShell`, so `kind_menu` needs no new parameters | one entry point owns connection, session, and dock |

## Filters and views

| # | Decision | Rationale |
|---|---|---|
| 12 | Regex via the **`regex` crate**: 1.13.1 is already locked (tracing-subscriber `env-filter`) → workspace dependency, no new package | linear-time engine (no catastrophic backtracking); avoids a hand-written matcher |
| 13 | One filter input + a **Regex** toggle; Plain stays the default (0004 decision 9). Regex is case-insensitive, `size_limit` 1 MiB; an invalid pattern keeps the previous matcher and shows `Invalid regex` | typing `a|(` must not blank the view |
| 14 | Level detection order: JSON field (string, or pino/bunyan number 10/20 Debug, 30 Info, 40 Warn, 50/60 Error) → logfmt `level=` → klog `E0501` → uppercase/bracketed/tab-delimited keyword in the first 64 bytes, with `_` a word character; indented lines inherit the previous level of the same source | covers JSON (incl. Node.js), Go/k8s, Java/Python loggers; `ERROR_COUNT` is not a level; stack traces follow their error |
| 15 | A line with no detected level counts as **INFO** for the chips | turning INFO off then hides plain chatter, which is what users mean |
| 16 | Chips default all on | show everything first; W8 shows a user-chosen state |
| 17 | Error rows get a `Bad` tone tint at 8 % opacity; no separate level column (the text already carries it), except JSON mode which shows the level tag | W8 `.hl` rows, no duplicate words |
| 18 | JSON toggle (off by default): object lines show `msg`/`message` as the row text and the other fields (minus level and time keys) as a pretty block; computed per rendered row | W8b JSON block; only visible rows pay |
| 19 | Level and source are computed once at push and stored per line | the filter rescan stays O(lines) without reparsing |

## Dock, export, safety

| # | Decision | Rationale |
|---|---|---|
| 20 | Layout **Compact** (Normal, Minimized) vs **Full** (Zoomed): Full adds the workload legend row and the histogram; toolbar identical | W8b note 4 |
| 21 | Histogram = kit `gpui_kit::component::chart::BarChart` over visible lines, ≤ 60 buckets, bars with errors in `Bad`; memoized per buffer revision. **Ceiling superseded by 0044:** a drag across the chart now sets a time window that filters the list | no custom `Plot` needed (0010 needed gaps and references; this does not) |
| 22 | **Export…** button (both layouts) → `cx.prompt_for_new_path` → writes visible lines with full RFC 3339 times and full `{pod}/{container}` prefixes; nothing on cancel; the platform dialog asks before overwriting | C9 (user-approved); reverses 0004 decision 15 for this button only |
| 23 | **Copy stays the plain clipboard**, not the 0016 private path | logs are app output, not Secret objects; the user copies on purpose to paste into tickets, where a 30 s auto-clear and history exclusion would break the workflow. Ceiling: a secret printed in logs reaches clipboard history, as with `kubectl logs \| clip` |
| 24 | **Kubelet/node logs deferred.** On v1.29 the journal query (`/logs/?query=kubelet`) needs the alpha `NodeLogQuery` gate plus kubelet `enableSystemLogQuery` (both off by default; beta 1.30). The always-on `/logs/` file browser exposes all of `/var/log`, including other namespaces' pod logs, outside 0011's fixed path allow-list | not reliable on UAT and widens the nodes/proxy surface |
| 25 | Pop out deferred (**superseded by 0044**: log tabs pop out to their own window; Shell tabs do not) | needs a second window sharing a `LogTab` entity (focus, theme, close lifecycle); Zoom covers the need |
| 26 | Container **Logs** sub-tab: clicking it opens or focuses the dock tab on that container; its body is a note + "Show in dock". It never streams by itself and navigation never auto-opens a stream | the dock sits right below the drawer; no duplicate stream; explicit action only |
| 27 | `LogDock::open` on an existing pod tab switches container only for an explicit container (`ContainerChoice::Explicit`) | a menu reopen must not undo the user's pick |
| 28 | "+ ▾": "Logs of selected" (disabled with the `NoLogTarget` text when there is no session or the selection is not a pod or workload) and "Shell into selected" (disabled through `action_availability(OpenShell)`). No placeholder Shell tab (0004 decision 1) | the Shell slot users can see, without a dead tab |
| 29 | Tab reorder by drag (GPUI `on_drag`/`on_drop`); the active tab stays active | W8 note 3; small |
| 30 | Container submenu in menus skipped | the tab picker and the Logs sub-tab cover per-container opening |
| 31 | Logs are never traced; export paths and file names are never traced either | 0004 decision 15 rule, extended to Export |
| 32 | Export writes the full `{pod}/{container}` prefix; the screen and Copy keep the short one | a saved file must identify pods without the legend |
| 33 | **Exception to "crates/cluster unchanged":** `LogRequest` gains `pub tail_lines: u32` (1000 for pod tabs and initial members, 50 for late joiners) | the only way to ask the API for a short late tail; one field, no new behavior |
| 34 | **Rejoin:** a pod name that left and returns (StatefulSet recreate) first drops that name's old streams, then reuses its color slot; one legend chip per name | stable colors and no duplicate chips for `db-0` |
| 35 | Export writes with one `std::fs::write` (truncate, then write); a failure mid-write can leave a partial file, reported as `Could not save the logs` | atomic temp-file + rename is more code for a user-chosen file the user can simply save again |
| 36 | `selected_log_target` returns `Result<LogTarget, NoLogTarget>` (an enum with `Display`), not a string | typed reasons; the menu shows the `Display` text |
| 37 | `log_target.rs` is pure (no session, no GPUI); `LogDock` holds a weak session handle and passes the session entity to `LogTab::new` | targets stay unit-testable; the tab never keeps the session alive |
| 38 | Matcher module is `line_matcher.rs`: `log_filter.rs` already exists (the tracing `EnvFilter` pin) | no name collision; AC 4's grep stays meaningful |
