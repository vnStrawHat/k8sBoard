#!/usr/bin/env bash
# k8sBoard lab: a kind cluster inside the VMware VM, driven from the host over SSH.
#
#   tools/kind/lab.sh up        create the cluster, install metrics-server, seed the scenarios,
#                               write ./kind-lab.yml (git-ignored) pointing at the VM
#   tools/kind/lab.sh seed      (re)apply the scenario manifests only
#   tools/kind/lab.sh status    nodes, pods, and the scenario states at a glance
#   tools/kind/lab.sh down      delete the cluster, remove ./kind-lab.yml
#   tools/kind/lab.sh ssh ...   run a command in the VM (no args: interactive shell)
#
# Every setting below can be overridden from the environment, e.g. LAB_HOST=10.0.0.5 lab.sh up.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LAB_HOST="${LAB_HOST:-192.168.13.128}"
LAB_USER="${LAB_USER:-root}"
LAB_KEY="${LAB_KEY:-$ROOT/id_rsa}"
LAB_CLUSTER="${LAB_CLUSTER:-k8sboard-lab}"
LAB_REGISTRY="${LAB_REGISTRY:-registry.example.com}"
# Kubernetes 1.29 matches the UAT cluster; the private registry mirrors Docker Hub and registry.k8s.io.
LAB_NODE_IMAGE="${LAB_NODE_IMAGE:-$LAB_REGISTRY/kindest/node:v1.29.14}"
LAB_IMAGE_BUSYBOX="${LAB_IMAGE_BUSYBOX:-$LAB_REGISTRY/library/busybox:1.36}"
LAB_IMAGE_NGINX="${LAB_IMAGE_NGINX:-$LAB_REGISTRY/library/nginx:1.27-alpine}"
LAB_IMAGE_METRICS="${LAB_IMAGE_METRICS:-$LAB_REGISTRY/metrics-server/metrics-server:v0.7.2}"
LAB_API_PORT="${LAB_API_PORT:-6443}"
REMOTE_DIR="/root/k8sboard-lab"
KUBECONFIG_OUT="$ROOT/kind-lab.yml"

# The corporate proxy must not capture the VM's private address.
unset HTTPS_PROXY HTTP_PROXY https_proxy http_proxy
SSH=(ssh -i "$LAB_KEY" -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=15 "$LAB_USER@$LAB_HOST")
SCP=(scp -q -i "$LAB_KEY" -o BatchMode=yes -o StrictHostKeyChecking=accept-new)

remote() { "${SSH[@]}" "$@"; }

check_vm() {
  remote 'command -v docker >/dev/null || { echo "docker is not installed in the VM" >&2; exit 1; }
          command -v kind >/dev/null || { echo "kind is not installed in the VM" >&2; exit 1; }
          command -v kubectl >/dev/null || { echo "kubectl is not installed in the VM" >&2; exit 1; }
          docker info >/dev/null 2>&1 || { echo "docker is not running or not usable by this user" >&2; exit 1; }
          echo "VM ok: $(hostname) $(nproc) cpu $(free -g | awk "/Mem:/{print \$2}") GB, kind $(kind version | cut -d" " -f2)"'
}

upload() {
  remote "mkdir -p $REMOTE_DIR/scenarios"
  # The manifests carry image placeholders so the registry stays a single setting here.
  local tmp="$ROOT/.tmp/kind-lab-upload"
  rm -rf "$tmp" && mkdir -p "$tmp/scenarios"
  for file in "$ROOT"/tools/kind/scenarios/*.yaml; do
    sed -e "s#__BUSYBOX__#$LAB_IMAGE_BUSYBOX#g" -e "s#__NGINX__#$LAB_IMAGE_NGINX#g" \
        -e "s#__METRICS_SERVER__#$LAB_IMAGE_METRICS#g" "$file" > "$tmp/scenarios/$(basename "$file")"
  done
  sed -e "s#__API_ADDRESS__#$LAB_HOST#g" -e "s#__API_PORT__#$LAB_API_PORT#g" \
      "$ROOT/tools/kind/kind-config.yaml" > "$tmp/kind-config.yaml"
  cp "$ROOT/tools/kind/seed.sh" "$tmp/seed.sh"
  "${SCP[@]}" -r "$tmp/." "$LAB_USER@$LAB_HOST:$REMOTE_DIR/"
  remote "chmod +x $REMOTE_DIR/seed.sh"
}

cmd_up() {
  check_vm
  upload
  if remote "kind get clusters 2>/dev/null | grep -qx '$LAB_CLUSTER'"; then
    echo "cluster $LAB_CLUSTER already exists; use 'down' first to recreate it"
  else
    remote "docker image inspect '$LAB_NODE_IMAGE' >/dev/null 2>&1 || docker pull '$LAB_NODE_IMAGE'"
    remote "kind create cluster --name '$LAB_CLUSTER' --image '$LAB_NODE_IMAGE' --config $REMOTE_DIR/kind-config.yaml --wait 120s"
  fi
  # Pre-pull the scenario images once; kind loads them into every node so pods start without a registry.
  remote "for image in '$LAB_IMAGE_BUSYBOX' '$LAB_IMAGE_NGINX' '$LAB_IMAGE_METRICS'; do
            docker image inspect \"\$image\" >/dev/null 2>&1 || docker pull \"\$image\";
            kind load docker-image --name '$LAB_CLUSTER' \"\$image\";
          done"
  remote "kind get kubeconfig --name '$LAB_CLUSTER'" > "$KUBECONFIG_OUT"
  echo "kubeconfig written to $KUBECONFIG_OUT (context kind-$LAB_CLUSTER)"
  cmd_seed
}

cmd_seed() {
  upload
  remote "cd $REMOTE_DIR && LAB_CLUSTER='$LAB_CLUSTER' ./seed.sh"
}

cmd_status() {
  remote "kubectl --context kind-$LAB_CLUSTER get nodes -o wide --no-headers | awk '{print \$1, \$2, \$6}';
          echo; kubectl --context kind-$LAB_CLUSTER get pods -A --no-headers | awk '\$4!=\"Running\" && \$4!=\"Completed\"{print \$1\"/\"\$2, \$4}';
          echo; kubectl --context kind-$LAB_CLUSTER get hpa,pdb,resourcequota -A --no-headers 2>/dev/null"
}

cmd_down() {
  remote "kind delete cluster --name '$LAB_CLUSTER' 2>/dev/null || true; rm -rf $REMOTE_DIR"
  rm -f "$KUBECONFIG_OUT" && rm -rf "$ROOT/.tmp/kind-lab-upload"
  echo "cluster $LAB_CLUSTER deleted and $KUBECONFIG_OUT removed"
}

case "${1:-}" in
  up) cmd_up ;;
  seed) cmd_seed ;;
  status) cmd_status ;;
  down) cmd_down ;;
  ssh) shift; if [ $# -eq 0 ]; then ssh -i "$LAB_KEY" "$LAB_USER@$LAB_HOST"; else remote "$@"; fi ;;
  *) sed -n '2,11p' "${BASH_SOURCE[0]}"; exit 1 ;;
esac
