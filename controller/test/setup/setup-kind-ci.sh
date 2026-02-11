#!/usr/bin/env bash

set -o errexit
set -o pipefail
set -o nounset

# Dependency graph:
#   create-kind-cluster --> deploy-metallb
#   create-kind-cluster --> create-local-kind-registry
#   create-local-kind-registry --> push-go-controller-to-local-registry
#   create-local-kind-registry --> push-proxy-to-local-registry
#   build-go-controller-binary --> push-go-controller-to-local-registry
#   build-proxy-binary --> push-proxy-to-local-registry

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
cd "$REPO_ROOT"

TIMINGS_FILE="${REPO_ROOT}/controller/_test/ci-step-timings.log"
CLUSTER_NAME="${CLUSTER_NAME:-kind}"
KIND_NODE_IMAGE="${KIND_NODE_IMAGE:-kindest/node:v1.35.0}"
KIND_REGISTRY_NAME="${KIND_REGISTRY_NAME:-kind-registry}"
KIND_REGISTRY_PORT="${KIND_REGISTRY_PORT:-5000}"
LOCAL_REGISTRY="localhost:${KIND_REGISTRY_PORT}"

CONTROLLER_IMAGE="${CONTROLLER_IMAGE:-${LOCAL_REGISTRY}/agentgateway-controller:ci}"
PROXY_IMAGE="${PROXY_IMAGE:-${LOCAL_REGISTRY}/agentgateway-proxy:ci}"

mkdir -p "$(dirname "${TIMINGS_FILE}")"
: >"${TIMINGS_FILE}"

get-tag () {
  if [[ -n "${TAG:-""}" ]]
  then
    echo ${TAG}
  else
    echo `date +%s`
  fi
}
export TAG="$(get-tag)"

run_timed_step() {
  local step_name="$1"
  shift

  local start_seconds
  local end_seconds
  local elapsed_seconds
  local rc

  start_seconds="$(date +%s)"
  echo "==> Step started: ${step_name}" >&2

  if "$@"; then
    rc=0
  else
    rc=$?
  fi

  end_seconds="$(date +%s)"
  elapsed_seconds=$((end_seconds - start_seconds))
  printf '%s: %ss\n' "${step_name}" "${elapsed_seconds}" >>"${TIMINGS_FILE}"

  if [[ "${rc}" -ne 0 ]]; then
    echo "Step failed: ${step_name} (exit ${rc})" >&2
  else
    echo "==> Step completed: ${step_name}" >&2
  fi

  return "${rc}"
}

step_create_kind_cluster() {
  cat <<EOF | kind create cluster --name "${CLUSTER_NAME}" --image "${KIND_NODE_IMAGE}" --config=-
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
kubeadmConfigPatches:
  - |
    kind: ClusterConfiguration
    metadata:
      name: config
    controllerManager:
      extraArgs:
        "kube-api-burst": "500"
        "kube-api-qps": "250"
networking:
  dnsSearch: []
nodes:
- role: control-plane
  labels:
    topology.kubernetes.io/region: region
    topology.kubernetes.io/zone: zone
EOF
}

step_deploy_metallb() {
  # TODO: deploy metallb after the cluster exists.
  # Good starting point from this repo:
   kubectl apply -f  "${REPO_ROOT}/controller/test/setup/metallb.yaml"
   kubectl wait -n metallb-system pod --timeout=120s -l app=metallb --for=condition=Ready
  if [ -z "${METALLB_IPS4+x}" ]; then
    # Take IPs from the end of the docker kind network subnet to use for MetalLB IPs
    DOCKER_KIND_SUBNET="$(docker inspect kind | jq '.[0].IPAM.Config[0].Subnet' -r)"
    METALLB_IPS4=()
    while read -r ip; do
      METALLB_IPS4+=("$ip")
    done < <(cidr_to_ips "$DOCKER_KIND_SUBNET" | tail -n 100)
    METALLB_IPS6=()
    if [[ "$(docker inspect kind | jq '.[0].IPAM.Config | length' -r)" == 2 ]]; then
      # Two configs? Must be dual stack.
      DOCKER_KIND_SUBNET="$(docker inspect kind | jq '.[0].IPAM.Config[1].Subnet' -r)"
      while read -r ip; do
        METALLB_IPS6+=("$ip")
      done < <(cidr_to_ips "$DOCKER_KIND_SUBNET" | tail -n 100)
    fi
  fi

  # Give this cluster of those IPs
  RANGE="["
  for i in {0..19}; do
    RANGE+="${METALLB_IPS4[1]},"
    METALLB_IPS4=("${METALLB_IPS4[@]:1}")
    if [[ "${#METALLB_IPS6[@]}" != 0 ]]; then
      RANGE+="${METALLB_IPS6[1]},"
      METALLB_IPS6=("${METALLB_IPS6[@]:1}")
    fi
  done
  RANGE="${RANGE%?}]"

  echo '
apiVersion: metallb.io/v1beta1
kind: IPAddressPool
metadata:
  name: default-pool
  namespace: metallb-system
spec:
  addresses: '"$RANGE"'
---
apiVersion: metallb.io/v1beta1
kind: L2Advertisement
metadata:
  name: default-l2
  namespace: metallb-system
spec:
  ipAddressPools:
  - default-pool
' | kubectl apply -f -
}

function cidr_to_ips() {
    CIDR="$1"
    # cidr_to_ips returns a list of single IPs from a CIDR. We skip 1000 (since they are likely to be allocated
    # already to other services), then pick the next 100.
    python3 - <<EOF
from ipaddress import ip_network, IPv6Network;
from itertools import islice;

net = ip_network('$CIDR')
net_bits = 128 if type(net) == IPv6Network else 32;
net_len = pow(2, net_bits - net.prefixlen)
start, end = int(net_len / 4 * 3), net_len
if net_len > 2000:
  start, end = 1000, 2000

[print(str(ip) + "/" + str(ip.max_prefixlen)) for ip in islice(ip_network('$CIDR').hosts(), start, end)]
EOF
}

function ips_to_cidrs() {
  IP_RANGE_START="$1"
  IP_RANGE_END="$2"
  python3 - <<EOF
from ipaddress import summarize_address_range, IPv4Address
[ print(n.compressed) for n in summarize_address_range(IPv4Address(u'$IP_RANGE_START'), IPv4Address(u'$IP_RANGE_END')) ]
EOF
}

function step_create_local_kind_registry() {
  # create a registry container if it not running already
  running="$(docker inspect -f '{{.State.Running}}' "${KIND_REGISTRY_NAME}" 2>/dev/null || true)"
  if [[ "${running}" != 'true' ]]; then
      docker run \
        -d --restart=always -p "${KIND_REGISTRY_PORT}:5000" --name "${KIND_REGISTRY_NAME}" \
        gcr.io/istio-testing/registry:2

    # Allow kind nodes to reach the registry
    docker network connect "kind" "${KIND_REGISTRY_NAME}"
  fi

    KIND_REGISTRY_DIR="/etc/containerd/certs.d/localhost:${KIND_REGISTRY_PORT}"
    for node in $(kind get nodes --name="${CLUSTER_NAME}"); do
      docker exec "${node}" mkdir -p "${KIND_REGISTRY_DIR}"
      cat <<EOF | docker exec -i "${node}" cp /dev/stdin "${KIND_REGISTRY_DIR}/hosts.toml"
[host."http://${KIND_REGISTRY_NAME}:5000"]
EOF
  done
}

function step_build_go_controller_binary() {
  make -C controller agentgateway-controller
}

function step_push_go_controller_to_local_registry() {
  make -C controller agentgateway-controller-docker
}

function step_build_proxy_binary() {
   (cd "${REPO_ROOT}" && DRY_RUN=true ./tools/proxy-dev-build quick-release)
}

function step_push_proxy_to_local_registry() {
   (cd "${REPO_ROOT}" && ./tools/proxy-dev-build quick-release)
}

function step_deploy_helm() {
	helm upgrade -i --create-namespace --namespace agentgateway-system agentgateway-crds ./controller/install/helm/agentgateway-crds/
	helm upgrade -i --namespace agentgateway-system agentgateway ./controller/install/helm/agentgateway  \
	  --set image.registry=localhost:5000 --set-string image.tag="${TAG}" "$@"
}

function main() {
  local rc=0

  echo "Timings will be written to: ${TIMINGS_FILE}"

  # Start root steps that do not depend on each other.
  run_timed_step "create-kind-cluster" step_create_kind_cluster &
  local pid_kind="$!"
  run_timed_step "build-go-controller-binary" step_build_go_controller_binary &
  local pid_build_controller="$!"
  run_timed_step "build-proxy-binary" step_build_proxy_binary &
  local pid_build_proxy="$!"

  # Registry and MetalLB both require the kind cluster first.
  if ! wait "${pid_kind}"; then
    echo "create-kind-cluster failed; stopping dependent steps." >&2
    wait || true
    exit 1
  fi

  run_timed_step "create-local-kind-registry" step_create_local_kind_registry &
  local pid_registry="$!"
  run_timed_step "deploy-metallb" step_deploy_metallb &
  local pid_metallb="$!"

  # Pushes require the registry.
  if ! wait "${pid_registry}"; then
    echo "create-local-kind-registry failed; skipping push steps." >&2
    wait || true
    exit 1
  fi

  if wait "${pid_build_controller}"; then
    run_timed_step "push-go-controller-to-local-registry" step_push_go_controller_to_local_registry &
    local pid_push_controller="$!"
  else
    echo "build-go-controller-binary failed; skipping controller push." >&2
    rc=1
  fi

  if wait "${pid_build_proxy}"; then
    run_timed_step "push-proxy-to-local-registry" step_push_proxy_to_local_registry &
    local pid_push_proxy="$!"
  else
    echo "build-proxy-binary failed; skipping proxy push." >&2
    rc=1
  fi

  if ! wait "${pid_metallb}"; then
    rc=1
  fi

  if [[ -n "${pid_push_controller:-}" ]] && ! wait "${pid_push_controller}"; then
    rc=1
  fi

  if [[ -n "${pid_push_proxy:-}" ]] && ! wait "${pid_push_proxy}"; then
    rc=1
  fi

  if [[ "${rc}" != 0 ]]; then
    exit "${rc}"
  fi

  run_timed_step "deploy-helm" step_deploy_helm "$@"
}

main "$@"
