# Inventory — W7 Kind Explorer (29 kinds + Port Forwarding)

[Back to index](README.md). Source: the `k(...)` definitions in the wireframe script (columns, drawer sections, menu per kind).

## Rules that apply to every kind

| Item | Status | Spec |
|---|---|---|
| YAML tab and "View YAML" | Done (0007); the Y key → 0028 | 0007, 0028 |
| Events tab | Planned(0006): a tab on Pods, a section on Node and kind drawers | 0006; becomes a tab in 0007 |
| Copy name | Done for live kinds | 0005 |
| Columns ▾, filter chips, sort | Missing | 0009 |
| Edit YAML (E) / Delete … (red, last) | Missing (disabled) | 0031 / 0033 |
| Monitor tab (◔ kinds: Pod, Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, Node) | Done | 0010 (CPU, Memory), 0011 (Network, Disk I/O) |

## Per kind

Cells: **T** table, **Dr** drawer Overview. Read-only gaps name the spec; "mut" lists the mutating actions and their spec.

| Group | Kind | T | Dr | Read-only gaps → spec | Mutating actions → spec |
|---|---|---|---|---|---|
| Cluster | Nodes | Done | Partial | conditions, allocatable, system 0008; CPU/Mem 0010; View pods on node 0009 | shell 0037; cordon, drain, taints, labels 0034; delete 0033 |
| Cluster | Namespaces | Partial | Partial | Pods, CPU req, Memory req columns 0012; STUCK box and remaining resources 0018; quota section 0013; "Set as default" 0024 | New 0031; delete 0033 |
| Cluster | Events | Planned(0006) | Planned(0006) | Warnings only, Go to object, Copy message: 0006; Pause stream, Filter similar 0009 | — |
| Workloads | Pods | Done | Partial | see [inventory-screens.md](inventory-screens.md) W4 rows | see W4 rows |
| Workloads | Deployments | Done | Partial | WHY box, Revisions list 0012; View logs (all pods) 0019 | scale, restart, roll back, pause 0032; port-forward 0035 |
| Workloads | StatefulSets | Done | Partial | pods by ordinal with their PVC 0012; logs 0019 | scale, restart 0032; port-forward 0035 |
| Workloads | DaemonSets | Done | Partial | rollout-by-node bars, not-ready nodes, WHY box 0012; logs 0019 | restart 0032 |
| Workloads | ReplicaSets | Done | Partial | Hide inactive 0009; Go to owner 0012; logs 0019 | — (scale locked by owner) |
| Workloads | Jobs | Done | Partial | attempts, BACKOFF LIMIT box 0012; logs 0019 | Re-run 0032 |
| Workloads | CronJobs | Partial | Partial | Next run column, next runs, recent jobs 0012 (cron dependency); logs of last job 0019 | Trigger now, Suspend 0032 |
| Network | Services | Partial | Partial | Endpoints column and list, "matches no pods" 0012; Show in Topology 0022 | port-forward 0035 |
| Network | Ingresses | Partial | Partial | TLS expiry column and CERTIFICATE box 0016; Open URL 0012; Show in Topology 0022 | — |
| Network | NetworkPolicies | Missing | Missing | 0013 (rules as sentences, Affects); Test traffic 0023 | — |
| Network | Port Forwarding (local page) | Missing | Missing | — | 0035 |
| Config | ConfigMaps | Partial | Partial | Used by 0012; values on demand 0012 | Edit, New, Compare with previous 0031 |
| Config | Secrets | Missing | Missing | 0016 (masked, Reveal 30 s, Copy, Used by, unused flag) | Edit 0031 |
| Config | HPAs | Missing | Missing | 0013 (metrics bars, scaling events from 0006) | Edit min/max 0032 |
| Config | ResourceQuotas | Missing | Missing | 0013 (usage bars, blocked Pending pods) | New, Edit 0031 |
| Config | PDBs | Missing | Missing | 0013 (allowed disruptions, BLOCKS DRAIN, selected pods) | New 0031 |
| Storage | PVCs | Missing | Missing | 0014; Used % and inodes: data ready (0011), UI 0014; Go to pod 0014 | Expand 0032 |
| Storage | PVs | Missing | Missing | 0014 (Released cleanup hint) | — |
| Storage | StorageClasses | Missing | Missing | 0014 (default ★, PV count) | Set default 0032 |
| Access Control | ServiceAccounts | Missing | Missing | 0015 (bound roles, used by, cloud identity, can-do); Check permissions 0023 | — |
| Access Control | Roles | Missing | Missing | 0015 (rules table, bindings); Who can… 0023 | — |
| Access Control | ClusterRoles | Missing | Missing | 0015 (aggregated, VERY BROAD box, Hide system); Who can… 0023 | — |
| Access Control | RoleBindings | Missing | Missing | 0015 (two-way links) | New 0031 |
| Access Control | ClusterRoleBindings | Missing | Missing | 0015 (cluster-admin to SA flag, Hide system) | — |
| Helm | Releases | Missing | Missing | 0017 (history, values, manifest, values diff, UPGRADE FAILED box) | Roll back, Uninstall 0038 |
| Custom Resources | CRDs | Missing | Missing | 0018 (versions, printer columns, schema, Browse instances) | — |
| Custom Resources | Certificates (any discovered CR) | Missing | Missing | 0018 (printer columns, schema drawer, status box) | Renew 0032 |

## Notes

- Live today (12): Nodes, Pods, Namespaces, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs, Services, Ingresses, ConfigMaps.
- Column deviations already decided in 0005 [kind-columns.md](../specs/0005-kind-explorer/kind-columns.md) (kubectl order for Jobs, Last schedule for CronJobs) stay; the specs above only add the missing columns.
- The Certificates sidebar item exists only when the cert-manager CRD is installed; 0018 makes every CRD an item (N6 in [inventory-shell.md](inventory-shell.md)).
- List-level buttons per kind (W7 note 1) follow their action's spec: read-only ones (Hide inactive, Hide system, Reveal all, Who can…, Browse instances, Open URL) ship with the kind; the rest ship with 0031–0035.
