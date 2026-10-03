# Gap plan — local-only specs 0024–0029, mutating specs 0030–0038, backlog

[Back to index](README.md). Decision IDs (C*) are in [cross-cutting.md](cross-cutting.md).

## Local only (no cluster writes; the app writes its own config)

### 0024 — Settings store, cluster registry, environments
App config directory with a `--config-dir` override (C2), versioned file format, a cluster registry (kubeconfig files, contexts, display name, env, color, order, default namespace, safety and metrics prefs; never tokens), `KUBECONFIG` multi-entry merge, env guessing from context names (C5), env theme tokens, env badge and top border in the title bar, persisted UI state (theme, table sort and hidden columns; dock height is a reserved key owned by 0019/0025), "Set as default" namespace.
- Deps: C2, C5. Risk: Med (first disk writes; agents must use `.tmp/`). New deps: config dirs, TOML/JSON (C6).

### 0025 — Settings window (W2)
Separate OS window via `cx.open_window`, single instance, shared registry entity; Ctrl , and "Manage clusters…" open it. Clusters page (env groups, drag order → Ctrl 1–9, form, read-only connection info, Test connection with latency, Safety and Metrics fields, Remove from k8sBoard), Add cluster (import file Ctrl O, watch folder, paste YAML), General, Appearance (theme, density 28/36), About; empty frames for pages owned by later specs.
- Deps: 0024. Risk: Med (second window and shared state in GPUI; folder watching needs a watcher dep or polling).

### 0026 — Cluster switcher (W1, single cluster)
Grouped by env, health line per cluster (connectivity, latency, issue count when known), Filter clusters, All/Connected, Retry for unreachable, Ctrl 1–9, Ctrl Shift C; status bar "API n ms".
- Deps: 0024, 0020. Risk: Med (background health polling cost, C4).

### 0027 — Multi-cluster views
Checkbox selection with "View N clusters" (apply once), several live sessions at once, Cluster column in every table, filter and sort by cluster, riskiest-env border, `+N` label, issue and overview merging, logs per cluster.
- Deps: 0026, 0009. Risk: High (session and memory model, C4; every table and drawer keyed by cluster).

### 0028 — Keyboard map
Done: focus model (single keys never act in text fields, menus, popovers, or dialogs), J/K, ⏎, Esc ladder, ↑↓ with the drawer open, `[` `]`, `/`, `?` sheet, Ctrl N, Ctrl `, Ctrl Shift M, Ctrl Tab, Ctrl W; Ctrl , and Ctrl O moved from 0025; Settings › Keyboard Shortcuts (read-only list). `:` is deferred to 0029. Letters S, F, C, D, E, R, ⇧S, Del are gated (no A key); only L, Y, and Ctrl C run.
- Deps: 0025 (page). Risk: Med (GPUI key contexts and kit focus).

### 0029 — Command palette (W9)
Done. Ctrl K, `:`, and the title-bar search box; prefixes `:` kind, `@` cluster, `#` namespace, `>` action; fuzzy match over live snapshots with live status; Go to; scope chips with env color; Tab preview; footer hints; mutating actions listed with "needs confirm" and disabled until their spec ships.
- Deps: 0028, 0026. Risk: Low–Med. No new dependency (own scorer). Tab preview moves the table cursor only (no drawer, no screen switch, no watch); Ctrl ⏎ moved to 0032; mutating actions stay listed and disabled with their 0028 reason.

## Mutating (RBAC-gated by SSAR, confirmation for destructive actions, disabled on UAT)

C3: the user approved 0030–0037 once on 2026-10-02 (one approval for all mutating specs); 0038 is deferred. Live allowed-path checks still need a write-capable test cluster ([risks.md](risks.md) R2). On UAT each action must render disabled with its SSAR reason.

### 0030 — Guardrails and write path
Per-cluster read-only mode (default on for PROD), lock toggle and Ctrl Shift R, one action gate = lock ∧ SSAR (per verb/subresource) ∧ env tier, confirmation dialog for every action (prod: type the name; staging, dev, local: click Confirm; user 2026-10-02), server-side dry-run helper, local audit log with optional note (C8, C10), and an allow-list replacing the 0001 read-only grep.
- Deps: 0024. Risk: Med (the contract every later spec relies on).
- Spec: [0030-guardrails-write-path](../specs/0030-guardrails-write-path/README.md). All steps are implemented: 1 (write path, clippy `disallowed-methods`), 2a (pure guard, gate, `confirm` setting, Safety page), 3 (audit module), 2b (per-session lock, badge, Ctrl Shift R, unlock dialog), and 4 (write flow, confirm dialog, Cordon / Uncordon, the first shipped mutating action).

### 0031 — Edit YAML (W10)
GPUI Kit Code Editor, diff vs cluster (default view), semantic change list, checks (dry-run, quota, rollout impact), Apply with conflict handling (C8), pre-apply snapshot for one-step rollback (incl. ConfigMaps), Revision history tab, "New" from templates (Namespace, ConfigMap, Quota, PDB, RoleBinding).
- Deps: 0030, 0007. Risk: High (editor maturity, LSP optional). New dep: `similar`.
- Spec: [0031-edit-yaml](../specs/0031-edit-yaml/README.md). **Done for the editor** (steps 3 and 4: the lazy `update` gate, the editor view, the Diff tab from a server dry-run, Apply with conflict rebase and the 422 panel, discard prompts, audit). Replaces the object with its `resourceVersion` instead of using SSA (C8 amended). Defers quota check, snapshot and rollback, Revision history, and templates to a later spec.

### 0032 — Workload and object actions
Scale, Restart rollout, Pause/Resume, Roll back to revision, CronJob Trigger now and Suspend, Job Re-run, HPA min/max, StorageClass set default, PVC Expand, Certificate Renew; list-level buttons on selected rows; palette actions enabled.
- Deps: 0030, 0009, 0012. Risk: Med.
- Spec: [0032-workload-actions](../specs/0032-workload-actions/README.md). **Done for workloads** (Scale, Restart, Pause/Resume, Roll back, CronJob Suspend/Resume, Trigger now, Job Re-run; menus, keys, popover, palette, drawer buttons, selection-bar batches). The object actions (HPA min/max, PVC Expand, StorageClass default) are 0032b, **done** (steps 1–4: popovers, dry-run then confirm, selection-bar `Edit limits` / `Expand` / `Set default`, the two-object Set default with `BatchFailure::Stop` and Retry; [as built](../specs/0032b-resource-edits/as-built.md)); Certificate Renew moved to 0018.

### 0033 — Delete and pod lifecycle
Delete for every kind (typed confirm on prod, red, last), Restart pod (delete and recreate), Evict (Eviction API), bulk delete from the selection bar.
- Deps: 0030, 0009. Risk: Med (blast radius; bulk confirm text).
- Spec: [0033-delete](../specs/0033-delete/README.md) covers delete only. **Done** (steps 1–3): single and bulk (50 max) from the menus, Del and ⌘⌫, the palette, and the selection bar `Delete…`; uid precondition, propagation choice, finalizer hints, one audit line per object. Restart pod and pod Evict are still open: neither 0032 nor 0034 took them (0034 evicts only inside a drain); see the audit's proposed 0040 pod lifecycle spec.

### 0034 — Node maintenance (W5, W6)
Cordon/Uncordon (bulk), Drain dialog (kubectl-flag options with consequences and counts, PDB-based per-pod preview, grace, timeout, typed node name, Cordon only), sequential multi-node drain that stops when stuck, progress tab in the dock with cancel, Edit taints and labels.
- Deps: 0030, 0013, 0009. Risk: High (long-running operation, eviction retries, cancel semantics).

### 0035 — Port-forward and Port Forwarding page
kube `ws` feature on; local listeners; Forward buttons (pods, services, workloads pick a ready pod); W4b live state; Port Forwarding page across clusters (Stop/Start/Retry, auto-reconnect up to 5, presets persisted, change local port, Open in browser, Copy address, traffic counters, events); status bar count. Built (steps 1 to 3b): the transport, the page and drawer, the guarded start (one confirm dialog per start, an audit line per start), Forward buttons, `Port-forward ▸` menus, F, New forward, Change local port…, Remove preset…. UDP and several ports in one row are out of scope.
- Deps: 0030, 0024. Risk: High (sockets, reconnect, port conflicts). UAT denies `create pods/portforward`.

### 0036 — Terminal and pod shell
`oneterm-vt` pinned git dependency (`default-features = false`), terminal element (grid, input, selection, copy, Find), Shell dock tabs (W8b), shell picker, Clear, Reconnect, sessions kept across navigation. Attach is a later item, no longer owned by 0036; Settings › Terminal & Shell moved out of 0036 (W2 draws no content for it). Built: guarded open (one confirm dialog, audit line per start), the multi-check gate, the "{N} shells will close" release confirm.
- Deps: 0030, 0028. Risk: High (largest custom UI; untestable on UAT: exec denied).

### 0037 — Node shell and debug containers
Done (steps 1 to 3). Privileged debug pod (hostPID, nsenter PID 1) like `kubectl debug node/`, deleted on every end of its session (tab close, shell exit, failure, switch, window close, quit best effort) with a leftover sweep at session start, per-cluster "Allow node shell" (W2 Safety; off for prod and for a guessed Staging), ephemeral "Debug container…" with an options dialog. The node name is typed in every environment. Allowed-path checks wait for a disposable cluster (R2); UAT denies `create pods`, `delete pods`, `patch pods/ephemeralcontainers`, and `create pods/attach`.
- Deps: 0036, 0025. Risk: High (privileged pod creation; orphan cleanup). W2 Safety is complete with the node shell toggle.

### 0038 — Helm write actions
**Deferred by the user (2026-10-02); not scheduled.** Roll back and Uninstall releases; decision C12 (native vs `helm` CLI). No other spec depends on it.
- Deps: 0017, 0030. Risk: High.

## Backlog (no wireframe screen; needs a user decision)

| Item | Source | Note |
|---|---|---|
| Prometheus source, 30-day ranges, Settings › Metrics | W4c note 1, W2 | after 0011 |
| Cloud scans (AWS EKS, GKE, AKS) | W2 add menu | runs external CLIs |
| Topology Traffic mode | W11 note 1 | needs mesh or eBPF |
| Extensions page, plugins (`gpui-shell`), AI via MCP, GitOps status | W2 nav, Stack phase 3 | not designed in the wireframes |
