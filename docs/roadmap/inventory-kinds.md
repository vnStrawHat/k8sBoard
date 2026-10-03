# Inventory — W7 Kind Explorer (29 kinds + Port Forwarding)

[Back to index](README.md). Source: the `k(...)` definitions in the wireframe script (columns, drawer sections, menu per kind).

## Rules that apply to every kind

| Item | Status | Spec |
|---|---|---|
| YAML tab and "View YAML" | Done (0007); the Y key → 0028 | 0007, 0028 |
| Events tab | Planned(0006): a tab on Pods, a section on Node and kind drawers | 0006; becomes a tab in 0007 |
| Copy name | Done for live kinds | 0005 |
| Columns ▾, filter chips, sort | Done (0009); sort and hidden columns are saved per screen (0024) | 0009, 0024 |
| Edit YAML (E) / Delete … (red, last) | Missing (disabled) | 0031 / 0033 |
| Monitor tab (◔ kinds: Pod, Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, Node) | Done | 0010 (CPU, Memory), 0011 (Network, Disk I/O) |

## Per kind

Cells: **T** table, **Dr** drawer Overview. Read-only gaps name the spec; "mut" lists the mutating actions and their spec.

| Group | Kind | T | Dr | Read-only gaps → spec | Mutating actions → spec |
|---|---|---|---|---|---|
| Cluster | Nodes | Done | Partial | conditions, allocatable, system 0008; CPU/Mem 0010; View pods on node 0009 | shell 0037; cordon, drain, taints, labels 0034; delete 0033 |
| Cluster | Namespaces | Partial | Partial | Pods, CPU req, Memory req columns Done (0012); STUCK box and remaining resources Done (0018; object names not listed); quota section Done (0013; LimitRange row open); "Set as default" Done (0024) | New 0031; delete 0033 |
| Cluster | Events | Planned(0006) | Planned(0006) | Warnings only, Go to object, Copy message: 0006; Pause stream, Filter similar 0009 | — |
| Workloads | Pods | Done | Partial | see [inventory-screens.md](inventory-screens.md) W4 rows | see W4 rows |
| Workloads | Deployments | Done | Done (WHY box, Revisions: 0012) | View logs (all pods) 0019 | scale, restart, roll back, pause 0032; port-forward 0035 |
| Workloads | StatefulSets | Done | Done (pods by ordinal with claims: 0012) | logs 0019 | scale, restart 0032; port-forward 0035 |
| Workloads | DaemonSets | Done | Done (rollout bars, not-ready nodes, WHY box: 0012) | logs 0019 | restart 0032 |
| Workloads | ReplicaSets | Done | Done (Hide inactive 0009; Go to owner 0012) | logs 0019 | — (scale locked by owner) |
| Workloads | Jobs | Done | Done (attempts, BACKOFF LIMIT box: 0012) | logs 0019 | Re-run 0032 |
| Workloads | CronJobs | Done (0012) | Done (next runs, recent jobs: 0012) | logs of last job 0019 | Trigger now, Suspend 0032 |
| Network | Services | Done (0012) | Done (Endpoints, "matches no pods": 0012) | Show in Topology: Done (0022) | port-forward 0035 |
| Network | Ingresses | Done (0016) | Done (TLS column with expiry replaces Ports, TLS section with Secret links and leaf facts, CERTIFICATE box: 0016; Open URL and backend links: 0012) | Show in Topology: Done (0022) | — |
| Network | NetworkPolicies | Done (0013) | Done (rules as sentences, Affects: 0013) | Done (Test traffic: 0023) | — |
| Network | Port Forwarding (local page) | Missing | Missing | — | 0035 |
| Config | ConfigMaps | Done (0012) | Done (Used by, value previews: 0012) | — | Edit, New, Compare with previous 0031 |
| Config | Secrets | Done (0016) | Done (masked Data, per-key Reveal and Reveal all for 30 s, Copy without reveal with private clipboard and 30 s clear, certificate section, CERTIFICATE box, Used by, unused flag: 0016) | — | Edit 0031 |
| Config | HPAs | Done (0013) | Done (metric bars, scaling events, AT MAX box: 0013) | — | Edit min/max 0032 |
| Config | ResourceQuotas | Done (0013) | Done (usage bars, blocked creations: 0013) | — | New, Edit 0031 |
| Config | PDBs | Done (0013) | Done (allowed disruptions, BLOCKS DRAIN, selected pods: 0013) | — | New 0031 |
| Storage | PVCs | Done (0014) | Done (Used %, Usage bars, Mounted by, Go to pod: 0014) | — | Expand 0032 |
| Storage | PVs | Done (0014) | Done (source, node affinity, RELEASED box, Go to claim: 0014) | — | — |
| Storage | StorageClasses | Done (0014) | Done (default ★, PV count, parameters with hidden values, volumes list: 0014) | — | Set default 0032 |
| Access Control | ServiceAccounts | Done (0015) | Done (bound roles, used by pods, cloud identity from three allowlisted annotations, secret names only, CLUSTER ADMIN box: 0015) | Done ("Can do", Check permissions: 0023) | — |
| Access Control | Roles | Done (0015) | Done (rules table, bindings, VERY BROAD box: 0015) | Done (Who can…: 0023) | — |
| Access Control | ClusterRoles | Done (0015) | Done (aggregated, built-in, bound to, VERY BROAD box, Hide system: 0015) | Done (Who can…: 0023) | — |
| Access Control | RoleBindings | Done (0015) | Done (role and service-account links, REVIEW box, Go to role: 0015) | — | New 0031 |
| Access Control | ClusterRoleBindings | Done (0015) | Done (cluster-admin to everyone or service accounts flag, REVIEW box, Hide system: 0015) | — | — |
| Helm | Releases | Done (0017) | Done (history, values and diff masked with 30 s Reveal, manifest, notes, UPGRADE FAILED box: 0017) | — | Roll back, Uninstall 0038 (deferred) |
| Custom Resources | CRDs | Done | Done | Versions, printer columns, schema, Instances, Browse instances Done (0018) | — |
| Custom Resources | Certificates (any discovered CR) | Done | Done | Done for read (0018): printer columns, Expires built-in, status box (NOT READY, UNAVAILABLE, READY UNKNOWN, or {TYPE} FAILING), Conditions, Status and Spec fields, Go to secret link, masked YAML | Renew 0032 |

## Notes

- Live today (12): Nodes, Pods, Namespaces, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs, Services, Ingresses, ConfigMaps.
- Column deviations already decided in 0005 [kind-columns.md](../specs/0005-kind-explorer/kind-columns.md) (kubectl order for Jobs, Last schedule for CronJobs) stay; the specs above only add the missing columns.
- The Certificates item exists only when the cert-manager CRD is installed; 0018 makes every Established CRD an item (N6 in [inventory-shell.md](inventory-shell.md)). cert-manager is not installed on UAT (72 CRDs, none of its), so Certificates is verified by fixture tests (the worked example) and the Argo CD Applications screens, which exercise the same generic path.
- List-level buttons per kind (W7 note 1) follow their action's spec: read-only ones (Hide inactive, Hide system, Reveal all, Who can…, Browse instances, Open URL) ship with the kind; the rest ship with 0031–0035.
