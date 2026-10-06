# kind lab in the VMware VM

A disposable Kubernetes 1.29 cluster for trying every write path of k8sBoard. It runs inside the
VMware VM and is driven from the host over SSH; k8sBoard on the host talks to its API server
through the VM's address.

## One-time VM setup

- Linux VM on VMnet8 (NAT), reachable from the host; sshd with key login for the lab user.
- `docker`, `kind` (0.20+), `kubectl` and `openssl` installed; the user can run `docker`.
- Firewall open for 22 and 6443 from the host.
- Images come from the private registry (`LAB_REGISTRY`, default `registry.example.com`); run
  `docker login` once in the VM if it needs credentials.

## Commands (Git Bash on the host, from the project root)

```bash
tools/kind/lab.sh up        # create the cluster, metrics-server, scenarios, ./kind-lab.yml
tools/kind/lab.sh seed      # re-apply the scenarios after you changed them
tools/kind/lab.sh status    # nodes, unhealthy pods, HPA/PDB/quota
tools/kind/lab.sh down      # delete the cluster and ./kind-lab.yml
tools/kind/lab.sh ssh ...   # run a command in the VM
```

Settings are environment variables read by `lab.sh`: `LAB_HOST`, `LAB_USER`, `LAB_KEY`
(default `./id_rsa`, git-ignored), `LAB_REGISTRY`, `LAB_NODE_IMAGE`, `LAB_IMAGE_BUSYBOX`,
`LAB_IMAGE_NGINX`, `LAB_IMAGE_METRICS`, `LAB_API_PORT`.

## Using it from k8sBoard

```bash
k8sboard --kubeconfig D:/TrungKFC-Research/Rust/k8sBoard/kind-lab.yml --context kind-k8sboard-lab
```

Debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1`. That variable is allowed **only**
with `kind-lab.yml`; the UAT kubeconfig stays read-only. Mark the context as Staging or
Production in Settings › Environments to try the guardrails. The kubeconfig is cluster-admin and
git-ignored; never print it.

## What the scenarios provide

| Namespace | State |
|---|---|
| `lab-shop` | `web` Deployment ×3 (Helm-labelled) reading `web-config` and `web-secret`, Service, Ingress with `shop-tls` expiring in 5 days, HPA pinned at max, `catalog-db` StatefulSet ×2 with a PDB that allows 0 disruptions (blocks drains), `debug-toolbox` two-container pod, `deployer` ServiceAccount/Role |
| `lab-batch` | `report-failed` Job (exit 3, backoff exceeded), `report-ok` Job, `cleanup-suspended` CronJob, `heartbeat` CronJob every 2 min |
| `lab-broken` | CrashLoopBackOff, ImagePullBackOff with a missing pull secret, Pending (too big, bad selector), PVC Pending (no class), stuck rollout, flapping sidecar |
| `lab-quota` | ResourceQuota nearly full on `limits.memory`, LimitRange, NetworkPolicy |
| nodes | two workers: one tainted `workload=data:NoSchedule`, one labelled `maintenance-window` |

`seed.sh` is idempotent: run `lab.sh seed` to restore the states after you deleted or changed
things from the app. `lab.sh down` removes everything, including the VM-side upload folder.
