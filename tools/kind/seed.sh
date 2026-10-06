#!/usr/bin/env bash
# Runs inside the VM (uploaded by lab.sh): applies the scenario manifests and the states that need
# a command rather than a manifest (a short-lived TLS secret, a tainted node, a cordoned node).
set -euo pipefail

CLUSTER="${LAB_CLUSTER:-k8sboard-lab}"
K=(kubectl --context "kind-$CLUSTER")
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"${K[@]}" apply -f "$DIR/scenarios/00-metrics-server.yaml"
"${K[@]}" apply -f "$DIR/scenarios/10-namespaces.yaml"

# A TLS secret that expires in 5 days, so the certificate warning window has something to show.
tls="$(mktemp -d)"
openssl req -x509 -newkey rsa:2048 -nodes -days 5 -subj "/CN=shop.lab.local" \
  -keyout "$tls/tls.key" -out "$tls/tls.crt" >/dev/null 2>&1
"${K[@]}" -n lab-shop create secret tls shop-tls --cert="$tls/tls.crt" --key="$tls/tls.key" \
  --dry-run=client -o yaml | "${K[@]}" apply -f -
rm -rf "$tls"

for file in "$DIR"/scenarios/2*.yaml "$DIR"/scenarios/3*.yaml; do
  "${K[@]}" apply -f "$file"
done

# Node states: one worker carries a taint, the other a maintenance label; nothing is cordoned so a
# drain can be tried from the app.
data_node="$("${K[@]}" get nodes -l k8sboard.io/pool=data -o name | head -1)"
web_node="$("${K[@]}" get nodes -l k8sboard.io/pool=web -o name | head -1)"
"${K[@]}" taint "$data_node" workload=data:NoSchedule --overwrite >/dev/null
"${K[@]}" label "$web_node" maintenance-window=sunday-02h --overwrite >/dev/null

echo "seeded: $("${K[@]}" get pods -A --no-headers | wc -l) pods across $("${K[@]}" get ns --no-headers | grep -c '^lab-') lab namespaces"
