#!/usr/bin/env bash

set -euo pipefail

KIND_DOCKER_NETWORK="${KIND_DOCKER_NETWORK:-kind}"
METALLB_NAMESPACE="${METALLB_NAMESPACE:-metallb-system}"
METALLB_POOL_NAME="${METALLB_POOL_NAME:-default-pool}"
METALLB_L2_NAME="${METALLB_L2_NAME:-default-l2}"
METALLB_CONFORMANCE_POOLS="${METALLB_CONFORMANCE_POOLS:-false}"
METALLB_STATIC_POOL_NAME="${METALLB_STATIC_POOL_NAME:-static-conformance-pool}"
METALLB_AUTO_ASSIGN_COUNT="${METALLB_AUTO_ASSIGN_COUNT:-20}"
METALLB_IP_COUNT="${METALLB_IP_COUNT:-51}"

network_json="$(docker inspect "${KIND_DOCKER_NETWORK}")"

echo "Configuring MetalLB address pools from Docker network ${KIND_DOCKER_NETWORK}" >&2

DOCKER_NETWORK_JSON="${network_json}" \
METALLB_NAMESPACE="${METALLB_NAMESPACE}" \
METALLB_POOL_NAME="${METALLB_POOL_NAME}" \
METALLB_L2_NAME="${METALLB_L2_NAME}" \
METALLB_CONFORMANCE_POOLS="${METALLB_CONFORMANCE_POOLS}" \
METALLB_STATIC_POOL_NAME="${METALLB_STATIC_POOL_NAME}" \
METALLB_AUTO_ASSIGN_COUNT="${METALLB_AUTO_ASSIGN_COUNT}" \
METALLB_IP_COUNT="${METALLB_IP_COUNT}" \
METALLB_IP_RANGE="${METALLB_IP_RANGE:-}" \
python3 - <<'PY' | kubectl apply -f -
import json
import os
from ipaddress import IPv4Address, IPv6Network, ip_network
from itertools import islice


def bool_env(name):
    return os.environ.get(name, "").lower() in ("1", "true", "yes")


def quote(value):
    return json.dumps(str(value))


def emit_pool(name, namespace, addresses, auto_assign=True):
    print("apiVersion: metallb.io/v1beta1")
    print("kind: IPAddressPool")
    print("metadata:")
    print(f"  name: {name}")
    print(f"  namespace: {namespace}")
    print("spec:")
    if not auto_assign:
        print("  autoAssign: false")
    print("  addresses:")
    for address in addresses:
        print(f"  - {quote(address)}")


def emit_l2(name, namespace, pools):
    print("---")
    print("apiVersion: metallb.io/v1beta1")
    print("kind: L2Advertisement")
    print("metadata:")
    print(f"  name: {name}")
    print(f"  namespace: {namespace}")
    print("spec:")
    print("  ipAddressPools:")
    for pool in pools:
        print(f"  - {pool}")


def default_range(subnet, count):
    first = int(subnet.network_address) + 1
    last = int(subnet.broadcast_address) - 1
    if last < first:
        raise SystemExit(f"subnet {subnet} has no usable IPv4 addresses")

    start = max(first, last - count + 1)
    return f"{IPv4Address(start)}-{IPv4Address(last)}"

network = json.loads(os.environ["DOCKER_NETWORK_JSON"])[0]
subnets = [
    item.get("Subnet")
    for item in network.get("IPAM", {}).get("Config", [])
    if item.get("Subnet")
]
networks = [ip_network(subnet) for subnet in subnets]
ipv4_subnets = [subnet for subnet in networks if subnet.version == 4]
ipv6_subnets = [subnet for subnet in networks if subnet.version == 6]

namespace = os.environ["METALLB_NAMESPACE"]
pool_name = os.environ["METALLB_POOL_NAME"]
l2_name = os.environ["METALLB_L2_NAME"]
static_pool_name = os.environ["METALLB_STATIC_POOL_NAME"]
override = os.environ.get("METALLB_IP_RANGE", "")
conformance_pools = bool_env("METALLB_CONFORMANCE_POOLS")
auto_assign_count = int(os.environ["METALLB_AUTO_ASSIGN_COUNT"])
ip_count = int(os.environ["METALLB_IP_COUNT"])

if not ipv4_subnets:
    raise SystemExit("no IPv4 subnet found in Docker network")

if not conformance_pools:
    if override:
        default_addresses = [override]
    else:
        default_addresses = [default_range(ipv4_subnets[0], ip_count)]

    emit_pool(pool_name, namespace, default_addresses)
    emit_l2(l2_name, namespace, [pool_name])
    raise SystemExit(0)


def conformance_candidates(subnet):
    bits = 128 if isinstance(subnet, IPv6Network) else 32
    net_len = 2 ** (bits - subnet.prefixlen)
    start, end = int(net_len / 4 * 3), net_len
    if net_len > 2000:
        start, end = 1000, 2000

    return list(islice(subnet.hosts(), start, end))[-100:]


ipv4_candidates = conformance_candidates(ipv4_subnets[0])
if len(ipv4_candidates) < auto_assign_count + 2:
    raise SystemExit(f"not enough IPv4 addresses found in subnet {ipv4_subnets[0]}")

static_address = f"{ipv4_candidates[-1]}/32"
default_addresses = [
    f"{address}/32" for address in ipv4_candidates[1:auto_assign_count + 1]
]

if ipv6_subnets:
    ipv6_candidates = conformance_candidates(ipv6_subnets[0])
    default_addresses.extend(
        f"{address}/128" for address in ipv6_candidates[1:auto_assign_count + 1]
    )

emit_pool(pool_name, namespace, default_addresses)
print("---")
emit_pool(static_pool_name, namespace, [static_address], auto_assign=False)
emit_l2(l2_name, namespace, [pool_name, static_pool_name])
PY
