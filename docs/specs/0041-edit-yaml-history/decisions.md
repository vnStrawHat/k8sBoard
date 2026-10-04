# 0041 · Decisions

[Back to index](README.md)

| # | Decision | Rationale |
|---|---|---|
| 1 | No snapshot, no rollback, no ConfigMap compare, no timeline ConfigMap rows | User decision 2026-10-03. Each needs stored history; the cluster keeps none for ConfigMaps. |
| 2 | Revision history for **Deployments only** | W10 draws a Deployment. Deployments keep history as ReplicaSets, which the app already reads (0039). StatefulSets and DaemonSets use ControllerRevisions: a new kind, watch, and masking path for no drawn need. |
| 3 | The tab **embeds** the 0039 `RevisionDiffView` | Same GETs, masking, env toggle, diff rows, and texts; no second diff view. The dialog and the tab differ only in their frame. |
| 4 | Quota: **steady state, unscoped quotas, requests and limits of CPU and memory, pods** | The dry-run of a Deployment `PUT` never checks quota: quota admission runs when pods are created. The check is advice. Surge pods, scoped quotas (`BestEffort`, `PriorityClass`, …), LimitRange defaults, and `spec.overhead` are known ceilings, named in the code with a `ponytail:` comment. |
| 5 | Quotas come from the session's **ResourceQuotas condition feed** (`issue_feeds.rs`) | It already watches quotas for the scope; no new request. When it is off (denied, narrow scope) the check says so instead of reading the cluster. |
| 6 | "Who" = the **field manager** name of `managedFields` | Kubernetes stores no user on an object (only the audit log, out of reach). The manager is what kubectl, Argo CD, Helm, or a CI tool set (`kubectl-edit`, `argocd-controller`, `helm`, `ci-bot`). The tooltip says "field manager". |
| 7 | Rule: the Deployment's newest non-status `managedFields` entry that owns `f:spec` › `f:template`, used when its time is at most 60 s after and at most 30 min before the event's `last_seen` | A rollout starts right after the template write; a later write cannot have caused it. Outside the window the event source stays (today's `deployment-controller`). `ponytail:` heuristic, an exact cause needs the audit log. |
| 8 | Click-to-diff: **newest revision vs the one before**, newest = highest revision number | The Deployment controller gives a rolled-back ReplicaSet the next number, so the highest number is always the current template; no Deployment GET is needed. |
| 9 | One new read, `deployment_revisions`: a LIST of the namespace's ReplicaSets, kept when their controller owner is the Deployment | Serves both the tab and the timeline; needs no selector (the Overview has no Deployment row in hand). The summaries are built on tokio; the raw list is dropped there. |
| 10 | Timeline rows keep **reveal** except Deployment rows, which open the diff; the diff dialog gains `Go to deployment` | W3 n4 says a click opens the diff (W10). Reveal stays one click away. |
| 11 | The quota line is a **warning**, never a block; an exceeded item is also a confirm-dialog warning | The server accepts the Deployment change either way; the user decides. |
