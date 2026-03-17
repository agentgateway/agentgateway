#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
export PATH="${REPO_ROOT}/tools:${PATH}"

PROFILE="local"
QUIET="false"
ACTION=""
ACTION_ARGS=()
CONFIG_JSON=""

usage() {
  cat <<'EOF'
usage: tools/dev-env.sh [--profile PROFILE] [--quiet] <action> [-- <args>]

Public actions:
  up
  down
  status

Internal actions:
  config
  ready
  cluster
  crds
  images
  preload
  deploy
  collect-artifacts
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="$2"
      shift 2
      ;;
    --quiet)
      QUIET="true"
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    --)
      shift
      ACTION_ARGS+=("$@")
      break
      ;;
    *)
      if [[ -z "${ACTION}" ]]; then
        ACTION="$1"
      else
        ACTION_ARGS+=("$1")
      fi
      shift
      ;;
  esac
done

if [[ -z "${ACTION}" ]]; then
  usage >&2
  exit 2
fi

resolve_config() {
  CONFIG_JSON="$(
    python3 "${REPO_ROOT}/tools/dev_env_config.py" --profile "${PROFILE}"
  )"
}

config_value() {
  jq -r --arg key "$1" '.[$key]' <<<"${CONFIG_JSON}"
}

refresh_config_json() {
  CONFIG_JSON="$(
    jq \
      --arg cluster_name "${CLUSTER_NAME}" \
      --arg cluster_resource_name "${CLUSTER_RESOURCE_NAME}" \
      --arg cluster_context "${CLUSTER_CONTEXT}" \
      --arg install_namespace "${INSTALL_NAMESPACE}" \
      --arg registry_name "${REGISTRY_NAME}" \
      --arg registry_host "${REGISTRY_HOST}" \
      --argjson registry_port "${REGISTRY_PORT}" \
      --arg registry_host_base "${DEFAULT_REGISTRY_HOST_BASE}" \
      --arg image_tag "${IMAGE_TAG}" \
      --arg proxy_build_profile "${PROXY_BUILD_PROFILE}" \
      --arg test_mode "${TEST_MODE}" \
      --argjson enable_inference_extension "${ENABLE_INFERENCE_EXTENSION}" \
      --arg controller_log_level "${CONTROLLER_LOG_LEVEL}" \
      --arg controller_image_repository "${CONTROLLER_IMAGE_REPOSITORY}" \
      --arg proxy_image_repository "${PROXY_IMAGE_REPOSITORY}" \
      --arg testbox_image_repository "${TESTBOX_IMAGE_REPOSITORY}" \
      --argjson registry_name_explicit "${REGISTRY_NAME_EXPLICIT}" \
      --argjson registry_host_explicit "${REGISTRY_HOST_EXPLICIT}" \
      --argjson registry_port_explicit "${REGISTRY_PORT_EXPLICIT}" \
      '.cluster_name = $cluster_name
      | .cluster_resource_name = $cluster_resource_name
      | .cluster_context = $cluster_context
      | .install_namespace = $install_namespace
      | .registry_name = $registry_name
      | .registry_host = $registry_host
      | .registry_port = $registry_port
      | .registry_host_base = $registry_host_base
      | .image_tag = $image_tag
      | .proxy_build_profile = $proxy_build_profile
      | .test_mode = $test_mode
      | .enable_inference_extension = $enable_inference_extension
      | .controller_log_level = $controller_log_level
      | .controller_image_repository = $controller_image_repository
      | .proxy_image_repository = $proxy_image_repository
      | .testbox_image_repository = $testbox_image_repository
      | .registry_name_explicit = $registry_name_explicit
      | .registry_host_explicit = $registry_host_explicit
      | .registry_port_explicit = $registry_port_explicit' \
      <<<"${CONFIG_JSON}"
  )"
}

resolve_config

CLUSTER_NAME="$(config_value cluster_name)"
CLUSTER_RESOURCE_NAME="$(config_value cluster_resource_name)"
CLUSTER_CONTEXT="$(config_value cluster_context)"
INSTALL_NAMESPACE="$(config_value install_namespace)"
REGISTRY_NAME="$(config_value registry_name)"
REGISTRY_HOST="$(config_value registry_host)"
REGISTRY_PORT="$(config_value registry_port)"
DEFAULT_REGISTRY_HOST_BASE="$(config_value registry_host_base)"
IMAGE_TAG="$(config_value image_tag)"
PROXY_BUILD_PROFILE="$(config_value proxy_build_profile)"
TEST_MODE="$(config_value test_mode)"
ENABLE_INFERENCE_EXTENSION="$(config_value enable_inference_extension)"
CONTROLLER_LOG_LEVEL="$(config_value controller_log_level)"
CONTROLLER_IMAGE_REPOSITORY="$(config_value controller_image_repository)"
PROXY_IMAGE_REPOSITORY="$(config_value proxy_image_repository)"
TESTBOX_IMAGE_REPOSITORY="$(config_value testbox_image_repository)"
REGISTRY_NAME_EXPLICIT="$(config_value registry_name_explicit)"
REGISTRY_HOST_EXPLICIT="$(config_value registry_host_explicit)"
REGISTRY_PORT_EXPLICIT="$(config_value registry_port_explicit)"

CTLPTL_TEMPLATE_FILE="${REPO_ROOT}/dev/cluster/local.ctlptl.yaml"
METALLB_FILE="${REPO_ROOT}/controller/test/setup/metallb.yaml"
HELM_DEV_VALUES_FILE="${REPO_ROOT}/controller/hack/helm/dev.yaml"
KUBECTL=(kubectl --context "${CLUSTER_CONTEXT}")
REGISTRY_CONTAINER_PORT="5000"

run_make() {
  make --no-print-directory "$@"
}

run_controller_make() {
  make --no-print-directory -C "${REPO_ROOT}/controller" "$@"
}

cidr_to_ips() {
  local cidr="$1"
  python3 - <<EOF
from ipaddress import ip_network, IPv6Network
from itertools import islice

net = ip_network("${cidr}")
net_bits = 128 if isinstance(net, IPv6Network) else 32
net_len = pow(2, net_bits - net.prefixlen)
start, end = int(net_len / 4 * 3), net_len
if net_len > 2000:
    start, end = 1000, 2000

for ip in islice(net.hosts(), start, end):
    print(f"{ip}/{ip.max_prefixlen}")
EOF
}

cluster_exists() {
  kind get clusters 2>/dev/null | grep -Fxq "${CLUSTER_NAME}"
}

registry_container_exists() {
  docker inspect "${REGISTRY_NAME}" >/dev/null 2>&1
}

registry_published_host_port() {
  docker port "${REGISTRY_NAME}" "${REGISTRY_CONTAINER_PORT}/tcp" 2>/dev/null | head -n1 | awk -F: '{print $NF}'
}

host_port_available() {
  local port="$1"
  python3 - <<EOF
import socket
import sys

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try:
    sock.bind(("0.0.0.0", ${port}))
except OSError:
    sys.exit(1)
finally:
    sock.close()
sys.exit(0)
EOF
}

render_ctlptl_config() {
  local output="$1"
  sed \
    -e "s/__CLUSTER_NAME__/${CLUSTER_NAME}/g" \
    -e "s/__CLUSTER_RESOURCE_NAME__/${CLUSTER_RESOURCE_NAME}/g" \
    -e "s/__REGISTRY_NAME__/${REGISTRY_NAME}/g" \
    -e "s/__REGISTRY_PORT__/${REGISTRY_PORT}/g" \
    "${CTLPTL_TEMPLATE_FILE}" >"${output}"
}

with_rendered_ctlptl_config() {
  local ctlptl_file
  local rc

  ctlptl_file="$(mktemp "${TMPDIR:-/tmp}/agentgateway-ctlptl.XXXXXX")"
  render_ctlptl_config "${ctlptl_file}"
  set +e
  "$@" "${ctlptl_file}"
  rc=$?
  set -e
  rm -f "${ctlptl_file}"
  return "${rc}"
}

print_config() {
  resolve_registry_conflicts
  refresh_config_json
  printf '%s\n' "${CONFIG_JSON}"
}

registry_conflict_hint() {
  cat >&2 <<EOF
Registry bootstrap conflict for ${REGISTRY_NAME} on ${REGISTRY_HOST}.
Either free the port, or rerun with overrides such as:
  REGISTRY_PORT=5001 REGISTRY_HOST=${DEFAULT_REGISTRY_HOST_BASE}:5001 make env-up PROFILE=${PROFILE}
EOF
}

named_registry_conflict_hint() {
  cat >&2 <<EOF
Registry bootstrap conflict for ${REGISTRY_NAME}.
Either:
  - reuse the existing ${REGISTRY_NAME} settings by removing the REGISTRY_HOST/REGISTRY_PORT override
  - remove or rename the existing ${REGISTRY_NAME} container
  - choose a different REGISTRY_NAME together with REGISTRY_PORT/REGISTRY_HOST
EOF
}

resolve_registry_conflicts() {
  local existing_named_port=""
  local published_owner=""

  if registry_container_exists; then
    if [[ "$(docker inspect -f '{{.State.Running}}' "${REGISTRY_NAME}")" != "true" ]]; then
      echo "registry container \"${REGISTRY_NAME}\" exists but is not running." >&2
      registry_conflict_hint
      exit 1
    fi

    existing_named_port="$(registry_published_host_port || true)"
    if [[ -n "${existing_named_port}" ]]; then
      if [[ "${REGISTRY_HOST_EXPLICIT}" == "true" || "${REGISTRY_PORT_EXPLICIT}" == "true" ]]; then
        if [[ "${existing_named_port}" != "${REGISTRY_PORT}" ]]; then
          echo "registry container \"${REGISTRY_NAME}\" already publishes host port ${existing_named_port}, not ${REGISTRY_PORT}." >&2
          named_registry_conflict_hint
          exit 1
        fi
      else
        REGISTRY_PORT="${existing_named_port}"
        REGISTRY_HOST="${DEFAULT_REGISTRY_HOST_BASE}:${REGISTRY_PORT}"
      fi
    fi
  fi

  published_owner="$(docker ps --filter "publish=${REGISTRY_PORT}" --format '{{.Names}}' | head -n1 || true)"
  if [[ -n "${published_owner}" && "${published_owner}" != "${REGISTRY_NAME}" ]]; then
    echo "host port ${REGISTRY_PORT} is already published by container \"${published_owner}\"." >&2
    registry_conflict_hint
    exit 1
  fi

  if [[ -z "${published_owner}" ]] && ! host_port_available "${REGISTRY_PORT}"; then
    echo "host port ${REGISTRY_PORT} is already in use by another process." >&2
    registry_conflict_hint
    exit 1
  fi

  refresh_config_json
}

ensure_cluster() {
  resolve_registry_conflicts
  with_rendered_ctlptl_config ctlptl_apply
}

ctlptl_apply() {
  local ctlptl_file="$1"
  (
    cd "${REPO_ROOT}"
    ctlptl apply -f "${ctlptl_file}"
  )
}

ensure_install_namespace() {
  "${KUBECTL[@]}" create namespace "${INSTALL_NAMESPACE}" --dry-run=client -o yaml | "${KUBECTL[@]}" apply -f -
}

configure_metallb_pool() {
  local docker_kind_subnet
  local range="["
  local -a metallb_ips4=()
  local -a metallb_ips6=()

  docker_kind_subnet="$(docker inspect kind | jq '.[0].IPAM.Config[0].Subnet' -r)"
  while IFS= read -r ip; do
    metallb_ips4+=("${ip}")
  done < <(cidr_to_ips "${docker_kind_subnet}" | tail -n 100)

  if [[ "$(docker inspect kind | jq '.[0].IPAM.Config | length' -r)" == "2" ]]; then
    docker_kind_subnet="$(docker inspect kind | jq '.[0].IPAM.Config[1].Subnet' -r)"
    while IFS= read -r ip; do
      metallb_ips6+=("${ip}")
    done < <(cidr_to_ips "${docker_kind_subnet}" | tail -n 100)
  fi

  for i in {0..19}; do
    range+="${metallb_ips4[1]},"
    metallb_ips4=("${metallb_ips4[@]:1}")
    if [[ "${#metallb_ips6[@]}" -ne 0 ]]; then
      range+="${metallb_ips6[1]},"
      metallb_ips6=("${metallb_ips6[@]:1}")
    fi
  done
  range="${range%,}]"

  "${KUBECTL[@]}" apply -f - <<EOF
apiVersion: metallb.io/v1beta1
kind: IPAddressPool
metadata:
  name: default-pool
  namespace: metallb-system
spec:
  addresses: ${range}
---
apiVersion: metallb.io/v1beta1
kind: L2Advertisement
metadata:
  name: default-l2
  namespace: metallb-system
spec:
  ipAddressPools:
    - default-pool
EOF
}

install_crds() {
  ensure_install_namespace
  run_controller_make gw-api-crds gie-crds
  "${KUBECTL[@]}" apply -f "${METALLB_FILE}"
  "${KUBECTL[@]}" rollout status -n metallb-system deployment/controller --timeout=120s
  "${KUBECTL[@]}" rollout status -n metallb-system daemonset/speaker --timeout=120s
  configure_metallb_pool
}

build_controller_image() {
  run_controller_make \
    IMAGE_REGISTRY="${REGISTRY_HOST}" \
    AGENTGATEWAY_IMAGE_REPO="${CONTROLLER_IMAGE_REPOSITORY}" \
    TAG="${IMAGE_TAG}" \
    agentgateway-controller-docker-local
}

build_proxy_image() {
  (
    cd "${REPO_ROOT}"
    if [[ "$(uname -s)" == "Darwin" ]]; then
      run_make IMAGE_TAG="${IMAGE_TAG}" docker-ci
      docker tag "ghcr.io/agentgateway/agentgateway:${IMAGE_TAG}" "${REGISTRY_HOST}/${PROXY_IMAGE_REPOSITORY}:${IMAGE_TAG}"
      docker push "${REGISTRY_HOST}/${PROXY_IMAGE_REPOSITORY}:${IMAGE_TAG}"
    else
      IMAGE_REGISTRY="${REGISTRY_HOST}" IMAGE_REPOSITORY="${PROXY_IMAGE_REPOSITORY}" TAG="${IMAGE_TAG}" TIMINGS="${TIMINGS:-}" ./tools/proxy-dev-build "${PROXY_BUILD_PROFILE}"
    fi
  )
}

build_images() {
  resolve_registry_conflicts
  build_controller_image
  build_proxy_image
}

preload_images() {
  case "${TEST_MODE}" in
    e2e)
      run_controller_make \
        IMAGE_REGISTRY="${REGISTRY_HOST}" \
        TESTBOX_IMAGE_REPO="${TESTBOX_IMAGE_REPOSITORY}" \
        CLUSTER_NAME="${CLUSTER_NAME}" \
        testbox-docker \
        kind-load-testbox
      docker exec "${CLUSTER_NAME}-control-plane" crictl pull gcr.io/solo-public/docs/ai-guardrail-webhook@sha256:01f81b20ae016d123f018841c62daff7f6f44d0dec9189ecf591b3e99753c6b1
      docker exec "${CLUSTER_NAME}-control-plane" crictl pull docker.io/otel/opentelemetry-collector-contrib:0.143.0
      docker exec "${CLUSTER_NAME}-control-plane" crictl pull docker.io/library/redis:7.4.3
      docker exec "${CLUSTER_NAME}-control-plane" crictl pull docker.io/envoyproxy/ratelimit:3e085e5b
      ;;
    *)
      ;;
  esac
}

deploy_helm() {
  local values_args=()
  if [[ -f "${HELM_DEV_VALUES_FILE}" ]]; then
    values_args=(-f "${HELM_DEV_VALUES_FILE}")
  fi

  ensure_install_namespace
  (
    cd "${REPO_ROOT}"
    helm upgrade -i --create-namespace --namespace "${INSTALL_NAMESPACE}" agentgateway-crds ./controller/install/helm/agentgateway-crds
    helm upgrade -i --namespace "${INSTALL_NAMESPACE}" agentgateway ./controller/install/helm/agentgateway \
      --set "image.registry=${REGISTRY_HOST}" \
      --set-string "image.tag=${IMAGE_TAG}" \
      --set "controller.image.repository=${CONTROLLER_IMAGE_REPOSITORY}" \
      --set "controller.logLevel=${CONTROLLER_LOG_LEVEL}" \
      --set "inferenceExtension.enabled=${ENABLE_INFERENCE_EXTENSION}" \
      "${values_args[@]}" \
      "${ACTION_ARGS[@]}"
  )
}

check_registry_ready() {
  local existing_named_port=""

  registry_container_exists || {
    echo "registry container \"${REGISTRY_NAME}\" is missing." >&2
    return 1
  }
  [[ "$(docker inspect -f '{{.State.Running}}' "${REGISTRY_NAME}")" == "true" ]] || {
    echo "registry container \"${REGISTRY_NAME}\" is not running." >&2
    return 1
  }

  existing_named_port="$(registry_published_host_port || true)"
  [[ -n "${existing_named_port}" ]] || {
    echo "registry container \"${REGISTRY_NAME}\" does not publish a host port." >&2
    return 1
  }
  [[ "${existing_named_port}" == "${REGISTRY_PORT}" ]] || {
    echo "registry container \"${REGISTRY_NAME}\" publishes ${existing_named_port}, expected ${REGISTRY_PORT}." >&2
    return 1
  }
}

check_addons_ready() {
  "${KUBECTL[@]}" get namespace "${INSTALL_NAMESPACE}" >/dev/null
  "${KUBECTL[@]}" get namespace metallb-system >/dev/null
  "${KUBECTL[@]}" get crd gatewayclasses.gateway.networking.k8s.io >/dev/null
  "${KUBECTL[@]}" get crd gateways.gateway.networking.k8s.io >/dev/null
  "${KUBECTL[@]}" get crd inferencepools.inference.networking.x-k8s.io >/dev/null
  "${KUBECTL[@]}" rollout status -n metallb-system deployment/controller --timeout=5s >/dev/null
  "${KUBECTL[@]}" rollout status -n metallb-system daemonset/speaker --timeout=5s >/dev/null
}

ready() {
  resolve_registry_conflicts
  cluster_exists || {
    echo "kind cluster \"${CLUSTER_NAME}\" is not running." >&2
    return 1
  }
  "${KUBECTL[@]}" cluster-info >/dev/null
  check_registry_ready
  check_addons_ready

  if [[ "${QUIET}" != "true" ]]; then
    cat <<EOF
environment ready
  profile: ${PROFILE}
  cluster context: ${CLUSTER_CONTEXT}
  registry: ${REGISTRY_HOST}
  namespace: ${INSTALL_NAMESPACE}
EOF
  fi
}

collect_artifacts() {
  local output_dir="${REPO_ROOT}/controller/_test/bug_report/${CLUSTER_NAME}"
  mkdir -p "${output_dir}"
  "${KUBECTL[@]}" get pods -A -o wide >"${output_dir}/pods.txt" || true
  "${KUBECTL[@]}" get events -A --sort-by=.metadata.creationTimestamp >"${output_dir}/events.txt" || true
  helm list -A >"${output_dir}/helm.txt" || true
}

status() {
  resolve_registry_conflicts
  if [[ "${QUIET}" == "true" ]]; then
    ready >/dev/null
    return
  fi

  echo "profile: ${PROFILE}"
  echo "cluster context: ${CLUSTER_CONTEXT}"
  echo "registry: ${REGISTRY_HOST}"
  echo "image tag: ${IMAGE_TAG}"
  if ready >/dev/null 2>&1; then
    echo "env ready: yes"
  else
    echo "env ready: no"
  fi
  echo

  ctlptl get cluster || true
  ctlptl get registry || true
  echo
  "${KUBECTL[@]}" get nodes || true
  echo
  "${KUBECTL[@]}" get pods -n "${INSTALL_NAMESPACE}" || true
}

down() {
  helm uninstall agentgateway -n "${INSTALL_NAMESPACE}" >/dev/null 2>&1 || true
  helm uninstall agentgateway-crds -n "${INSTALL_NAMESPACE}" >/dev/null 2>&1 || true
  with_rendered_ctlptl_config ctlptl_delete
}

ctlptl_delete() {
  local ctlptl_file="$1"
  (
    cd "${REPO_ROOT}"
    ctlptl delete -f "${ctlptl_file}" >/dev/null 2>&1 || true
  )
}

case "${ACTION}" in
  up)
    ensure_cluster
    install_crds
    ready
    build_images
    preload_images
    deploy_helm
    ;;
  down)
    down
    ;;
  status)
    status
    ;;
  config)
    print_config
    ;;
  ready)
    ready
    ;;
  cluster)
    ensure_cluster
    ;;
  crds)
    install_crds
    ;;
  images)
    build_images
    ;;
  preload)
    preload_images
    ;;
  deploy)
    deploy_helm
    ;;
  collect-artifacts)
    collect_artifacts
    ;;
  *)
    echo "unknown action: ${ACTION}" >&2
    usage >&2
    exit 2
    ;;
esac
