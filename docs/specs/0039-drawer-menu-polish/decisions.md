# 0039 · Decisions

[Back to index](README.md). Architect defaults recorded without a user round-trip. Every item is read-only or reuses an action that already ships.

| # | Decision | Rationale |
|---|---|---|
| 1 | View logs on a pod with several containers is a **submenu** (`LogsMenu`, built like `ShellMenu`); one container stays a direct item with hint L | W4 n2; the same rule Shell and Port-forward follow |
| 2 | The submenu lists **every** container, init included, tagged `MAIN`/`SIDECAR`/`INIT`; none is disabled for its state | logs of a crashed or finished init container are the ones people need; the API serves them |
| 3 | The container ⋯ menu has View logs, Open shell, Copy image, **without key hints** | L and S act on the pod's default container, so a hint there would lie |
| 4 | Open shell in the container menu reuses the 0036 guarded flow (`start_shell` with that container) | no new connect path; the gate and the lock already apply |
| 5 | Attach is **not** listed in either menu | it is a new mutating connect (audit 0040) |
| 6 | "Last job" = the Job the controller names `{cronjob}-{unix minutes of last_schedule_at}`; its pods must still be in the pods list | works with the drawer closed and from the key, using only always-loaded lists; manual triggers are a known ceiling (`ponytail:`) |
| 7 | L opens workload logs on Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, and CronJobs through the one `RowAction::ViewLogs` path | W7 shows L on these items; the menu and the key must agree (0028) |
| 8 | The diff compares **pod templates** read by one GET per ReplicaSet (`pod_template_yaml`), not the summaries | summaries keep containers and images only; a revision differs in env, probes, resources, annotations |
| 9 | Older revision on the left, newer on the right, whichever row was clicked; `pod-template-hash` and a null `creationTimestamp` are dropped | a stable reading direction; the hash always differs and says nothing |
| 10 | Masking is the YAML tab's (`mask_object`): env hidden by default with a toggle | the same secret rules everywhere (C1); a hidden value that changed reads the same on both sides, so the toggle is needed |
| 11 | The diff lives in a **dialog**, fetched on open, never cached | it is a one-off look; the drawer keeps its tabs |
| 12 | The ResourceQuotas `Edit` placeholder is removed, not wired | Edit YAML (E) is what W7's `Edit` means for quotas |
| 13 | The ConfigMap hint is **text in Used by**, shown whenever an env reader exists, with no restart button | the drawer stays open after an Edit YAML commit, so the hint is in view right after the change; a button would be a write |
| 14 | CronJob and Job owners are left out of the hint | each run starts new pods that read the new values |
| 15 | LimitRanges come from a **watch** in the Namespace drawer's related stream, next to the quotas, gated by a new `AccessCheck::ListLimitRanges` | the related-watch pattern of quotas; a denied list never starts (no 403 loop) |
| 16 | `LimitRangeSummary` keeps quantities **as written** (`BTreeMap<String, String>`) | display only; parsing would add nothing |
