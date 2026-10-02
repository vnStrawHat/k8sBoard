# Gap plan — read-only specs 0007–0023

[Back to index](README.md). Each spec = one architect spec + 1–3 coder steps. All are verifiable on UAT (`readonly@Monitor`). Order = delivery order; "Deps" are hard prerequisites. Decision IDs (C*) are in [cross-cutting.md](cross-cutting.md).

### 0007 — Drawer YAML tab (view only)
On-demand `get` of the selected object, serialized to YAML with `managedFields` hidden (toggle) and `status` kept; read-only GPUI Kit code view with highlighting, Copy, and Y shortcut slot. Adds the tab bar to Node and kind drawers (Overview / YAML / Events, moving the 0006 Events section into a tab) and a YAML tab to Pods; every later kind gets its YAML tab for free. Applies the 0003 secret rules (explicit action, never logged or written, Copy only) and masks Secret `data`/`stringData` (C1).
- Deps: 0006, C1, C6 (YAML serializer). Risk: Low. New dep: YAML serializer.

### 0008 — Pod and Node drawer details
Pods: WHY box (pure reason rules per container), service account and `→` links (node, owner), container Info with digest, pull policy, ports (disabled Forward), resource requests/limits, probes with current result, Env and Mounts sub-tabs showing **names and sources only** (C1), "Copy kubectl command". Nodes: conditions, capacity/allocatable, system info (OS, kernel, runtime, kubelet), "View pods on node" via a node filter.
- Deps: none. Risk: Low–Med (probe "failing n/n" needs events from 0006 or a ready-condition rule). Cluster crate: new summary fields.

### 0009 — Table toolkit
Filter chips (Namespace multi, Status, label query, "+ Filter"), `/` quick filter, "N of M match", column sort, Columns ▾ toggles, Nodes summary chips as filters with version-skew tint, ReplicaSet "Hide inactive", multi-namespace picker in the title bar, row checkboxes and multi-select (no bulk actions yet), cheap per-kind sidebar counts (C11), Events "Pause stream" and "Filter similar" (0006 open items).
- Deps: none. Risk: Med (GPUI Kit `Table` sort/selection API; one filter model shared by every kind).

### 0010 — Metrics I: metrics-server and the Monitor tab
A per-session sampler polling `metrics.k8s.io` every 15 s into bounded ring buffers (pods, containers, nodes; lazy per visible screen). Pods Memory/CPU columns, Nodes CPU/Memory bars, container usage bars, Monitor tab (CPU, Memory) for Pod, container, Node and aggregated workloads with request/limit lines, OOMKilled markers from terminations, 15m–24h ranges (longer ones greyed until Prometheus), Table view, and ⤢ for kind drawers.
- Deps: 0008. Risk: Med (chart component capability, C6; memory budget). UAT: `metrics.k8s.io/v1beta1` available.

### 0011 — Metrics II: kubelet stats
Read `GET /api/v1/nodes/{node}/proxy/stats/summary` (fixed path allow-list) for Network rx/tx and Disk I/O read/write charts, PVC used bytes and inodes (for 0014), and per-node allocatable usage. One poll per node per 15 s: always on for up to 10 Ready nodes, else while a drawer needs the node; cAdvisor (Disk I/O) only while a Monitor tab shows (at most 3 nodes).
- Deps: 0010. Risk: Med (nodes/proxy is powerful; must never build arbitrary proxy paths). UAT: `get nodes/proxy` allowed.

### 0012 — Kind drawer completions (0005 open items) — implemented
`PodSummary.labels`; Services Endpoints column and list via an EndpointSlice watch plus "matches no pods"; ConfigMap "Used by" from pod specs and values fetched on drawer open; Deployment revisions (ReplicaSets by owner, read-only list); StatefulSet pods by ordinal with PVCs; DaemonSet rollout-by-node; Job attempts; WHY boxes for DaemonSet, Job, Deployment; CronJob Next run and next runs; Namespace Pods/CPU req/Memory req columns; Ingress Open URL (system browser); Go to owner links.
- Deps: 0008. Risk: Med (breadth; split into 3 steps). New dep: cron parser + jiff time zones (C6).

### 0013 — Policy kinds: NetworkPolicies, HPAs, ResourceQuotas, PDBs
Four new watches and kinds: policy rules rewritten as sentences with Affects counts; HPA target, min/max, metrics bars, scaling events (0006), "at max" warning; quota usage bars and blocked Pending pods; PDB allowed disruptions, selected pods, BLOCKS DRAIN box (logic reused by the 0034 drain preview).
- Deps: 0012 (pod labels), 0006. Risk: Low–Med. UAT RBAC for these kinds unknown → probe first.

### 0014 — Storage kinds: PVCs, PVs, StorageClasses
PVC status, capacity, Used % (from 0011, "—" without it), access, class, mounted-by pod; PV source and Released hint; StorageClass default ★ and PV count.
- Deps: 0011 (Used %), 0012. Risk: Low.

### 0015 — Access-control kinds
ServiceAccounts, Roles, ClusterRoles, RoleBindings, ClusterRoleBindings: rules table, aggregation flag, two-way binding links, bound roles and "can do" computed locally from bindings, cloud identity annotations, cluster-admin warnings, Hide system.
- Deps: 0012 (used-by). Risk: Low (data), Med (rule aggregation correctness).

### 0016 — Secrets and TLS expiry
Secrets kind with values never kept in summaries (key names, sizes, type), drawer values fetched on demand, masked, Reveal for 30 s, Copy without reveal, Used by, unused flag; "Reveal all" per C1. TLS Secrets parsed for not-after: Ingress TLS column and CERTIFICATE box, Overview cert rule.
- Deps: C1 (blocking), 0012. Risk: High (real credentials; UAT allows `list secrets`). New dep: x509 parser.

### 0017 — Helm releases (read)
Decode `sh.helm.release.v1.*` Secrets (base64 → gzip → JSON) in the cluster crate; Releases table (chart, app version, revision, status), history, values (masked by default), manifest, values diff between revisions, UPGRADE FAILED box.
- Deps: 0016, C1. Risk: Med–High (payload size, secrets in values). New deps: gzip, base64, JSON, diff (C6).

### 0018 — CRDs and generic custom resources
Discovery; CRDs kind (versions, printer columns, schema); every established CRD becomes a sidebar item; a generic `DynamicObject` table driven by additionalPrinterColumns (JSONPath subset) and a schema-driven drawer with a status box; Certificates is the acceptance example; Namespace "remaining resources" for stuck namespaces.
- Deps: 0007. Risk: High (JSONPath subset, unbounded kinds, lazy drawer content per 0005 ceiling).

### 0019 — Workload logs and dock polish (W8, W8b)
`deploy/`, `sts/`, `ds/`, `job/` log tabs merging pods by selector with pod colors; container chips; level toggles; JSON pretty-print; regex; density histogram; container submenu in menus; "+ ▾" new tab; tab reorder; kubelet logs for nodes (nodes/proxy `/logs/`); Pop out window; Export only via a save dialog (C9).
- Deps: 0004, 0012. Risk: Med (merge ordering, many streams). New dep: regex.

### 0020 — Issues engine and Issues screen
Pure rule engine over watch snapshots (pod and container states, Warning events, node conditions, cert expiry, PDB blocks, stuck namespaces, unmatched Services) with plain-language causes and one primary action; Issues screen; red sidebar counts; `⚑ N` title-bar button.
- Deps: 0006, 0013, 0016. Risk: Med (needs pods, nodes, events watched at all times: memory).

### 0021 — Overview dashboard (W3)
Needs attention (top issues), Capacity three-layer bars, node heatmap with click-through, Recent changes timeline (events + revisions; managedFields later), "Last 15 min ▾", Export report (C9).
- Deps: 0010, 0011, 0020. Risk: Med.

### 0022 — Topology, Resources mode (W11)
Graph model (ownerRef, selector, mounts, ingress → service), config checks shared with 0020, layered (Sugiyama) layout, canvas edges and nodes, zoom/pan, drag with remembered positions, minimap, Fit, group by, kind toggles, select → drawer, live status, "Show in Topology" from Services/Ingresses, Export PNG (C9).
- Deps: 0012–0016. Risk: High (no GPUI component; layout dependency C6).

### 0023 — RBAC and policy analysis
"Who can…" (verb × resource → subjects, from bindings), "Check permissions" for a ServiceAccount, NetworkPolicy "Test traffic" (pod A → pod B evaluated locally).
- Deps: 0013, 0015. Risk: Med (correctness claims; label results "computed from RBAC objects").
