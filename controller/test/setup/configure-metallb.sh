#!/usr/bin/env bash

set -euo pipefail

KIND_DOCKER_NETWORK="${KIND_DOCKER_NETWORK:-kind}"
METALLB_NAMESPACE="${METALLB_NAMESPACE:-metallb-system}"
METALLB_POOL_NAME="${METALLB_POOL_NAME:-default-pool}"
METALLB_L2_NAME="${METALLB_L2_NAME:-default-l2}"

if [[ -z "${METALLB_IP_RANGE:-}" ]]; then
  network_json="$(docker inspect "${KIND_DOCKER_NETWORK}")"
  METALLB_IP_RANGE="$(
    DOCKER_NETWORK_JSON="${network_json}" python3 - <<'PY'
import json
import os
from ipaddress import IPv4Address, ip_network

network = json.loads(os.environ["DOCKER_NETWORK_JSON"])[0]
subnets = [
    item.get("Subnet")
    for item in network.get("IPAM", {}).get("Config", [])
    if item.get("Subnet")
]
ipv4_subnets = [ip_network(subnet) for subnet in subnets if ":" not in subnet]
if not ipv4_subnets:
    raise SystemExit("no IPv4 subnet found in Docker network")

subnet = ipv4_subnets[0]
first = int(subnet.network_address) + 1
last = int(subnet.broadcast_address) - 1
if last < first:
    raise SystemExit(f"subnet {subnet} has no usable IPv4 addresses")

start = max(first, last - 50)
print(f"{IPv4Address(start)}-{IPv4Address(last)}")
PY
  )"
fi

echo "Configuring MetalLB address pool ${METALLB_POOL_NAME} with ${METALLB_IP_RANGE}"

kubectl apply -f - <<EOF
apiVersion: metallb.io/v1beta1
kind: IPAddressPool
metadata:
  name: ${METALLB_POOL_NAME}
  namespace: ${METALLB_NAMESPACE}
spec:
  addresses:
  - ${METALLB_IP_RANGE}
---
apiVersion: metallb.io/v1beta1
kind: L2Advertisement
metadata:
  name: ${METALLB_L2_NAME}
  namespace: ${METALLB_NAMESPACE}
spec:
  ipAddressPools:
  - ${METALLB_POOL_NAME}
EOF
