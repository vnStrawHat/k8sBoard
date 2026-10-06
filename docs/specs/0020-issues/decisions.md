# 0020 · Decisions

[Back to index](README.md). Architect defaults (the user asked not to stop for questions), amended after the advisor review. Settles C13 for this spec.

## Feeds and cost

| # | Decision | Rationale |
|---|---|---|
| 1 | **Core feeds** (always on, every scope): namespaces, nodes, pods (already on), pod and node metrics, kubelet stats (already on, 0010/0011), plus **one new Warning events watch** (`watch_events(scope, WarningsOnly)`, N watches) | pods and nodes carry most problems; Warning events give causes no state field has (FailedCreate, FailedMount, probe failures); metrics are already sampled |
| 2 | **Condition feeds** (always on): Deployments, DaemonSets, Jobs, HPAs, PDBs, ResourceQuotas, PVCs, TLS Secrets. Multiplicity `N = max(1, scope.namespaces().len())`; N ≤ 2 → one watch per namespace; **N > 2 → one All-scope watch per kind**, rows outside the scope dropped on tokio before they reach the session | every scope gets every rule; the watch count stays bounded; dropping early keeps memory to the scope |
| 3 | A 403 on an All-scope condition watch turns that feed Off (`not permitted cluster-wide`) | users with namespace-only RBAC see why the rule is missing |
| 4 | Condition feeds keep **compact summaries** as `LiveList<KindObject>` (the 0012 summary enum), never `KindRow` with pre-built drawer sections | `kind_diagnosis` reads `KindObject` directly; rows would double memory |
| 5 | Not fed: StatefulSets, ReplicaSets, CronJobs, Services, Ingresses, ConfigMaps, PVs, RBAC kinds | no condition-only rule (0012) or not an outage; their pod problems arrive through pods; Service/Ingress checks are 0022 |
| 6 | A rule runs only on Ready feeds. A loading, denied, or failed feed makes coverage **Partial**, named in the header and the ⚑ tooltip. A **Limited** feed (the 0011 kubelet cap of 10 nodes) is noted in muted text and does not make coverage partial | the list never claims "no problems" for something it does not watch; a designed cap is not a gap |
| 7 | Condition feeds start after the access review is `Known` or `Unknown`; a `Known` denial (N ≤ 2) turns the feed Off with `not permitted: {check}` | same gate as the 0010 nodes feed |
| 8 | Feed updates use a **silent** subscription (no `cx.notify()`); they only mark the board dirty | core lists already notify; condition feeds must not repaint the app 8 more times per batch |
| 9 | While the explorer shows a fed kind, two watches of that kind run | sharing would rewire 0005/0012 explorer ownership; README open item 1 |
| 10 | Budget AC: idle RSS with all feeds stays **< 150 MB** on UAT; the numbers, including the UAT Jobs count, are recorded below before merge | C13; measured, not estimated |
| 11 | A cluster-wide TLS Secrets watch appears in API audit logs as `list/watch secrets`; it is opt-out as `general.watch_tls_secrets` in Settings › General (0043); off, coverage says `Not checked: certificates (off in Settings)` | the user should know; certificates have no other signal |

## Engine

| # | Decision | Rationale |
|---|---|---|
| 12 | Rules are **pure functions** evaluated on the **main thread**, at most once per `ISSUE_TICK` (1 s) when dirty, and every `TIME_REFRESH` (30 s) | inputs already live on the main thread; tokio would clone MBs per run; budget ≤ 4 ms (AC 6) |
| 13 | Notify when issues or coverage changed; also on each `TIME_REFRESH` run while the Issues screen is visible (ages move) | a stable cluster costs zero repaints elsewhere |
| 14 | **Reuse the WHY rules**: `pod_diagnosis` (0008), `kind_diagnosis` (0012–0018) with **no pods**, `expiry_state` (0016), `usage_tone`/`quota_tone` thresholds | one wording and threshold per fact; an empty pod slice leaves only the condition-only (*) rules |
| 15 | `PodDiagnosis` gains `cause: DiagnosisCause` | severity, pill, grace, and action depend on the rule that fired |
| 16 | **At most one issue per object**: first match in rule order (pods → nodes → namespaces → kinds → certificates → volume usage → events) | never two rows for one object; the most specific cause wins |
| 17 | **Grouping**: pod issues with the same rule and workload (Deployment via the ReplicaSet hash, else the controller) become one issue with `count`; with 16 a workload shows its worst pod rule | W3 lists 4 rows, not 40 pods; the workload drawer lists the rest |
| 18 | Event issues are dropped when their object, or a pod of a group, already has an issue, or the Pod/Node is gone | events restate state the pod rules explain |
| 19 | Two severities, **Critical** (Bad) and **Warning** (Warn); kind rules map tone → severity; HPA, PDB, quota capped at Warning | W3 shows `pill bad` and `pill warn` only; limits are constraints, not outages |
| 20 | **Grace is generic**: `Finding.grace`; the board records first-seen for every finding but shows it only once `now − since ≥ grace`. A held finding still takes its object's slot (no fall-through) | one mechanism for every transient state; tests are one table |
| 21 | Age `since` = the rule's onset (condition `changed_at`, event `first_seen`, the pod `Ready` transition for the exit rule; crash and image-pull rules use the oldest retained `BackOff` / `Failed` event `first_seen`, else the pod creation; a certificate never starts before its `notBefore` or the secret's creation), else first-seen for ordering and grace only; the Age column shows `—` without a real onset | real onset when the API has it; `finished_at` is the last exit, not the onset, except for restarts |
| 22 | The board (`first_seen`) lives in `ClusterSession`: retries and scope changes keep it; a context switch makes a new session and clears it; it does **not survive an app restart** | no persistence (decision 27); stable ages while connected |
| 23 | Cordoned nodes, SchedulingGated pods, `NoCertificate` secrets, Services matching no pods are not issues | intentional or config hygiene; Service checks are 0022 |

## Screen and shell

| # | Decision | Rationale |
|---|---|---|
| 24 | No Issues wireframe (W3 nav jumps to Overview): a toolkit table with the W3 "Needs attention" content as columns | consistent with every list; 0021 renders the same issues as cards |
| 25 | Clicking a row **reveals** the object (screen + drawer); no drawer on Issues | reveal already handles loading |
| 26 | Primary actions are read-only: **View logs** (crash, exit, restart rules), else **Open**; "Fix image" waits for 0032 | read-only rule |
| 27 | No snooze, acknowledge, persistence, severity chips, or menu filters | not wireframed; quick filter and column sort cover filtering (YAGNI) |
| 28 | Sidebar: Issues item with the total; each item shows its issue count (reveal target = that screen) before the muted total, toned by the worst severity | anatomy "counts and error counts"; one unit everywhere |
| 29 | Title bar: ghost button with Lucide **`IconName::Flag`** (bundled as `flag.svg` in gpui-kit-assets 0.7.0; the coder verifies it renders, else `TriangleAlert`) and the count, toned by worst severity | W2 shows `⚑ 4`; the flag keeps the wireframe's glyph |
| 30 | Counts show only after pods and nodes are Ready | no "0 issues" while loading |

## Budget measurements (fill before merge)

| Measure | Before 0020 | With 0020 |
|---|---|---|
| Idle RSS after 5 min, UAT, scope All | not measured (needs a build of the commit before 0020) | 113.5 MB working set (release, `--screen issues`; 106 MB at 30 s, 113 MB at 2 min) |
| Watches (status bar), scope All / 3 namespaces | — | 13 / 17 (Issues screen, no drawer; `open_watch_count`) |
| UAT Jobs count (probe `count` line, 0012) | 5 (sidebar count) | — |
| `issue_evaluation_budget` (release, ms) | — | 0.94 (1,000 pods with 50 problems, 2,000 events, 1,000 Deployments) |
