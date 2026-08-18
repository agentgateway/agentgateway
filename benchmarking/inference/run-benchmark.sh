#!/usr/bin/env bash
#
# Runs one independently selectable inference benchmark treatment end to end
# and writes its native evidence into a shared campaign directory.
#
# Clones and manages its own llm-d-benchmark checkout - no setup needed
# beyond skopeo (brew install skopeo, see the image-loading gotcha below).
# Needs llm-d-benchmark#1696 or later for --data-access-timeout on standup,
# which is why LLM_D_BENCHMARK_REF defaults to main instead of a tag.
#
# Required: BENCHMARK_TREATMENT and BENCHMARK_CAMPAIGN_ID. Supported treatments
# are service, agentgateway-standalone, agentgateway-gateway, and
# envoy-standalone.
# Optional: BENCHMARK_CLUSTER_PROVIDER (kind or gke; default: kind),
# BENCHMARK_REPETITION (positive campaign repetition; default: 1),
# BENCHMARK_ACCELERATOR_TYPE and BENCHMARK_BACKEND_TYPE (default together to
# sim and inference-sim), BENCHMARK_ACCELERATOR_MODEL (default: auto),
# BENCHMARK_HARNESS (inference-perf or guidellm; default: inference-perf),
# BENCHMARK_ROUTER_CHART_VERSION (immutable standalone chart; default: v0.9.0),
# BENCHMARK_ROUTING_POLICY (default, optimized-baseline, cache-only, or
# load-only; default: default),
# BENCHMARK_REFERENCE_PROFILE (empty or
# optimized-baseline-qwen3-32b-h100-v0.9; default: empty),
# BENCHMARK_ENDPOINT_PATH (internal or external; default: internal),
# BENCHMARK_SCENARIO (default: agentgateway-comparison),
# BENCHMARK_KUBE_CONTEXT (required for GKE),
# BENCHMARK_MODELSERVICE_DEPLOY_TIMEOUT (seconds; default: 7200 for GPU,
# 1500 otherwise),
# BENCHMARK_HARDWARE_MODEL (override hardware metadata in result reports),
# BENCHMARK_WORKLOAD (upstream profile name; overrides the default local sweep),
# BENCHMARK_WORKLOAD_FILE_PATH (local profile; takes precedence over its name),
# BENCHMARK_WORKLOAD_VARIANT (upstream or deterministic; default: upstream for
# bundled optimized-baseline workloads),
# BENCHMARK_WORKLOAD_RATE_SCALE (upstream profiles only; default: replicas / 8),
# BENCHMARK_REQUEST_TIMEOUT (upstream profiles only; defaults to 300 seconds for
# upstream and 120 seconds for deterministic),
# BENCHMARK_RUNTIME_METRICS (direct vLLM/EPP/agentgateway capture; default: true),
# BENCHMARK_METRICS_INTERVAL (direct and GKE scrape interval; default: 5 seconds),
# BENCHMARK_GKE_MONITORING (auto, required, or off; default: auto),
# BENCHMARK_FAST_COLLECT (use llm-d-benchmark's compressed result transfer;
# default: true),
# BENCHMARK_GPU_RELEASE_POLICY (never, after-run, or after-load; default:
# never),
# BENCHMARK_GKE_PROJECT (defaults from a standard gke_* kube context),
# BENCHMARK_GKE_LOCATION (defaults from a standard gke_* kube context),
# BENCHMARK_GKE_CLUSTER (defaults from a standard gke_* kube context),
# BENCHMARK_GKE_GPU_NODEPOOL (default: gpu-h100),
# BENCHMARK_SECRET_NAMESPACE and BENCHMARK_HF_SECRET_NAME (GKE defaults:
# benchmark-secrets and llm-d-hf-token),
# CLUSTER_NAME (default: agentgateway-benchmark; kind only),
# AGTW_BENCHMARKING_DIR (suite input override; defaults to the selected suite),
# LLM_D_BENCHMARK_REF (default: main - branch, tag, or commit),
# LLM_D_BENCHMARK_DIR (skip the managed clone entirely and use this checkout
# instead - your responsibility to keep it up to date then),
# LLM_D_BENCHMARK_CACHE_DIR (where the managed clone lives, default: see below)
#
# Usage: ./run-benchmark.sh [--list-scenarios]
#        ./run-benchmark.sh --clean  remove the managed llm-d-benchmark clone

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCHMARK_SUITE="${BENCHMARK_SUITE:-llm-d-benchmark}"
SUITE_DIR="${SCRIPT_DIR}/suites/${BENCHMARK_SUITE}"

CLUSTER_NAME="${CLUSTER_NAME:-agentgateway-benchmark}"
BENCHMARK_TREATMENT="${BENCHMARK_TREATMENT:-}"
BENCHMARK_CAMPAIGN_ID="${BENCHMARK_CAMPAIGN_ID:-}"
BENCHMARK_REPETITION="${BENCHMARK_REPETITION:-1}"
BENCHMARK_CLUSTER_PROVIDER="${BENCHMARK_CLUSTER_PROVIDER:-kind}"
BENCHMARK_GATEWAY_IMPLEMENTATION="${BENCHMARK_GATEWAY_IMPLEMENTATION:-agentgateway}"
BENCHMARK_ROUTER_MODE="${BENCHMARK_ROUTER_MODE:-standalone}"
BENCHMARK_HARNESS="${BENCHMARK_HARNESS:-inference-perf}"
BENCHMARK_ROUTER_CHART_VERSION="${BENCHMARK_ROUTER_CHART_VERSION:-v0.9.0}"
BENCHMARK_ROUTING_POLICY="${BENCHMARK_ROUTING_POLICY:-default}"
BENCHMARK_REFERENCE_PROFILE="${BENCHMARK_REFERENCE_PROFILE:-}"
BENCHMARK_ENDPOINT_PATH="${BENCHMARK_ENDPOINT_PATH:-internal}"
BENCHMARK_WORKLOAD_VARIANT="${BENCHMARK_WORKLOAD_VARIANT:-upstream}"
BENCHMARK_SCENARIO="${BENCHMARK_SCENARIO:-agentgateway-comparison}"
BENCHMARK_ACCELERATOR_TYPE="${BENCHMARK_ACCELERATOR_TYPE:-}"
BENCHMARK_ACCELERATOR_MODEL="${BENCHMARK_ACCELERATOR_MODEL:-auto}"
BENCHMARK_BACKEND_TYPE="${BENCHMARK_BACKEND_TYPE:-}"
BENCHMARK_KUBE_CONTEXT="${BENCHMARK_KUBE_CONTEXT:-}"
AGTW_BENCHMARKING_DIR="${AGTW_BENCHMARKING_DIR:-$SUITE_DIR}"
LLM_D_BENCHMARK_REF="${LLM_D_BENCHMARK_REF:-main}"
LLM_D_BENCHMARK_CACHE_DIR="${LLM_D_BENCHMARK_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/agentgateway-benchmark/llm-d-benchmark}"
HELM_DIFF_HELM4_VERSION="${BENCHMARK_HELM_DIFF_HELM4_VERSION:-v3.15.11}"

AGW_VERSION="${AGW_VERSION:-v1.4.1}"
AGW_IMAGE="${AGW_IMAGE:-cr.agentgateway.dev/agentgateway:${AGW_VERSION}}"
AGW_CONTROLLER_NAMESPACE="${AGW_CONTROLLER_NAMESPACE:-agentgateway-system}"
WORKLOAD_FILE_PATH=""
WORKLOAD=""
PROVIDER_KUBECONFIG=""
RESULTS_ROOT="${BENCHMARK_RESULTS_DIR:-${SCRIPT_DIR}/results/${BENCHMARK_SUITE}}"
RESULTS_DIR=""
SPEC_DIR=""
DATA_ACCESS_TIMEOUT="${BENCHMARK_DATA_ACCESS_TIMEOUT:-600}"
MODELSERVICE_DEPLOY_TIMEOUT="${BENCHMARK_MODELSERVICE_DEPLOY_TIMEOUT:-}"
BENCHMARK_RESUME="${BENCHMARK_RESUME:-false}"
WORKLOAD_RATE_SCALE="${BENCHMARK_WORKLOAD_RATE_SCALE:-}"
REQUEST_TIMEOUT="${BENCHMARK_REQUEST_TIMEOUT:-}"
RUNTIME_METRICS_ENABLED="${BENCHMARK_RUNTIME_METRICS:-true}"
METRICS_INTERVAL="${BENCHMARK_METRICS_INTERVAL:-5}"
GKE_MONITORING_MODE="${BENCHMARK_GKE_MONITORING:-auto}"
FAST_COLLECT="${BENCHMARK_FAST_COLLECT:-true}"
GPU_RELEASE_POLICY="${BENCHMARK_GPU_RELEASE_POLICY:-never}"
GKE_PROJECT="${BENCHMARK_GKE_PROJECT:-}"
GKE_LOCATION="${BENCHMARK_GKE_LOCATION:-}"
GKE_CLUSTER="${BENCHMARK_GKE_CLUSTER:-}"
GKE_GPU_NODEPOOL="${BENCHMARK_GKE_GPU_NODEPOOL:-gpu-h100}"
GKE_CLEANUP_TIMEOUT="${BENCHMARK_GKE_CLEANUP_TIMEOUT:-1200}"
BENCHMARK_SECRET_NAMESPACE="${BENCHMARK_SECRET_NAMESPACE:-benchmark-secrets}"
BENCHMARK_HF_SECRET_NAME="${BENCHMARK_HF_SECRET_NAME:-llm-d-hf-token}"

MODEL="${BENCHMARK_MODEL:-Qwen/Qwen3-32B}"
REPLICAS="${BENCHMARK_REPLICAS:-2}"
TENSOR_PARALLELISM="${BENCHMARK_TENSOR_PARALLELISM:-2}"
GKE_ACCELERATOR="${BENCHMARK_GKE_ACCELERATOR:-nvidia-h100-80gb}"
MODEL_STORAGE_STRATEGY="${BENCHMARK_MODEL_STORAGE_STRATEGY:-default}"
MODEL_STORAGE_PROFILE="${BENCHMARK_MODEL_STORAGE_PROFILE:-default}"
MODEL_STORAGE_CLASS="${BENCHMARK_MODEL_STORAGE_CLASS:-}"
MODEL_STORAGE_SIZE="${BENCHMARK_MODEL_STORAGE_SIZE:-}"
WORKLOAD_STORAGE_PROFILE="${BENCHMARK_WORKLOAD_STORAGE_PROFILE:-default}"
WORKLOAD_STORAGE_CLASS="${BENCHMARK_WORKLOAD_STORAGE_CLASS:-}"
WORKLOAD_STORAGE_SIZE="${BENCHMARK_WORKLOAD_STORAGE_SIZE:-}"
MODEL_CACHE_ID="${BENCHMARK_MODEL_CACHE_ID:-}"
MODEL_CACHE_POLICY="${BENCHMARK_MODEL_CACHE_POLICY:-ephemeral}"
SCENARIO_NAME=""
TREATMENT_ID=""
CONFIG_SHA256=""
ROUTER_CHART_DIGEST="not-applicable"
STANDALONE_INVARIANT_SHA256="not-applicable"
WORKLOAD_SHA256=""
SCENARIO_SOURCE_SHA256=""
SHARED_CONFIG_SHA256=""
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
MONITOR_PID=""
TREATMENT_ACTIVE=false
RUNTIME_COLLECTOR_ACTIVE=false
RUNTIME_COLLECTOR_NAME="agentgateway-runtime-metrics"
RUNTIME_COLLECTOR_CONFIGMAP="agentgateway-runtime-metrics-scripts"
RUNTIME_METRICS_CAPTURE_DIR=""
TRAFFIC_COMPLETE_SOURCE=""
GPU_RELEASE_EVIDENCE=""
BENCHMARK_MANAGED_BY="agentgateway-benchmark"
BENCHMARK_LOCK_NAMESPACE="agentgateway-benchmark-system"
BENCHMARK_LOCK_NAME="campaign-lock"

log() { echo "[run-benchmark] $*"; }

resolve_workload() {
  if [[ -n "${BENCHMARK_WORKLOAD_FILE_PATH:-}" ]]; then
    WORKLOAD_FILE_PATH="${BENCHMARK_WORKLOAD_FILE_PATH}"
  elif [[ -n "${BENCHMARK_WORKLOAD:-}" ]]; then
    WORKLOAD="${BENCHMARK_WORKLOAD}"
    return
  else
    if [[ "${BENCHMARK_HARNESS}" == "guidellm" ]]; then
      WORKLOAD="sanity_random.yaml"
      return
    fi
    case "${BENCHMARK_ROUTING_POLICY}" in
      optimized-baseline|cache-only)
        WORKLOAD_FILE_PATH="${SUITE_DIR}/workloads/${BENCHMARK_WORKLOAD_VARIANT}-optimized-baseline.yaml.in"
        ;;
      load-only)
        WORKLOAD_FILE_PATH="${SUITE_DIR}/workloads/${BENCHMARK_WORKLOAD_VARIANT}-load-only.yaml.in"
        ;;
      default)
        WORKLOAD_FILE_PATH="${SUITE_DIR}/workloads/agentgateway-comparison.yaml.in"
        ;;
    esac
  fi
  WORKLOAD="$(basename "${WORKLOAD_FILE_PATH}")"
  WORKLOAD="${WORKLOAD%.in}"
}

resolve_request_timeout() {
  if [[ -z "${REQUEST_TIMEOUT}" ]]; then
    if [[ "${BENCHMARK_WORKLOAD_VARIANT}" == "upstream" ]]; then
      # llm-d-benchmark#1787 raised this timeout because the 7,200-token input
      # and 1,000-token output queue beyond 30 seconds at high Poisson rates.
      REQUEST_TIMEOUT=300
    else
      REQUEST_TIMEOUT=120
    fi
  fi
  [[ "${REQUEST_TIMEOUT}" =~ ^[1-9][0-9]*$ ]] || {
    log "BENCHMARK_REQUEST_TIMEOUT must be a positive integer"
    return 2
  }
}

render_bundled_upstream_workload() {
  case "${WORKLOAD_FILE_PATH}" in
    "${SUITE_DIR}/workloads/upstream-optimized-baseline.yaml.in"|\
    "${SUITE_DIR}/workloads/upstream-load-only.yaml.in"|\
    "${SUITE_DIR}/workloads/deterministic-optimized-baseline.yaml.in"|\
    "${SUITE_DIR}/workloads/deterministic-load-only.yaml.in") ;;
    *) return ;;
  esac

  resolve_request_timeout

  local rate_scale="${WORKLOAD_RATE_SCALE}"
  if [[ -z "${rate_scale}" ]]; then
    rate_scale="$(awk -v replicas="${REPLICAS}" 'BEGIN { printf "%.8g", replicas / 8 }')"
  fi
  [[ "${rate_scale}" =~ ^([0-9]+([.][0-9]*)?|[.][0-9]+)$ ]] || {
    log "BENCHMARK_WORKLOAD_RATE_SCALE must be a positive number"
    return 2
  }
  awk -v value="${rate_scale}" 'BEGIN { exit !(value > 0) }' || {
    log "BENCHMARK_WORKLOAD_RATE_SCALE must be greater than zero"
    return 2
  }
  local rendered_workload="${SPEC_DIR}/${WORKLOAD}"
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" \
    "${SUITE_DIR}/scripts/render-workload.py" \
    --input "${WORKLOAD_FILE_PATH}" \
    --output "${rendered_workload}" \
    --rate-scale "${rate_scale}" \
    --request-timeout "${REQUEST_TIMEOUT}"
  WORKLOAD_FILE_PATH="${rendered_workload}"
  log "optimized-baseline workload: variant=${BENCHMARK_WORKLOAD_VARIANT}, ${REPLICAS}/8 replica rate scale=${rate_scale}, request timeout=${REQUEST_TIMEOUT}s"
}

cleanup_sensitive_temp() {
  if [[ -n "${MONITOR_PID}" ]]; then
    kill "${MONITOR_PID}" >/dev/null 2>&1 || true
    wait "${MONITOR_PID}" >/dev/null 2>&1 || true
  fi
  [[ -z "${PROVIDER_KUBECONFIG}" ]] || rm -f "${PROVIDER_KUBECONFIG}"
}

cleanup_on_exit() {
  local status=$?
  trap - EXIT
  if [[ "${RUNTIME_COLLECTOR_ACTIVE}" == "true" ]]; then
    log "stopping runtime metrics collector after interrupted or failed run"
    stop_runtime_metrics || log "warning: runtime metrics cleanup failed"
  fi
  if (( status != 0 )) && [[ "${TREATMENT_ACTIVE}" == "true" && \
      "${MODEL_CACHE_POLICY}" == "ephemeral" && -f "${SPEC_DIR}/spec.yaml" ]] && \
      kubectl get namespace "${SCENARIO_NAME}" >/dev/null 2>&1; then
    log "failure cleanup: tearing down ${SCENARIO_NAME}"
    teardown_treatment || log "warning: teardown failed; ${SCENARIO_NAME} remains for campaign cleanup"
  fi
  cleanup_sensitive_temp
  exit "${status}"
}
trap cleanup_on_exit EXIT

resolve_gke_identity() {
  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" != "gke" ]]; then
    return
  fi
  if [[ -n "${GKE_PROJECT}" && -n "${GKE_LOCATION}" && \
      -n "${GKE_CLUSTER}" ]]; then
    return
  fi
  if [[ "${BENCHMARK_KUBE_CONTEXT}" =~ ^gke_([^_]+)_([^_]+)_(.+)$ ]]; then
    [[ -n "${GKE_PROJECT}" ]] || GKE_PROJECT="${BASH_REMATCH[1]}"
    [[ -n "${GKE_LOCATION}" ]] || GKE_LOCATION="${BASH_REMATCH[2]}"
    [[ -n "${GKE_CLUSTER}" ]] || GKE_CLUSTER="${BASH_REMATCH[3]}"
  fi
  [[ -n "${GKE_PROJECT}" && -n "${GKE_LOCATION}" && \
      -n "${GKE_CLUSTER}" ]] || {
    log "BENCHMARK_GKE_PROJECT, BENCHMARK_GKE_LOCATION, and BENCHMARK_GKE_CLUSTER are required when the kube context is not a standard gke_<project>_<location>_<cluster> name"
    return 2
  }
}

acquire_campaign_lock() {
  [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" ]] || return
  kubectl create namespace "${BENCHMARK_LOCK_NAMESPACE}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

  local existing
  existing="$(kubectl get configmap "${BENCHMARK_LOCK_NAME}" \
    --namespace "${BENCHMARK_LOCK_NAMESPACE}" \
    -o jsonpath='{.data.campaign-id}' 2>/dev/null || true)"
  if [[ -n "${existing}" ]]; then
    [[ "${existing}" == "${BENCHMARK_CAMPAIGN_ID}" ]] || {
      log "GKE benchmark cluster is locked by campaign ${existing}"
      return 1
    }
    return
  fi

  if ! kubectl create configmap "${BENCHMARK_LOCK_NAME}" \
      --namespace "${BENCHMARK_LOCK_NAMESPACE}" \
      --from-literal=campaign-id="${BENCHMARK_CAMPAIGN_ID}" \
      --from-literal=created-at="$(date -u +%Y-%m-%dT%H:%M:%SZ)" >/dev/null 2>&1; then
    existing="$(kubectl get configmap "${BENCHMARK_LOCK_NAME}" \
      --namespace "${BENCHMARK_LOCK_NAMESPACE}" \
      -o jsonpath='{.data.campaign-id}' 2>/dev/null || true)"
    [[ "${existing}" == "${BENCHMARK_CAMPAIGN_ID}" ]] || {
      log "another campaign acquired the GKE benchmark lock"
      return 1
    }
  fi
  log "campaign lock: ${BENCHMARK_CAMPAIGN_ID}"
}

load_hf_token_from_cluster() {
  [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" && -z "${HF_TOKEN:-}" ]] || return 0
  HF_TOKEN="$(kubectl get secret "${BENCHMARK_HF_SECRET_NAME}" \
    --namespace "${BENCHMARK_SECRET_NAMESPACE}" \
    --template='{{index .data "HF_TOKEN" | base64decode}}')"
  [[ "${HF_TOKEN}" == hf_* ]] || {
    unset HF_TOKEN
    log "${BENCHMARK_SECRET_NAMESPACE}/${BENCHMARK_HF_SECRET_NAME} does not contain a valid HF_TOKEN"
    return 1
  }
  export HF_TOKEN
  log "loaded HF_TOKEN from ${BENCHMARK_SECRET_NAMESPACE}/${BENCHMARK_HF_SECRET_NAME}"
}

ensure_benchmark_namespace() {
  if kubectl get namespace "${SCENARIO_NAME}" >/dev/null 2>&1; then
    local existing_owner existing_campaign
    existing_owner="$(kubectl get namespace "${SCENARIO_NAME}" \
      -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}')"
    existing_campaign="$(kubectl get namespace "${SCENARIO_NAME}" \
      -o jsonpath='{.metadata.annotations.benchmark\.agentgateway\.dev/campaign-id}')"
    if [[ "${existing_owner}" != "${BENCHMARK_MANAGED_BY}" || \
          "${existing_campaign}" != "${BENCHMARK_CAMPAIGN_ID}" ]]; then
      log "refusing namespace ${SCENARIO_NAME}: it belongs to campaign ${existing_campaign:-unknown}"
      return 1
    fi
  fi
  kubectl create namespace "${SCENARIO_NAME}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  kubectl label namespace "${SCENARIO_NAME}" \
    app.kubernetes.io/managed-by="${BENCHMARK_MANAGED_BY}" \
    benchmark.agentgateway.dev/treatment="${TREATMENT_ID}" \
    --overwrite >/dev/null
  kubectl annotate namespace "${SCENARIO_NAME}" \
    benchmark.agentgateway.dev/campaign-id="${BENCHMARK_CAMPAIGN_ID}" \
    --overwrite >/dev/null
  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" ]]; then
    # Keep the credential in the cluster's dedicated secret namespace. Copy
    # the opaque Secret object without decoding or printing its token so each
    # independently named benchmark namespace can authenticate model access.
    local secret_json copied_secret
    secret_json="$(mktemp "${TMPDIR:-/tmp}/agentgateway-hf-secret.XXXXXX.json")"
    copied_secret="$(mktemp "${TMPDIR:-/tmp}/agentgateway-hf-secret-copy.XXXXXX.json")"
    kubectl get secret "${BENCHMARK_HF_SECRET_NAME}" \
      --namespace "${BENCHMARK_SECRET_NAMESPACE}" -o json > "${secret_json}"
    "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - \
      "${secret_json}" "${SCENARIO_NAME}" > "${copied_secret}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    secret = json.load(stream)
metadata = secret.setdefault("metadata", {})
metadata["namespace"] = sys.argv[2]
for key in ("creationTimestamp", "managedFields", "resourceVersion", "uid"):
    metadata.pop(key, None)
secret.pop("status", None)
json.dump(secret, sys.stdout)
PY
    kubectl apply -f "${copied_secret}" >/dev/null
    rm -f -- "${secret_json}" "${copied_secret}"
    log "copied ${BENCHMARK_SECRET_NAMESPACE}/${BENCHMARK_HF_SECRET_NAME} to ${SCENARIO_NAME}"
  fi
}

usage() {
  cat <<'EOF'
Usage: run-benchmark.sh [options]

Options:
  --list-scenarios    list the local default and upstream guide scenarios
  --clean             remove the managed llm-d-benchmark checkout and exit
  -h, --help          show this help

BENCHMARK_TREATMENT and BENCHMARK_CAMPAIGN_ID are required. GKE runs also
require BENCHMARK_KUBE_CONTEXT.
EOF
}

parse_args() {
  while (( $# > 0 )); do
    case "$1" in
      --list-scenarios)
        ensure_llm_d_benchmark
        list_scenarios
        exit 0
        ;;
      --clean)
        clean
        exit 0
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        log "unknown argument: $1"
        usage >&2
        return 2
        ;;
    esac
  done
}

resolve_storage_profiles() {
  case "${MODEL_STORAGE_PROFILE}" in
    default|shared|high-throughput-shared) ;;
    *) log "unsupported BENCHMARK_MODEL_STORAGE_PROFILE: ${MODEL_STORAGE_PROFILE}"; return 2 ;;
  esac
  case "${WORKLOAD_STORAGE_PROFILE}" in
    default|shared) ;;
    *) log "unsupported BENCHMARK_WORKLOAD_STORAGE_PROFILE: ${WORKLOAD_STORAGE_PROFILE}"; return 2 ;;
  esac

  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" != "gke" && \
      ( "${MODEL_STORAGE_PROFILE}" != "default" || \
        "${WORKLOAD_STORAGE_PROFILE}" != "default" ) ]]; then
    log "storage profiles are not implemented for BENCHMARK_CLUSTER_PROVIDER=${BENCHMARK_CLUSTER_PROVIDER}"
    return 2
  fi

  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" ]]; then
    case "${MODEL_STORAGE_PROFILE}" in
      high-throughput-shared)
        [[ "${MODEL_STORAGE_STRATEGY}" != "default" ]] || MODEL_STORAGE_STRATEGY=filestore
        [[ -n "${MODEL_STORAGE_CLASS}" ]] || MODEL_STORAGE_CLASS=premium-rwx
        [[ -n "${MODEL_STORAGE_SIZE}" ]] || MODEL_STORAGE_SIZE=2560Gi
        ;;
      shared)
        [[ "${MODEL_STORAGE_STRATEGY}" != "default" ]] || MODEL_STORAGE_STRATEGY=filestore
        [[ -n "${MODEL_STORAGE_CLASS}" ]] || MODEL_STORAGE_CLASS=standard-rwx
        [[ -n "${MODEL_STORAGE_SIZE}" ]] || MODEL_STORAGE_SIZE=1Ti
        ;;
    esac
    if [[ "${WORKLOAD_STORAGE_PROFILE}" == "shared" ]]; then
      [[ -n "${WORKLOAD_STORAGE_CLASS}" ]] || WORKLOAD_STORAGE_CLASS=standard-rwx
      [[ -n "${WORKLOAD_STORAGE_SIZE}" ]] || WORKLOAD_STORAGE_SIZE=100Gi
    fi
  fi
}

validate_configuration() {
  case "${BENCHMARK_SUITE}" in
    llm-d-benchmark) ;;
    *) log "unsupported BENCHMARK_SUITE: ${BENCHMARK_SUITE}"; return 2 ;;
  esac
  [[ -d "${SUITE_DIR}" ]] || {
    log "benchmark suite directory does not exist: ${SUITE_DIR}"
    return 2
  }

  case "${BENCHMARK_TREATMENT}" in
    service) ;;
    agentgateway-standalone)
      BENCHMARK_GATEWAY_IMPLEMENTATION=agentgateway
      BENCHMARK_ROUTER_MODE=standalone
      ;;
    agentgateway-gateway)
      BENCHMARK_GATEWAY_IMPLEMENTATION=agentgateway
      BENCHMARK_ROUTER_MODE=gateway
      ;;
    envoy-standalone)
      BENCHMARK_GATEWAY_IMPLEMENTATION=envoy
      BENCHMARK_ROUTER_MODE=standalone
      ;;
    "")
      log "BENCHMARK_TREATMENT is required"
      return 2
      ;;
    *)
      log "unsupported BENCHMARK_TREATMENT: ${BENCHMARK_TREATMENT}"
      return 2
      ;;
  esac

  [[ "${BENCHMARK_CAMPAIGN_ID}" =~ ^[a-z0-9][a-z0-9.-]*$ ]] || {
    log "BENCHMARK_CAMPAIGN_ID is required and must contain lowercase letters, digits, dots, or hyphens"
    return 2
  }
  [[ "${BENCHMARK_REPETITION}" =~ ^[1-9][0-9]*$ ]] || {
    log "BENCHMARK_REPETITION must be a positive integer"
    return 2
  }
  case "${BENCHMARK_CLUSTER_PROVIDER}" in
    kind|gke) ;;
    *) log "unsupported BENCHMARK_CLUSTER_PROVIDER: ${BENCHMARK_CLUSTER_PROVIDER}"; return 2 ;;
  esac
  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" != "gke" && \
      ( -n "${BENCHMARK_GKE_CPU_NODEPOOL:-}" || \
        -n "${BENCHMARK_GKE_HARNESS_NODEPOOL:-}" ) ]]; then
    log "BENCHMARK_GKE_* node-pool settings require BENCHMARK_CLUSTER_PROVIDER=gke"
    return 2
  fi
  case "${BENCHMARK_ROUTER_MODE}" in standalone|gateway) ;; *) log "unsupported BENCHMARK_ROUTER_MODE: ${BENCHMARK_ROUTER_MODE}"; return 2 ;; esac
  case "${BENCHMARK_GATEWAY_IMPLEMENTATION}" in agentgateway|envoy) ;; *) log "unsupported BENCHMARK_GATEWAY_IMPLEMENTATION: ${BENCHMARK_GATEWAY_IMPLEMENTATION}"; return 2 ;; esac
  case "${BENCHMARK_ROUTING_POLICY}" in
    default|optimized-baseline|cache-only|load-only) ;;
    *) log "unsupported BENCHMARK_ROUTING_POLICY: ${BENCHMARK_ROUTING_POLICY}"; return 2 ;;
  esac
  case "${BENCHMARK_REFERENCE_PROFILE}" in
    ""|optimized-baseline-qwen3-32b-h100-v0.9|optimized-baseline-qwen3-32b-h100-v0.9-vllm-v0.27.1) ;;
    *) log "unsupported BENCHMARK_REFERENCE_PROFILE: ${BENCHMARK_REFERENCE_PROFILE}"; return 2 ;;
  esac
  case "${BENCHMARK_ENDPOINT_PATH}" in
    internal|external) ;;
    *) log "BENCHMARK_ENDPOINT_PATH must be internal or external"; return 2 ;;
  esac
  case "${BENCHMARK_WORKLOAD_VARIANT}" in
    upstream|deterministic) ;;
    *) log "unsupported BENCHMARK_WORKLOAD_VARIANT: ${BENCHMARK_WORKLOAD_VARIANT}"; return 2 ;;
  esac
  case "${BENCHMARK_HARNESS}" in
    inference-perf|guidellm) ;;
    *) log "unsupported BENCHMARK_HARNESS: ${BENCHMARK_HARNESS}"; return 2 ;;
  esac
  if [[ "${BENCHMARK_HARNESS}" != "inference-perf" && \
      "${BENCHMARK_ROUTING_POLICY}" != "default" ]]; then
    log "BENCHMARK_ROUTING_POLICY=${BENCHMARK_ROUTING_POLICY} requires BENCHMARK_HARNESS=inference-perf"
    return 2
  fi
  if [[ ! "${BENCHMARK_ROUTER_CHART_VERSION}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    log "BENCHMARK_ROUTER_CHART_VERSION must be an immutable vMAJOR.MINOR.PATCH release"
    return 2
  fi

  resolve_workload

  if [[ -z "${BENCHMARK_ACCELERATOR_TYPE}" && -z "${BENCHMARK_BACKEND_TYPE}" ]]; then
    BENCHMARK_ACCELERATOR_TYPE=sim
    BENCHMARK_BACKEND_TYPE=inference-sim
  elif [[ -z "${BENCHMARK_ACCELERATOR_TYPE}" ]]; then
    log "BENCHMARK_ACCELERATOR_TYPE is required when BENCHMARK_BACKEND_TYPE is set"
    return 2
  elif [[ -z "${BENCHMARK_BACKEND_TYPE}" ]]; then
    BENCHMARK_BACKEND_TYPE=vllm
  fi
  case "${BENCHMARK_ACCELERATOR_TYPE}" in
    sim|cpu|gpu) ;;
    amd|xpu|hpu|tpu/v6|tpu/v7)
      log "BENCHMARK_ACCELERATOR_TYPE=${BENCHMARK_ACCELERATOR_TYPE} is recognized but its adapter is not implemented"
      return 2
      ;;
    *) log "unsupported BENCHMARK_ACCELERATOR_TYPE: ${BENCHMARK_ACCELERATOR_TYPE}"; return 2 ;;
  esac
  case "${BENCHMARK_BACKEND_TYPE}" in inference-sim|vllm) ;; sglang|trtllm|triton) log "BENCHMARK_BACKEND_TYPE=${BENCHMARK_BACKEND_TYPE} is recognized but its adapter is not implemented"; return 2 ;; *) log "unsupported BENCHMARK_BACKEND_TYPE: ${BENCHMARK_BACKEND_TYPE}"; return 2 ;; esac
  if [[ "${BENCHMARK_ACCELERATOR_TYPE}/${BENCHMARK_BACKEND_TYPE}" == "sim/inference-sim" ]]; then
    MODEL="${BENCHMARK_MODEL:-facebook/opt-125m}"
    REPLICAS="${BENCHMARK_REPLICAS:-3}"
    TENSOR_PARALLELISM="${BENCHMARK_TENSOR_PARALLELISM:-0}"
  elif [[ "${BENCHMARK_ACCELERATOR_TYPE}" == "cpu" ]]; then
    MODEL="${BENCHMARK_MODEL:-facebook/opt-125m}"
    REPLICAS="${BENCHMARK_REPLICAS:-1}"
    TENSOR_PARALLELISM="${BENCHMARK_TENSOR_PARALLELISM:-1}"
  elif [[ "${BENCHMARK_BACKEND_TYPE}" == "inference-sim" ]]; then
    log "inference-sim requires BENCHMARK_ACCELERATOR_TYPE=sim"
    return 2
  fi
  if [[ "${BENCHMARK_ROUTING_POLICY}" != "default" ]]; then
    if [[ -z "${BENCHMARK_REPLICAS+x}" ]]; then
      # The published optimized-baseline result uses eight TP=2 model servers
      # on 16 H100s. Keep that topology unless the caller deliberately selects
      # a smaller scale experiment.
      REPLICAS=8
    fi
    if [[ "${BENCHMARK_ACCELERATOR_MODEL}" == "auto" ]]; then
      BENCHMARK_ACCELERATOR_MODEL=h100
    fi
    if [[ "${BENCHMARK_SCENARIO}" != "optimized-baseline" ]]; then
      log "BENCHMARK_ROUTING_POLICY=${BENCHMARK_ROUTING_POLICY} requires BENCHMARK_SCENARIO=optimized-baseline"
      return 2
    fi
    if [[ "${BENCHMARK_ACCELERATOR_TYPE}/${BENCHMARK_BACKEND_TYPE}" != "gpu/vllm" ]]; then
      log "BENCHMARK_ROUTING_POLICY=${BENCHMARK_ROUTING_POLICY} requires gpu/vllm"
      return 2
    fi
    if [[ "${MODEL}" != "Qwen/Qwen3-32B" || "${TENSOR_PARALLELISM}" != "2" ]]; then
      log "upstream routing-policy reproductions require Qwen/Qwen3-32B with BENCHMARK_TENSOR_PARALLELISM=2"
      return 2
    fi
    if [[ "${BENCHMARK_ACCELERATOR_MODEL}" != "h100" ]]; then
      log "upstream routing-policy reproductions require BENCHMARK_ACCELERATOR_MODEL=h100"
      return 2
    fi
  fi
  if [[ "${BENCHMARK_REFERENCE_PROFILE}" == "optimized-baseline-qwen3-32b-h100-v0.9" ||
        "${BENCHMARK_REFERENCE_PROFILE}" == "optimized-baseline-qwen3-32b-h100-v0.9-vllm-v0.27.1" ]]; then
    # The published v0.9 report is a historical artifact. Its calibration
    # matrix records vLLM v0.23.0 even though the mutable v0.9 guide overlay
    # now selects v0.26.0. Reproducing the report therefore requires locking
    # the report-era component set rather than following the drifting tag.
    [[ "${BENCHMARK_ROUTING_POLICY}" == "optimized-baseline" && \
       "${BENCHMARK_SCENARIO}" == "optimized-baseline" && \
       "${BENCHMARK_ACCELERATOR_TYPE}/${BENCHMARK_BACKEND_TYPE}" == "gpu/vllm" && \
       "${MODEL}" == "Qwen/Qwen3-32B" && \
       "${REPLICAS}" == "8" && "${TENSOR_PARALLELISM}" == "2" && \
       "${BENCHMARK_ACCELERATOR_MODEL}" == "h100" && \
       "${BENCHMARK_ROUTER_CHART_VERSION}" == "v0.9.0" ]] || {
      log "${BENCHMARK_REFERENCE_PROFILE} requires optimized-baseline, Qwen3-32B, 8 TP=2 H100 vLLM replicas, and router v0.9.0"
      return 2
    }
    if [[ "${BENCHMARK_TREATMENT}" == agentgateway-* && \
          "${AGW_VERSION}" != "v1.4.1" ]]; then
      log "${BENCHMARK_REFERENCE_PROFILE} requires AGW_VERSION=v1.4.1"
      return 2
    fi
  fi
  if [[ "${BENCHMARK_GATEWAY_IMPLEMENTATION}/${BENCHMARK_ROUTER_MODE}" == "envoy/gateway" ]]; then
    log "Envoy AI Gateway is not implemented yet; envoy currently supports standalone mode only"
    return 2
  fi
  resolve_storage_profiles
  case "${MODEL_STORAGE_STRATEGY}" in default|filestore) ;; hyperdisk-ml|local-ssd|gcs-fuse) log "BENCHMARK_MODEL_STORAGE_STRATEGY=${MODEL_STORAGE_STRATEGY} is recognized but its adapter is not implemented"; return 2 ;; *) log "unsupported BENCHMARK_MODEL_STORAGE_STRATEGY: ${MODEL_STORAGE_STRATEGY}"; return 2 ;; esac
  if [[ "${MODEL_STORAGE_STRATEGY}" == "filestore" && \
      "${BENCHMARK_CLUSTER_PROVIDER}" != "gke" ]]; then
    log "BENCHMARK_MODEL_STORAGE_STRATEGY=filestore requires BENCHMARK_CLUSTER_PROVIDER=gke"
    return 2
  fi
  case "${MODEL_CACHE_POLICY}" in ephemeral|retain) ;; *) log "BENCHMARK_MODEL_CACHE_POLICY must be ephemeral or retain"; return 2 ;; esac
  case "${BENCHMARK_RESUME}" in true|false) ;; *) log "BENCHMARK_RESUME must be true or false"; return 2 ;; esac
  case "${RUNTIME_METRICS_ENABLED}" in true|false) ;; *) log "BENCHMARK_RUNTIME_METRICS must be true or false"; return 2 ;; esac
  case "${GKE_MONITORING_MODE}" in auto|required|off) ;; *) log "BENCHMARK_GKE_MONITORING must be auto, required, or off"; return 2 ;; esac
  case "${FAST_COLLECT}" in true|false) ;; *) log "BENCHMARK_FAST_COLLECT must be true or false"; return 2 ;; esac
  case "${GPU_RELEASE_POLICY}" in never|after-run|after-load) ;; *) log "BENCHMARK_GPU_RELEASE_POLICY must be never, after-run, or after-load"; return 2 ;; esac
  if [[ "${GPU_RELEASE_POLICY}" != "never" && \
      "${BENCHMARK_CLUSTER_PROVIDER}/${BENCHMARK_ACCELERATOR_TYPE}" != "gke/gpu" ]]; then
    log "BENCHMARK_GPU_RELEASE_POLICY=${GPU_RELEASE_POLICY} requires a GKE GPU benchmark"
    return 2
  fi
  if [[ "${GPU_RELEASE_POLICY}" == "after-load" && \
      "${BENCHMARK_HARNESS}" != "inference-perf" ]]; then
    log "BENCHMARK_GPU_RELEASE_POLICY=after-load requires BENCHMARK_HARNESS=inference-perf"
    return 2
  fi
  if [[ "${GPU_RELEASE_POLICY}" != "never" && -z "${GKE_GPU_NODEPOOL}" ]]; then
    log "BENCHMARK_GKE_GPU_NODEPOOL is required when GPU release is enabled"
    return 2
  fi
  [[ "${GKE_CLEANUP_TIMEOUT}" =~ ^[1-9][0-9]*$ ]] || {
    log "BENCHMARK_GKE_CLEANUP_TIMEOUT must be a positive integer"
    return 2
  }
  [[ "${METRICS_INTERVAL}" =~ ^[1-9][0-9]*$ ]] || {
    log "BENCHMARK_METRICS_INTERVAL must be a positive integer"
    return 2
  }

  TREATMENT_ID="${BENCHMARK_TREATMENT}"
  local policy_name=""
  [[ "${BENCHMARK_ROUTING_POLICY}" == "default" ]] || policy_name="-${BENCHMARK_ROUTING_POLICY}"
  SCENARIO_NAME="${TREATMENT_ID}-${BENCHMARK_SCENARIO}${policy_name}-${BENCHMARK_ACCELERATOR_TYPE}-${BENCHMARK_CLUSTER_PROVIDER}"
  RESULTS_DIR="${RESULTS_ROOT}/${BENCHMARK_CAMPAIGN_ID}"
  SPEC_DIR="${RESULTS_DIR}/.work/${TREATMENT_ID}/repetition-${BENCHMARK_REPETITION}"

  if [[ -z "${MODELSERVICE_DEPLOY_TIMEOUT}" ]]; then
    if [[ "${BENCHMARK_ACCELERATOR_TYPE}" == "gpu" ]]; then
      MODELSERVICE_DEPLOY_TIMEOUT=7200
    else
      MODELSERVICE_DEPLOY_TIMEOUT=1500
    fi
  fi
  [[ "${DATA_ACCESS_TIMEOUT}" =~ ^[1-9][0-9]*$ ]] || {
    log "BENCHMARK_DATA_ACCESS_TIMEOUT must be a positive integer"
    return 2
  }
  [[ "${MODELSERVICE_DEPLOY_TIMEOUT}" =~ ^[1-9][0-9]*$ ]] || {
    log "BENCHMARK_MODELSERVICE_DEPLOY_TIMEOUT must be a positive integer"
    return 2
  }

  [[ "${REPLICAS}" =~ ^[1-9][0-9]*$ ]] || {
    log "BENCHMARK_REPLICAS must be a positive integer"
    return 2
  }
  [[ "${TENSOR_PARALLELISM}" =~ ^[0-9]+$ ]] || {
    log "BENCHMARK_TENSOR_PARALLELISM must be a non-negative integer"
    return 2
  }
  [[ "${HELM_DIFF_HELM4_VERSION}" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    log "BENCHMARK_HELM_DIFF_HELM4_VERSION must be a semantic version"
    return 2
  }

  if [[ -n "${WORKLOAD_FILE_PATH}" && ! -f "${WORKLOAD_FILE_PATH}" ]]; then
    log "BENCHMARK_WORKLOAD_FILE_PATH does not exist: ${WORKLOAD_FILE_PATH}"
    return 2
  fi

  command -v kubectl >/dev/null || { log "kubectl is required"; return 1; }
  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "kind" ]]; then
    command -v kind >/dev/null || { log "kind is required for the kind provider"; return 1; }
  else
    command -v gcloud >/dev/null || { log "gcloud is required for the GKE provider"; return 1; }
  fi
}

configure_provider() {
  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "kind" ]]; then
    kind get clusters | grep -Fxq "${CLUSTER_NAME}" || {
      log "kind cluster ${CLUSTER_NAME} does not exist; create it first or use make -C controller benchmark"
      return 1
    }
    PROVIDER_KUBECONFIG="$(mktemp "${TMPDIR:-/tmp}/agentgateway-kubeconfig.XXXXXX")"
    kind get kubeconfig --name "${CLUSTER_NAME}" > "${PROVIDER_KUBECONFIG}"
    export KUBECONFIG="${PROVIDER_KUBECONFIG}"
    return
  fi

  if [[ -z "${BENCHMARK_KUBE_CONTEXT}" ]]; then
    log "BENCHMARK_KUBE_CONTEXT is required for the gke provider"
    return 2
  fi
  kubectl config get-contexts "${BENCHMARK_KUBE_CONTEXT}" -o name | grep -Fxq "${BENCHMARK_KUBE_CONTEXT}" || {
    log "kubectl context ${BENCHMARK_KUBE_CONTEXT} does not exist"
    return 1
  }

  # Isolate llm-d-benchmark from the caller's mutable current context. This
  # also means the script never changes the user's active kubectl context.
  PROVIDER_KUBECONFIG="$(mktemp "${TMPDIR:-/tmp}/agentgateway-kubeconfig.XXXXXX")"
  kubectl config view --context "${BENCHMARK_KUBE_CONTEXT}" --minify --flatten > "${PROVIDER_KUBECONFIG}"
  export KUBECONFIG="${PROVIDER_KUBECONFIG}"

  local provider_ids
  provider_ids="$(kubectl get nodes -o jsonpath='{range .items[*]}{.spec.providerID}{"\n"}{end}')"
  if [[ -z "${provider_ids}" ]]; then
    log "GKE context ${BENCHMARK_KUBE_CONTEXT} has no nodes; scale the required node pools up first"
    return 1
  fi
  if grep -Evq '^gce://' <<<"${provider_ids}"; then
    log "context ${BENCHMARK_KUBE_CONTEXT} does not appear to point exclusively at GKE nodes"
    return 1
  fi

  if [[ "${BENCHMARK_ACCELERATOR_TYPE}" == "gpu" ]]; then
    local required_gpus available_gpus storage_class
    required_gpus=$((REPLICAS * TENSOR_PARALLELISM))
    available_gpus="$(kubectl get nodes -l "cloud.google.com/gke-accelerator=${GKE_ACCELERATOR}" \
      -o jsonpath='{range .items[*]}{.status.allocatable.nvidia\.com/gpu}{"\n"}{end}' | \
      awk '{ total += $1 } END { print total + 0 }')"
    if (( available_gpus < required_gpus )); then
      log "GPU acceleration needs ${required_gpus} allocatable ${GKE_ACCELERATOR} GPUs, but only ${available_gpus} were found"
      return 1
    fi
    storage_class="${MODEL_STORAGE_CLASS:-standard-rwx}"
    kubectl get storageclass "${storage_class}" >/dev/null || {
      log "GPU acceleration needs model storage class ${storage_class}; enable the GKE Filestore CSI driver or set BENCHMARK_MODEL_STORAGE_CLASS"
      return 1
    }
    if [[ -n "${WORKLOAD_STORAGE_CLASS}" ]]; then
      kubectl get storageclass "${WORKLOAD_STORAGE_CLASS}" >/dev/null || {
        log "workload storage class ${WORKLOAD_STORAGE_CLASS} does not exist"
        return 1
      }
    fi
    if [[ "${storage_class}" == "standard-rwx" ]]; then
      log "warning: standard-rwx is Basic HDD Filestore (~100 MiB/s at 1 TiB); concurrent TP workers can make Qwen3-32B startup take about 50 minutes"
      log "use BENCHMARK_MODEL_STORAGE_PROFILE=high-throughput-shared for substantially faster cold starts"
    fi
  fi
}

# llm-d-benchmark#1741: install.sh currently accepts Helm 4 but then pins
# helm-diff v3.13.0, whose --validate/--dry-run combination fails under Helm 4
# when helmfile diffs an existing release. Keep this downstream workaround
# until upstream selects a plugin version based on the detected Helm major.
ensure_helm_diff_compatibility() {
  local helm_version helm_major installed_version wanted_version
  helm_version="$(helm version --template '{{.Version}}')"
  if [[ ! "${helm_version}" =~ ^v?([0-9]+)\. ]]; then
    log "couldn't parse Helm version: ${helm_version}"
    return 1
  fi
  helm_major="${BASH_REMATCH[1]}"
  (( helm_major >= 4 )) || return 0

  installed_version="$(helm plugin list 2>/dev/null | awk '$1 == "diff" { print $2 }')"
  wanted_version="${HELM_DIFF_HELM4_VERSION#v}"
  if [[ "${installed_version#v}" == "${wanted_version}" ]]; then
    return 0
  fi

  log "Helm ${helm_version} needs a compatible helm-diff; installing ${HELM_DIFF_HELM4_VERSION}"
  if [[ -n "${installed_version}" ]]; then
    helm plugin uninstall diff </dev/null
  fi
  # helm-diff does not publish provenance artifacts for Helm 4's default
  # plugin verification, so installation from this pinned Git tag must opt out.
  CI=1 GIT_TERMINAL_PROMPT=0 helm plugin install \
    https://github.com/databus23/helm-diff \
    --version "${HELM_DIFF_HELM4_VERSION}" --verify=false </dev/null

  installed_version="$(helm plugin list 2>/dev/null | awk '$1 == "diff" { print $2 }')"
  if [[ "${installed_version#v}" != "${wanted_version}" ]]; then
    log "helm-diff is ${installed_version:-missing}; expected ${HELM_DIFF_HELM4_VERSION}"
    return 1
  fi
}

# If the caller already pointed us at a clone (LLM_D_BENCHMARK_DIR), use it
# as-is - that's on them to keep updated. Otherwise manage our own clone in
# a cache dir: clone once, fetch+checkout LLM_D_BENCHMARK_REF on every run
# after that instead of re-cloning from scratch each time.
#
# Setup itself (venv, the llmdbenchmark CLI, the separate `planner` package
# it needs, picking a Python 3.11+ interpreter) is delegated to
# llm-d-benchmark's own install.sh instead of reimplementing it - it already
# handles all of that correctly, including cases plain `pip install -e .`
# doesn't (planner isn't pulled in by that alone, and macOS's system python3
# is usually too old for llm-d-benchmark's >=3.11 requirement).
ensure_llm_d_benchmark() {
  if [[ -n "${LLM_D_BENCHMARK_DIR:-}" ]]; then
    log "using existing llm-d-benchmark checkout at ${LLM_D_BENCHMARK_DIR}"
  else
    LLM_D_BENCHMARK_DIR="${LLM_D_BENCHMARK_CACHE_DIR}"

    if [[ ! -d "${LLM_D_BENCHMARK_DIR}/.git" ]]; then
      log "cloning llm-d-benchmark into ${LLM_D_BENCHMARK_DIR}"
      mkdir -p "$(dirname "${LLM_D_BENCHMARK_DIR}")"
      GIT_TERMINAL_PROMPT=0 git clone https://github.com/llm-d/llm-d-benchmark.git "${LLM_D_BENCHMARK_DIR}"
      touch "${LLM_D_BENCHMARK_DIR}/.agentgateway-benchmark-managed"
    fi

    log "checking out llm-d-benchmark @ ${LLM_D_BENCHMARK_REF}"
    (cd "${LLM_D_BENCHMARK_DIR}" && GIT_TERMINAL_PROMPT=0 git fetch origin "${LLM_D_BENCHMARK_REF}" && git checkout FETCH_HEAD)

    # --uv, not --no-uv/-y: this machine's plain `python3` is often an old
    # system Python (e.g. macOS's 3.9), and -y forces using it as-is with no
    # version check. --uv lets install.sh download and use a real 3.11+
    # itself instead of failing on whatever `python3` happens to resolve to.
    # Force the checkout-local venv even if the caller has another environment
    # active. CI/noninteractive settings and closed stdin make unexpected
    # prompts fail instead of hanging a CI job. Homebrew may otherwise update
    # itself and formula metadata implicitly before install/upgrade commands.
    log "running llm-d-benchmark's install.sh"
    (
      cd "${LLM_D_BENCHMARK_DIR}"
      unset VIRTUAL_ENV CONDA_PREFIX
      CI=1 \
        NONINTERACTIVE=1 \
        DEBIAN_FRONTEND=noninteractive \
        GIT_TERMINAL_PROMPT=0 \
        HOMEBREW_NO_AUTO_UPDATE=1 \
        LLMDBENCH_VENV_DIR="${LLM_D_BENCHMARK_DIR}/.venv" \
        ./install.sh --uv </dev/null
    )
  fi

  if [[ ! -x "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" || \
        ! -x "${LLM_D_BENCHMARK_DIR}/.venv/bin/llmdbenchmark" ]]; then
    log "llm-d-benchmark venv is incomplete at ${LLM_D_BENCHMARK_DIR}/.venv"
    return 1
  fi
  ensure_helm_diff_compatibility
}

resolve_router_chart_digest() {
  if [[ "${BENCHMARK_TREATMENT}" == "service" || \
        "${BENCHMARK_ROUTER_MODE}" != "standalone" ]]; then
    return
  fi
  local chart_output
  chart_output="$(helm show chart \
    oci://ghcr.io/llm-d/charts/llm-d-router-standalone \
    --version "${BENCHMARK_ROUTER_CHART_VERSION}" 2>&1)" || {
      log "could not resolve llm-d-router standalone ${BENCHMARK_ROUTER_CHART_VERSION}"
      return 1
    }
  ROUTER_CHART_DIGEST="$(awk '/^Digest: sha256:/ {print $2; exit}' <<<"${chart_output}")"
  [[ "${ROUTER_CHART_DIGEST}" =~ ^sha256:[0-9a-f]{64}$ ]] || {
    log "could not parse the llm-d-router chart digest"
    return 1
  }
  log "resolved llm-d-router standalone ${BENCHMARK_ROUTER_CHART_VERSION} to ${ROUTER_CHART_DIGEST}"
}

resolve_campaign_input_hashes() {
  local scenario_source
  if [[ "${BENCHMARK_SCENARIO}" == "agentgateway-comparison" ]]; then
    scenario_source="${AGTW_BENCHMARKING_DIR}/scenarios/defaults/agentgateway-comparison.yaml"
  else
    scenario_source="${LLM_D_BENCHMARK_DIR}/config/scenarios/guides/${BENCHMARK_SCENARIO}.yaml"
  fi
  [[ -f "${scenario_source}" ]] || {
    log "unknown BENCHMARK_SCENARIO=${BENCHMARK_SCENARIO}; use --list-scenarios"
    return 2
  }
  SCENARIO_SOURCE_SHA256="$(shasum -a 256 "${scenario_source}" | awk '{print $1}')"

  local -a shared_inputs=(
    "${AGTW_BENCHMARKING_DIR}/scenarios/common.yaml"
    "${AGTW_BENCHMARKING_DIR}/scenarios/accelerators/types/${BENCHMARK_ACCELERATOR_TYPE}.yaml"
    "${AGTW_BENCHMARKING_DIR}/scenarios/backends/${BENCHMARK_BACKEND_TYPE}.yaml"
    "${AGTW_BENCHMARKING_DIR}/scenarios/providers/${BENCHMARK_CLUSTER_PROVIDER}.yaml"
    "${scenario_source}"
  )
  local optional_input
  for optional_input in \
    "${AGTW_BENCHMARKING_DIR}/scenarios/backends/${BENCHMARK_BACKEND_TYPE}-${BENCHMARK_ACCELERATOR_TYPE}.yaml" \
    "${AGTW_BENCHMARKING_DIR}/scenarios/accelerators/models/${BENCHMARK_ACCELERATOR_MODEL}.yaml" \
    "${AGTW_BENCHMARKING_DIR}/scenarios/providers/${BENCHMARK_CLUSTER_PROVIDER}-${BENCHMARK_ACCELERATOR_TYPE}.yaml"; do
    [[ ! -f "${optional_input}" ]] || shared_inputs+=("${optional_input}")
  done
  if [[ "${BENCHMARK_ROUTING_POLICY}" != "default" ]]; then
    shared_inputs+=(
      "${AGTW_BENCHMARKING_DIR}/scenarios/routing/upstream-common.yaml"
      "${AGTW_BENCHMARKING_DIR}/scenarios/routing/${BENCHMARK_ROUTING_POLICY}.yaml"
    )
  fi
  if [[ -n "${BENCHMARK_REFERENCE_PROFILE}" ]]; then
    shared_inputs+=(
      "${AGTW_BENCHMARKING_DIR}/references/${BENCHMARK_REFERENCE_PROFILE}.yaml"
    )
  fi
  SHARED_CONFIG_SHA256="$({
    local input
    for input in "${shared_inputs[@]}"; do
      [[ -f "${input}" ]] || {
        log "missing shared campaign input: ${input}"
        return 1
      }
      printf '%s  %s\n' "$(shasum -a 256 "${input}" | awk '{print $1}')" "${input##*/}"
    done
  } | shasum -a 256 | awk '{print $1}')"

  if [[ -n "${WORKLOAD_FILE_PATH}" ]]; then
    WORKLOAD_SHA256="$(shasum -a 256 "${WORKLOAD_FILE_PATH}" | awk '{print $1}')"
    return
  fi
  local -a workload_matches=()
  while IFS= read -r match; do
    workload_matches+=("${match}")
  done < <(
    find "${LLM_D_BENCHMARK_DIR}/workload/profiles/${BENCHMARK_HARNESS}" \
      -type f \( -name "${WORKLOAD}" -o -name "${WORKLOAD}.in" \) -print | sort
  )
  if (( ${#workload_matches[@]} != 1 )); then
    log "could not uniquely resolve upstream workload ${WORKLOAD}; set BENCHMARK_WORKLOAD_FILE_PATH"
    return 2
  fi
  WORKLOAD_SHA256="$(shasum -a 256 "${workload_matches[0]}" | awk '{print $1}')"
}

# The router chart's agentgateway image preset points at a tag that doesn't
# exist upstream (see benchmarking/inference/README.md), so this needs to be loaded
# into kind by hand before an agentgateway treatment can come up.
#
# Plain `kind load docker-image` fails here: cr.agentgateway.dev publishes a
# multi-arch index with buildx attestation manifests, and on Docker Desktop's
# containerd image store `docker save` keeps that whole index without the
# content for platforms/attestations that were never actually pulled - kind's
# `ctr images import --all-platforms` then chokes on a missing digest. Route
# around it with skopeo, which flattens to a single-platform classic tar.
load_agentgateway_image() {
  local arch platform
  arch="$(uname -m)"
  case "${arch}" in
    x86_64) platform="amd64" ;;
    aarch64|arm64) platform="arm64" ;;
    *) log "unrecognized host arch ${arch}, assuming amd64"; platform="amd64" ;;
  esac

  log "loading ${AGW_IMAGE} (linux/${platform}) into kind cluster ${CLUSTER_NAME}"
  local tar
  tar="$(mktemp "${TMPDIR:-/tmp}/agentgateway-image.XXXXXX.tar")"
  if ! skopeo copy --override-os linux --override-arch "${platform}" \
    "docker://${AGW_IMAGE}" "docker-archive:${tar}:${AGW_IMAGE}"; then
    rm -f "${tar}"
    return 1
  fi
  if ! kind load image-archive "${tar}" --name "${CLUSTER_NAME}"; then
    rm -f "${tar}"
    return 1
  fi
  rm -f "${tar}"
}

list_scenarios() {
  echo "agentgateway-comparison (wrapper default)"
  local guide
  while IFS= read -r guide; do
    basename "${guide}" .yaml
  done < <(find "${LLM_D_BENCHMARK_DIR}/config/scenarios/guides" -maxdepth 1 -type f -name '*.yaml' -print | sort)
}

render_scenario() {
  local -a overlays=(
    "${AGTW_BENCHMARKING_DIR}/scenarios/common.yaml"
    "${AGTW_BENCHMARKING_DIR}/scenarios/accelerators/types/${BENCHMARK_ACCELERATOR_TYPE}.yaml"
    "${AGTW_BENCHMARKING_DIR}/scenarios/backends/${BENCHMARK_BACKEND_TYPE}.yaml"
    "${AGTW_BENCHMARKING_DIR}/scenarios/providers/${BENCHMARK_CLUSTER_PROVIDER}.yaml"
  )
  local backend_accelerator="${AGTW_BENCHMARKING_DIR}/scenarios/backends/${BENCHMARK_BACKEND_TYPE}-${BENCHMARK_ACCELERATOR_TYPE}.yaml"
  [[ ! -f "${backend_accelerator}" ]] || overlays+=("${backend_accelerator}")
  local accelerator_model="${AGTW_BENCHMARKING_DIR}/scenarios/accelerators/models/${BENCHMARK_ACCELERATOR_MODEL}.yaml"
  [[ "${BENCHMARK_ACCELERATOR_MODEL}" == "auto" || ! -f "${accelerator_model}" ]] || overlays+=("${accelerator_model}")
  local combination="${AGTW_BENCHMARKING_DIR}/scenarios/providers/${BENCHMARK_CLUSTER_PROVIDER}-${BENCHMARK_ACCELERATOR_TYPE}.yaml"
  [[ ! -f "${combination}" ]] || overlays+=("${combination}")
  if [[ "${BENCHMARK_SCENARIO}" == "agentgateway-comparison" ]]; then
    overlays+=("${AGTW_BENCHMARKING_DIR}/scenarios/defaults/agentgateway-comparison.yaml")
  fi
  if [[ "${BENCHMARK_ROUTING_POLICY}" != "default" ]]; then
    # Backend and workload constraints belong to every treatment; only the EPP
    # plugin policy below is specific to the inference-gateway treatment.
    overlays+=("${AGTW_BENCHMARKING_DIR}/scenarios/routing/upstream-common.yaml")
  fi
  if [[ -n "${BENCHMARK_REFERENCE_PROFILE}" ]]; then
    overlays+=("${AGTW_BENCHMARKING_DIR}/references/${BENCHMARK_REFERENCE_PROFILE}.yaml")
  fi
  if [[ "${BENCHMARK_TREATMENT}" == "service" ]]; then
    overlays+=("${AGTW_BENCHMARKING_DIR}/scenarios/baseline.yaml")
  else
    overlays+=("${AGTW_BENCHMARKING_DIR}/scenarios/gateways/${BENCHMARK_GATEWAY_IMPLEMENTATION}/${BENCHMARK_ROUTER_MODE}.yaml")
    if [[ "${BENCHMARK_ROUTING_POLICY}" != "default" ]]; then
      overlays+=("${AGTW_BENCHMARKING_DIR}/scenarios/routing/${BENCHMARK_ROUTING_POLICY}.yaml")
    fi
  fi

  local -a render_args=(
    --output "${SPEC_DIR}/scenario.yaml"
    --scenario-name "${SCENARIO_NAME}"
    --treatment "${BENCHMARK_TREATMENT}"
    --gateway-implementation "${BENCHMARK_GATEWAY_IMPLEMENTATION}"
    --router-mode "${BENCHMARK_ROUTER_MODE}"
    --routing-policy "${BENCHMARK_ROUTING_POLICY}"
    --gateway-image "${AGW_IMAGE}"
    --agentgateway-version "${AGW_VERSION}"
    --harness "${BENCHMARK_HARNESS}"
    --router-chart-version "${BENCHMARK_ROUTER_CHART_VERSION}"
    --workload "${WORKLOAD}"
    --accelerator-type "${BENCHMARK_ACCELERATOR_TYPE}"
    --accelerator-model "${BENCHMARK_ACCELERATOR_MODEL}"
    --backend-type "${BENCHMARK_BACKEND_TYPE}"
    --model "${MODEL}"
    --replicas "${REPLICAS}"
    --tensor-parallelism "${TENSOR_PARALLELISM}"
    --gke-accelerator "${GKE_ACCELERATOR}"
    --model-storage-class "${MODEL_STORAGE_CLASS}"
    --model-storage-size "${MODEL_STORAGE_SIZE}"
    --workload-storage-class "${WORKLOAD_STORAGE_CLASS}"
    --workload-storage-size "${WORKLOAD_STORAGE_SIZE}"
    --cpu-nodepool "${BENCHMARK_GKE_CPU_NODEPOOL:-}"
    --harness-nodepool "${BENCHMARK_GKE_HARNESS_NODEPOOL:-}"
  )
  local overlay
  for overlay in "${overlays[@]}"; do
    [[ -f "${overlay}" ]] || { log "missing scenario adapter: ${overlay}"; return 1; }
    render_args+=(--overlay "${overlay}")
  done
  if [[ "${BENCHMARK_SCENARIO}" != "agentgateway-comparison" ]]; then
    local guide="${LLM_D_BENCHMARK_DIR}/config/scenarios/guides/${BENCHMARK_SCENARIO}.yaml"
    [[ -f "${guide}" ]] || { log "unknown BENCHMARK_SCENARIO=${BENCHMARK_SCENARIO}; use --list-scenarios"; return 2; }
    render_args+=(--guide "${guide}" --guide-entry "${BENCHMARK_SCENARIO}")
  fi
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" \
    "${AGTW_BENCHMARKING_DIR}/scripts/render-scenario.py" "${render_args[@]}"

  if [[ "${BENCHMARK_TREATMENT}" != "service" && \
        "${BENCHMARK_ROUTER_MODE}" == "standalone" ]]; then
    STANDALONE_INVARIANT_SHA256="$(
      "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" \
        "${AGTW_BENCHMARKING_DIR}/scripts/standalone-invariant.py" \
        --input "${SPEC_DIR}/scenario.yaml" \
        --output "${SPEC_DIR}/standalone-invariant.yaml"
    )"
    [[ "${STANDALONE_INVARIANT_SHA256}" =~ ^[0-9a-f]{64}$ ]] || {
      log "could not compute the standalone comparison invariant"
      return 1
    }
  fi

  CONFIG_SHA256="$(
    { shasum -a 256 "${SPEC_DIR}/scenario.yaml"; \
      printf '%s\n' "${WORKLOAD}" "${BENCHMARK_BACKEND_TYPE}" \
        "${BENCHMARK_ACCELERATOR_TYPE}" "${MODEL_CACHE_ID}" \
        "${BENCHMARK_ROUTING_POLICY}" "${BENCHMARK_WORKLOAD_VARIANT}" \
        "${ROUTER_CHART_DIGEST}" \
        "${RUNTIME_METRICS_ENABLED}" "${METRICS_INTERVAL}" \
        "${GKE_MONITORING_MODE}"; } | \
      shasum -a 256 | awk '{print $1}'
  )"
}

write_spec() {
  cat > "${SPEC_DIR}/spec.yaml" <<EOF
base_dir: ${LLM_D_BENCHMARK_DIR}
values_file:
  path: ${LLM_D_BENCHMARK_DIR}/config/templates/values/defaults.yaml
template_dir:
  path: ${LLM_D_BENCHMARK_DIR}/config/templates/jinja
scenario_file:
  path: ${SPEC_DIR}/scenario.yaml
EOF
}

# Resolve the in-cluster address explicitly. llm-d-benchmark's gateway
# discovery prefers Gateway.status.addresses, which is an external address for
# a GKE LoadBalancer Service. Passing this URL to the run phase keeps the
# measured path inside the cluster and matches the upstream published runs.
resolve_internal_endpoint() {
  local resources
  resources="$(mktemp "${TMPDIR:-/tmp}/agentgateway-endpoints.XXXXXX.json")"
  kubectl get service,gateway --namespace "${SCENARIO_NAME}" -o json \
    > "${resources}"
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - \
      "${resources}" "${BENCHMARK_TREATMENT}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    document = json.load(stream)
treatment = sys.argv[2]
items = document.get("items", [])
services = [item for item in items if item.get("kind") == "Service"]
gateways = [item for item in items if item.get("kind") == "Gateway"]

def service_port(service):
    ports = service.get("spec", {}).get("ports", [])
    for port in ports:
        if port.get("name") == "http" or port.get("port") == 80:
            return int(port.get("port", 80))
    return int(ports[0].get("port", 80)) if ports else 80

def choose(suffix):
    matches = [
        service for service in services
        if service.get("metadata", {}).get("name", "").endswith(suffix)
    ]
    if len(matches) != 1:
        raise SystemExit(
            f"expected one Service ending in {suffix!r}; found "
            f"{[item.get('metadata', {}).get('name') for item in matches]}"
        )
    return matches[0]

if treatment == "service":
    selected = choose("-direct")
elif treatment in ("envoy-standalone", "agentgateway-standalone"):
    # Both standalone implementations are deployed behind the router-epp
    # Service. Selecting that ClusterIP keeps their measured network path
    # identical and avoids the external GKE load balancer.
    selected = choose("-router-epp")
elif treatment == "agentgateway-gateway":
    if len(gateways) != 1:
        raise SystemExit(f"expected one Gateway; found {len(gateways)}")
    gateway_name = gateways[0].get("metadata", {}).get("name")
    matches = [
        service for service in services
        if service.get("metadata", {}).get("labels", {}).get(
            "gateway.networking.k8s.io/gateway-name"
        ) == gateway_name
    ]
    if len(matches) != 1:
        raise SystemExit(
            f"expected one data-plane Service for Gateway {gateway_name!r}; "
            f"found {[item.get('metadata', {}).get('name') for item in matches]}"
        )
    selected = matches[0]
else:
    raise SystemExit(f"internal endpoint selection is not implemented for {treatment}")

spec = selected.get("spec", {})
cluster_ip = spec.get("clusterIP")
if not cluster_ip or cluster_ip == "None":
    raise SystemExit("selected Service has no routable ClusterIP")
print(f"http://{cluster_ip}:{service_port(selected)}")
PY
  local status=$?
  rm -f -- "${resources}"
  return "${status}"
}

verify_reference_runtime() {
  local expected_vllm_version
  case "${BENCHMARK_REFERENCE_PROFILE}" in
    optimized-baseline-qwen3-32b-h100-v0.9)
      expected_vllm_version="v0.23.0"
      ;;
    optimized-baseline-qwen3-32b-h100-v0.9-vllm-v0.27.1)
      expected_vllm_version="v0.27.1"
      ;;
    *) return 0 ;;
  esac
  local inventory="${SPEC_DIR}/runtime-image-inventory.txt"
  local pod_json
  pod_json="$(mktemp "${TMPDIR:-/tmp}/agentgateway-pods.XXXXXX.json")"
  kubectl get pods --namespace "${SCENARIO_NAME}" -o json > "${pod_json}"
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - \
    "${pod_json}" > "${inventory}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    pods = json.load(stream)
for pod in pods.get("items", []):
    name = pod.get("metadata", {}).get("name", "")
    for container in pod.get("spec", {}).get("containers", []):
        print(f"{name}\t{container.get('name', '')}\t{container.get('image', '')}")
PY
  rm -f -- "${pod_json}"
  local decode_count
  decode_count="$(awk -F '\t' -v image="docker.io/vllm/vllm-openai:${expected_vllm_version}" \
    '$1 ~ /-decode-/ && $3 == image {count++} END {print count+0}' "${inventory}")"
  (( decode_count == REPLICAS )) || {
    log "reference preflight expected ${REPLICAS} vLLM ${expected_vllm_version} decode containers; found ${decode_count}"
    cat "${inventory}" >&2
    return 1
  }
  if [[ "${BENCHMARK_TREATMENT}" != "service" ]]; then
    grep -Fq $'\tghcr.io/llm-d/llm-d-router-endpoint-picker:v0.9.0' \
      "${inventory}" || {
      log "reference preflight did not observe EPP v0.9.0"
      cat "${inventory}" >&2
      return 1
    }
  fi
  if [[ "${BENCHMARK_TREATMENT}" == agentgateway-* ]]; then
    grep -Fq $'\tcr.agentgateway.dev/agentgateway:v1.4.1' \
      "${inventory}" || {
      log "reference preflight did not observe agentgateway v1.4.1"
      cat "${inventory}" >&2
      return 1
    }
  fi
  if [[ "${BENCHMARK_TREATMENT}" == "agentgateway-gateway" ]]; then
    local controller_images
    controller_images="$(kubectl get deployments \
      --namespace "${AGW_CONTROLLER_NAMESPACE}" \
      -l app.kubernetes.io/name=agentgateway \
      -o jsonpath='{range .items[*].spec.template.spec.containers[*]}{.image}{"\n"}{end}')"
    grep -Fxq "cr.agentgateway.dev/controller:v1.4.1" \
      <<<"${controller_images}" || {
      log "reference preflight did not observe agentgateway controller v1.4.1"
      printf '%s\n' "${controller_images}" >&2
      return 1
    }
  fi
  log "reference preflight: observed the locked runtime component versions"
}

configure_agentgateway_gateway_image() {
  [[ "${BENCHMARK_TREATMENT}" == "agentgateway-gateway" ]] || return 0

  local image_without_tag registry repository tag gateway_name parameters_name
  [[ "${AGW_IMAGE}" == */*:* && "${AGW_IMAGE}" != *@* ]] || {
    log "gateway mode requires AGW_IMAGE in registry/repository:tag form; got ${AGW_IMAGE}"
    return 1
  }
  image_without_tag="${AGW_IMAGE%:*}"
  registry="${image_without_tag%%/*}"
  repository="${image_without_tag#*/}"
  tag="${AGW_IMAGE##*:}"
  gateway_name="$(kubectl get gateways.gateway.networking.k8s.io \
    --namespace "${SCENARIO_NAME}" \
    -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}')"
  [[ -n "${gateway_name}" && "${gateway_name}" != *$'\n'* ]] || {
    log "expected exactly one Gateway in ${SCENARIO_NAME}"
    return 1
  }
  parameters_name="${gateway_name}-benchmark-image"

  # llm-d-benchmark currently configures the controller chart version but not
  # the Gateway data-plane image. Pin it through the native parametersRef so a
  # release comparison cannot silently run the controller's older default.
  kubectl apply --namespace "${SCENARIO_NAME}" -f - >/dev/null <<EOF
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayParameters
metadata:
  name: ${parameters_name}
spec:
  image:
    registry: ${registry}
    repository: ${repository}
    tag: ${tag}
    pullPolicy: IfNotPresent
EOF
  kubectl patch gateway.gateway.networking.k8s.io "${gateway_name}" \
    --namespace "${SCENARIO_NAME}" --type merge \
    --patch "{\"spec\":{\"infrastructure\":{\"parametersRef\":{\"group\":\"agentgateway.dev\",\"kind\":\"AgentgatewayParameters\",\"name\":\"${parameters_name}\"}}}}" \
    >/dev/null

  local observed="" ready=""
  for _ in {1..60}; do
    observed="$(kubectl get pods --namespace "${SCENARIO_NAME}" \
      -l "gateway.networking.k8s.io/gateway-name=${gateway_name}" \
      -o jsonpath='{range .items[*].spec.containers[*]}{.image}{"\n"}{end}')"
    ready="$(kubectl get pods --namespace "${SCENARIO_NAME}" \
      -l "gateway.networking.k8s.io/gateway-name=${gateway_name}" \
      -o jsonpath='{range .items[*].status.containerStatuses[*]}{.ready}{"\n"}{end}')"
    if grep -Fxq "${AGW_IMAGE}" <<<"${observed}" && \
      [[ -n "${ready}" ]] && ! grep -Fxq false <<<"${ready}"; then
      log "agentgateway data plane: observed ${AGW_IMAGE} via AgentgatewayParameters"
      return 0
    fi
    sleep 5
  done
  log "timed out waiting for agentgateway data plane ${AGW_IMAGE}; observed: ${observed:-none}"
  return 1
}

ensure_agentgateway_gateway_controller() {
  [[ "${BENCHMARK_TREATMENT}" == "agentgateway-gateway" ]] || return 0

  # llm-d-benchmark treats the presence of agentgateway CRDs as sufficient and
  # can therefore leave an older controller installed. Reconcile the requested
  # release before standup so the controller and its generated proxy are an
  # intentional, reproducible pair.
  helm upgrade --install agentgateway \
    oci://cr.agentgateway.dev/charts/agentgateway \
    --kube-context "${BENCHMARK_KUBE_CONTEXT}" \
    --namespace "${AGW_CONTROLLER_NAMESPACE}" --create-namespace \
    --version "${AGW_VERSION}" --reuse-values --wait --timeout 5m >/dev/null
  local image
  image="$(kubectl get deployment agentgateway \
    --namespace "${AGW_CONTROLLER_NAMESPACE}" \
    -o jsonpath='{.spec.template.spec.containers[0].image}')"
  [[ "${image}" == "cr.agentgateway.dev/controller:${AGW_VERSION}" ]] || {
    log "agentgateway controller reconciliation expected v${AGW_VERSION}; observed ${image:-none}"
    return 1
  }
  log "agentgateway controller: observed ${image}"
}

# Pin each invocation to a known workspace so result discovery never depends
# on parsing a random temporary path from llm-d-benchmark's stdout.
workspace_dir() {
  echo "${SPEC_DIR}/workspace-${RUN_ID}"
}

llmdbench() {
  (cd "${LLM_D_BENCHMARK_DIR}" && .venv/bin/llmdbenchmark "$@")
}

rendered_stack_value() {
  local expression="$1"
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - \
    "${LLM_D_BENCHMARK_DIR}/config/templates/values/defaults.yaml" \
    "${SPEC_DIR}/scenario.yaml" "${expression}" <<'PY'
import copy
import sys

import yaml

defaults_path, scenario_path, expression = sys.argv[1:]
with open(defaults_path, encoding="utf-8") as stream:
    defaults = yaml.safe_load(stream) or {}
with open(scenario_path, encoding="utf-8") as stream:
    entry = (yaml.safe_load(stream) or {})["scenario"][0]


def merge(left, right):
    result = copy.deepcopy(left)
    for key, value in right.items():
        if isinstance(value, dict) and isinstance(result.get(key), dict):
            result[key] = merge(result[key], value)
        else:
            result[key] = copy.deepcopy(value)
    return result


stack = merge(defaults, entry.get("common", {}))
stack = merge(stack, entry.get("modelservice", {}))
value = stack
for key in expression.split("."):
    value = value.get(key) if isinstance(value, dict) else None
if value is not None:
    print(value)
PY
}

runtime_collector_image() {
  local repository tag
  repository="$(rendered_stack_value images.benchmark.repository)"
  tag="$(rendered_stack_value images.benchmark.tag)"
  [[ -n "${repository}" && -n "${tag}" ]] || {
    log "could not resolve the llm-d-benchmark collector image"
    return 1
  }
  echo "${repository}:${tag}"
}

runtime_collector_service_account() {
  local account
  account="$(kubectl get rolebinding inference-perf-job-creator-binding \
    --namespace "${SCENARIO_NAME}" \
    -o jsonpath='{.subjects[0].name}' 2>/dev/null || true)"
  echo "${account:-inference-perf-runner}"
}

# Run llm-d-benchmark's direct collector outside the harness pod until
# llm-d-benchmark#1766 decouples metricsScrapeEnabled from Prometheus Operator
# CRD validation. The companion entrypoint adds agentgateway samples to the
# same raw directory before invoking upstream process_metrics.py.
start_runtime_metrics() {
  [[ "${RUNTIME_METRICS_ENABLED}" == "true" ]] || return 0

  local image service_account vllm_port manifest
  image="$(runtime_collector_image)"
  service_account="$(runtime_collector_service_account)"
  vllm_port="$(rendered_stack_value decode.vllm.port)"
  vllm_port="${vllm_port:-8200}"
  manifest="${SPEC_DIR}/runtime-metrics-collector.yaml"
  RUNTIME_METRICS_CAPTURE_DIR="${SPEC_DIR}/runtime-metrics-${RUN_ID}"

  if [[ -e "${RUNTIME_METRICS_CAPTURE_DIR}" || -L "${RUNTIME_METRICS_CAPTURE_DIR}" ]]; then
    log "refusing to overwrite runtime metrics capture: ${RUNTIME_METRICS_CAPTURE_DIR}"
    return 1
  fi

  # These names are owned exclusively by this wrapper inside its treatment
  # namespace. Remove a stale collector left by an interrupted previous run.
  kubectl delete pod "${RUNTIME_COLLECTOR_NAME}" --namespace "${SCENARIO_NAME}" \
    --ignore-not-found --wait=true >/dev/null
  kubectl create configmap "${RUNTIME_COLLECTOR_CONFIGMAP}" \
    --namespace "${SCENARIO_NAME}" \
    --from-file=collect_metrics.sh="${LLM_D_BENCHMARK_DIR}/workload/harnesses/collect_metrics.sh" \
    --from-file=process_metrics.py="${LLM_D_BENCHMARK_DIR}/workload/harnesses/process_metrics.py" \
    --from-file=process_epp_logs.py="${LLM_D_BENCHMARK_DIR}/workload/harnesses/process_epp_logs.py" \
    --from-file=runtime-metrics-collector.sh="${AGTW_BENCHMARKING_DIR}/scripts/runtime-metrics-collector.sh" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

  local -a render_args=(
    --output "${manifest}"
    --namespace "${SCENARIO_NAME}"
    --service-account "${service_account}"
    --image "${image}"
    --name "${RUNTIME_COLLECTOR_NAME}"
    --configmap "${RUNTIME_COLLECTOR_CONFIGMAP}"
    --interval "${METRICS_INTERVAL}"
    --vllm-port "${vllm_port}"
  )
  if [[ -n "${BENCHMARK_GKE_HARNESS_NODEPOOL:-}" ]]; then
    render_args+=(--nodepool "${BENCHMARK_GKE_HARNESS_NODEPOOL}")
  fi
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" \
    "${AGTW_BENCHMARKING_DIR}/scripts/render-runtime-collector.py" \
    "${render_args[@]}"
  kubectl apply -f "${manifest}" >/dev/null
  RUNTIME_COLLECTOR_ACTIVE=true
  if ! kubectl wait pod/"${RUNTIME_COLLECTOR_NAME}" \
      --namespace "${SCENARIO_NAME}" --for=condition=Ready --timeout=180s; then
    kubectl describe pod/"${RUNTIME_COLLECTOR_NAME}" \
      --namespace "${SCENARIO_NAME}" >&2 || true
    return 1
  fi
  log "runtime metrics: collecting vLLM, EPP, agentgateway, and resource samples every ${METRICS_INTERVAL}s"
}

capture_gke_monitoring_status() {
  [[ -f "${SPEC_DIR}/gke-podmonitoring.yaml" ]] || return 0
  kubectl get podmonitorings.monitoring.googleapis.com \
    --namespace "${SCENARIO_NAME}" -o yaml \
    > "${SPEC_DIR}/gke-podmonitoring-status.yaml" 2>&1 || true
}

verify_gke_monitoring() {
  [[ "${GKE_MONITORING_MODE}" == "required" && \
      -f "${SPEC_DIR}/gke-podmonitoring.yaml" ]] || return 0
  local healthy=false snapshot
  for _ in {1..12}; do
    snapshot="$(kubectl get podmonitorings.monitoring.googleapis.com \
      --namespace "${SCENARIO_NAME}" -o json)"
    if SNAPSHOT="${snapshot}" "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - <<'PY'
import json
import os

items = json.loads(os.environ["SNAPSHOT"]).get("items", [])
if not items:
    raise SystemExit(1)
for item in items:
    statuses = item.get("status", {}).get("endpointStatuses", [])
    if not statuses:
        raise SystemExit(1)
    for status in statuses:
        if status.get("activeTargets", 0) < 1 or status.get("unhealthyTargets", 0) > 0:
            raise SystemExit(1)
PY
    then
      healthy=true
      break
    fi
    sleep 5
  done
  if [[ "${healthy}" != "true" ]]; then
    capture_gke_monitoring_status
    log "GKE PodMonitoring targets did not become healthy"
    return 1
  fi
}

stop_runtime_metrics() {
  [[ "${RUNTIME_COLLECTOR_ACTIVE}" == "true" ]] || return 0
  local capture_ready=false phase="" status=0
  kubectl exec --namespace "${SCENARIO_NAME}" "${RUNTIME_COLLECTOR_NAME}" \
    -- touch /control/stop >/dev/null 2>&1 || true
  for _ in {1..60}; do
    phase="$(kubectl get pod "${RUNTIME_COLLECTOR_NAME}" \
      --namespace "${SCENARIO_NAME}" -o jsonpath='{.status.phase}' 2>/dev/null || true)"
    if [[ "${phase}" == "Running" ]] && kubectl exec \
        --namespace "${SCENARIO_NAME}" "${RUNTIME_COLLECTOR_NAME}" \
        -- test -e /control/complete >/dev/null 2>&1; then
      capture_ready=true
      break
    fi
    [[ "${phase}" == "Succeeded" || "${phase}" == "Failed" ]] && break
    sleep 5
  done

  mkdir -p "${RUNTIME_METRICS_CAPTURE_DIR}"
  if [[ "${capture_ready}" != "true" ]]; then
    log "runtime metrics collector did not signal that artifacts were ready (phase=${phase:-unknown})"
    status=1
  # A full upstream saturation ladder creates thousands of small samples.
  # Both kubectl cp and a live compressed exec stream can terminate early at
  # that scale. Finalize one archive in the stopped collector, download it
  # with bounded retries, and verify the complete gzip before extracting it.
  else
    local archive copied=false attempt
    archive="$(mktemp "${TMPDIR:-/tmp}/agentgateway-runtime-metrics.XXXXXX")"
    if ! kubectl exec --namespace "${SCENARIO_NAME}" \
        "${RUNTIME_COLLECTOR_NAME}" -- sh -c \
        'rm -f /tmp/runtime-metrics.tar.gz && tar -C /runtime-metrics -czf /tmp/runtime-metrics.tar.gz .'; then
      log "could not archive runtime metrics in collector pod"
      status=1
    else
      for attempt in {1..3}; do
        if kubectl exec --namespace "${SCENARIO_NAME}" \
            "${RUNTIME_COLLECTOR_NAME}" -- \
            cat /tmp/runtime-metrics.tar.gz > "${archive}" && \
            tar -tzf "${archive}" >/dev/null 2>&1; then
          copied=true
          break
        fi
        log "runtime metrics archive transfer failed integrity check (attempt ${attempt}/3)"
      done
      if [[ "${copied}" == "true" ]]; then
        tar -C "${RUNTIME_METRICS_CAPTURE_DIR}" -xzf "${archive}" || status=1
      else
        log "could not copy a complete runtime metrics archive from collector pod"
        status=1
      fi
      kubectl exec --namespace "${SCENARIO_NAME}" \
        "${RUNTIME_COLLECTOR_NAME}" -- \
        rm -f /tmp/runtime-metrics.tar.gz >/dev/null 2>&1 || true
    fi
    rm -f -- "${archive}"
  fi
  kubectl exec --namespace "${SCENARIO_NAME}" "${RUNTIME_COLLECTOR_NAME}" \
    -- touch /control/copied >/dev/null 2>&1 || true
  for _ in {1..36}; do
    phase="$(kubectl get pod "${RUNTIME_COLLECTOR_NAME}" \
      --namespace "${SCENARIO_NAME}" -o jsonpath='{.status.phase}' 2>/dev/null || true)"
    [[ "${phase}" == "Succeeded" || "${phase}" == "Failed" ]] && break
    sleep 5
  done
  if [[ "${phase}" != "Succeeded" ]]; then
    log "runtime metrics collector ended in ${phase:-unknown} phase"
    kubectl logs "${RUNTIME_COLLECTOR_NAME}" --namespace "${SCENARIO_NAME}" >&2 || true
    status=1
  fi
  capture_gke_monitoring_status
  kubectl delete -f "${SPEC_DIR}/runtime-metrics-collector.yaml" \
    --ignore-not-found --wait=true >/dev/null 2>&1 || status=1
  kubectl delete configmap "${RUNTIME_COLLECTOR_CONFIGMAP}" \
    --namespace "${SCENARIO_NAME}" --ignore-not-found >/dev/null 2>&1 || status=1
  RUNTIME_COLLECTOR_ACTIVE=false
  return "${status}"
}

request_runtime_metrics_stop() {
  [[ "${RUNTIME_COLLECTOR_ACTIVE}" == "true" ]] || return 0
  kubectl exec --namespace "${SCENARIO_NAME}" "${RUNTIME_COLLECTOR_NAME}" \
    -- touch /control/stop >/dev/null 2>&1 || true
}

expected_final_stage_id() {
  local profile_path="${WORKLOAD_FILE_PATH}"
  if [[ -z "${profile_path}" ]]; then
    profile_path="${LLM_D_BENCHMARK_DIR}/workload/profiles/${BENCHMARK_HARNESS}/${WORKLOAD}"
  fi
  [[ -f "${profile_path}" ]] || return 0
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - "${profile_path}" <<'PY'
import sys
import yaml

with open(sys.argv[1], encoding="utf-8") as stream:
    stages = (yaml.safe_load(stream) or {}).get("load", {}).get("stages")
if isinstance(stages, list) and stages:
    print(len(stages) - 1)
PY
}

traffic_complete_visible() {
  local final_stage_id="$1" pod logs
  while IFS= read -r pod; do
    [[ -n "${pod}" ]] || continue
    if kubectl exec --namespace "${SCENARIO_NAME}" "${pod}" -- \
        sh -c 'find /requests -type f -name traffic_complete.yaml -print -quit | grep -q .' \
        >/dev/null 2>&1; then
      TRAFFIC_COMPLETE_SOURCE=llmdbench-marker
      return 0
    fi
    logs="$(kubectl logs --namespace "${SCENARIO_NAME}" "${pod}" \
      --all-containers --tail=4000 2>&1 || true)"
    if grep -Fq 'LLMDBENCH_EVENT_V1 traffic_complete' <<<"${logs}"; then
      TRAFFIC_COMPLETE_SOURCE=llmdbench-event
      return 0
    fi
    # Temporary fallback until llm-d-benchmark#1796 is fixed upstream. The
    # native inference-perf message is stable today but is not a public
    # llm-d-benchmark lifecycle contract.
    if [[ -n "${final_stage_id}" ]] && \
        grep -Fq "Stage ${final_stage_id} - run completed" <<<"${logs}"; then
      TRAFFIC_COMPLETE_SOURCE=inference-perf-log-fallback
      return 0
    fi
  done < <(
    kubectl get pods --namespace "${SCENARIO_NAME}" \
      -l app=llmdbench-harness-launcher -o name 2>/dev/null || true
  )
  return 1
}

wait_for_traffic_complete() {
  local run_pid="$1" final_stage_id
  final_stage_id="$(expected_final_stage_id)"
  [[ "${final_stage_id}" =~ ^[0-9]+$ ]] || {
    log "after-load GPU release requires an inference-perf profile with explicit stages"
    return 1
  }
  log "GPU release: waiting for traffic-complete event after stage ${final_stage_id}"
  while kill -0 "${run_pid}" >/dev/null 2>&1; do
    if traffic_complete_visible "${final_stage_id}"; then
      log "GPU release: traffic complete (${TRAFFIC_COMPLETE_SOURCE})"
      return 0
    fi
    sleep 5
  done
  traffic_complete_visible "${final_stage_id}"
}

gpu_workload_targets() {
  kubectl get deployments,statefulsets --namespace "${SCENARIO_NAME}" \
    -o json | "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" -c '
import json
import sys

for item in json.load(sys.stdin).get("items", []):
    spec = item.get("spec", {}).get("template", {}).get("spec", {})
    containers = spec.get("initContainers", []) + spec.get("containers", [])
    gpu = 0
    for container in containers:
        resources = container.get("resources", {})
        for key in ("requests", "limits"):
            value = resources.get(key, {}).get("nvidia.com/gpu", 0)
            try:
                gpu = max(gpu, int(value))
            except (TypeError, ValueError):
                pass
    if gpu:
        print("{}/{}".format(item["kind"].lower(), item["metadata"]["name"]))
'
}

release_gke_gpu_pool() {
  local traffic_complete_at scale_requested_at pool_name nodes_output node_count
  local deadline=$((SECONDS + GKE_CLEANUP_TIMEOUT)) target
  local -a targets=()
  traffic_complete_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  GPU_RELEASE_EVIDENCE="${SPEC_DIR}/gpu-release.yaml"

  while IFS= read -r target; do
    [[ -n "${target}" ]] || continue
    targets+=("${target}")
  done < <(gpu_workload_targets)
  if (( ${#targets[@]} == 0 )); then
    log "GPU release: no GPU workload controllers found in ${SCENARIO_NAME}"
    return 1
  fi
  log "GPU release: scaling ${#targets[@]} model workload(s) to zero"
  for target in "${targets[@]}"; do
    kubectl scale "${target}" --namespace "${SCENARIO_NAME}" --replicas=0
  done

  pool_name="$(gcloud container node-pools describe "${GKE_GPU_NODEPOOL}" \
    --cluster "${GKE_CLUSTER}" --project "${GKE_PROJECT}" \
    --location "${GKE_LOCATION}" --format='value(name)')"
  [[ "${pool_name}" == "${GKE_GPU_NODEPOOL}" ]] || {
    log "GPU release: could not validate node pool ${GKE_GPU_NODEPOOL}"
    return 1
  }
  scale_requested_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  log "GPU release: scaling ${GKE_GPU_NODEPOOL} to zero while reporting continues"
  gcloud container clusters resize "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --node-pool "${GKE_GPU_NODEPOOL}" --num-nodes 0 --quiet

  while true; do
    nodes_output="$(kubectl get nodes \
      -l "cloud.google.com/gke-nodepool=${GKE_GPU_NODEPOOL}" -o name 2>&1)" || {
        log "GPU release: could not verify GPU nodes: ${nodes_output}"
        return 1
      }
    node_count="$(awk 'NF {count++} END {print count+0}' <<<"${nodes_output}")"
    [[ "${node_count}" == "0" ]] && break
    if (( SECONDS >= deadline )); then
      log "GPU release: ${node_count} GPU node(s) remain after timeout"
      return 1
    fi
    sleep 10
  done

  local evidence_tmp="${GPU_RELEASE_EVIDENCE}.tmp"
  printf 'schema_version: 1\npolicy: "%s"\nupstream_issue: "llm-d/llm-d-benchmark#1796"\ntraffic_complete_source: "%s"\ntraffic_complete_detected_at: "%s"\nnode_pool_scale_requested_at: "%s"\nnode_pool_zero_at: "%s"\nnode_pool: "%s"\naccelerators_released: %s\n' \
    "${GPU_RELEASE_POLICY}" "${TRAFFIC_COMPLETE_SOURCE:-run-returned}" \
    "${traffic_complete_at}" "${scale_requested_at}" \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${GKE_GPU_NODEPOOL}" \
    "$((REPLICAS * TENSOR_PARALLELISM))" > "${evidence_tmp}"
  mv "${evidence_tmp}" "${GPU_RELEASE_EVIDENCE}"
  log "GPU release: verified ${GKE_GPU_NODEPOOL} is at zero"
}

configure_gke_monitoring() {
  [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" ]] || return 0
  [[ "${GKE_MONITORING_MODE}" != "off" ]] || return 0

  if ! kubectl get crd podmonitorings.monitoring.googleapis.com >/dev/null 2>&1; then
    if [[ "${GKE_MONITORING_MODE}" == "required" ]]; then
      log "GKE Managed Prometheus PodMonitoring CRD is missing"
      return 1
    fi
    log "warning: GKE Managed Prometheus is unavailable; direct runtime metrics remain enabled"
    log "enable it with: gcloud container clusters update <cluster> --location=<location> --enable-managed-prometheus"
    return 0
  fi

  # Application PodMonitoring endpoints do not expose container CPU/memory.
  # Those durable series come from GKE's curated cAdvisor/Kubelet packages;
  # keep direct `kubectl top` snapshots as a portable fallback.
  if command -v gcloud >/dev/null && \
      [[ "${BENCHMARK_KUBE_CONTEXT}" =~ ^gke_([^_]+)_([^_]+)_(.+)$ ]]; then
    local gke_project="${BASH_REMATCH[1]}"
    local gke_location="${BASH_REMATCH[2]}"
    local gke_cluster="${BASH_REMATCH[3]}"
    local monitored_components
    monitored_components="$(gcloud container clusters describe "${gke_cluster}" \
      --project "${gke_project}" --location "${gke_location}" \
      --format='csv[no-heading](monitoringConfig.componentConfig.enableComponents)' \
      2>/dev/null || true)"
    if [[ "${monitored_components}" != *CADVISOR* || \
        "${monitored_components}" != *KUBELET* ]]; then
      log "warning: GKE cAdvisor/Kubelet managed metrics are not both enabled; Cloud Monitoring will lack durable container CPU/memory series"
      log "enable them with: gcloud container clusters update ${gke_cluster} --project=${gke_project} --location=${gke_location} --monitoring=SYSTEM,CADVISOR,KUBELET"
      if [[ "${GKE_MONITORING_MODE}" == "required" ]]; then
        return 1
      fi
    fi
  fi

  local collector_namespace="" epp_secret="" agentgateway_mode="none" vllm_port
  if kubectl get serviceaccount collector --namespace gmp-system >/dev/null 2>&1; then
    collector_namespace=gmp-system
  elif kubectl get serviceaccount collector --namespace gke-gmp-system >/dev/null 2>&1; then
    collector_namespace=gke-gmp-system
  fi
  if kubectl get secret inference-gateway-sa-metrics-reader-secret \
      --namespace "${SCENARIO_NAME}" >/dev/null 2>&1; then
    epp_secret=inference-gateway-sa-metrics-reader-secret
  fi
  if [[ "${BENCHMARK_TREATMENT}" != "service" && \
      "${BENCHMARK_GATEWAY_IMPLEMENTATION}" == "agentgateway" ]]; then
    agentgateway_mode="${BENCHMARK_ROUTER_MODE}"
  fi
  vllm_port="$(rendered_stack_value decode.vllm.port)"
  vllm_port="${vllm_port:-8200}"

  local -a render_args=(
    --output "${SPEC_DIR}/gke-podmonitoring.yaml"
    --namespace "${SCENARIO_NAME}"
    --interval "${METRICS_INTERVAL}s"
    --vllm-port "${vllm_port}"
    --agentgateway-mode "${agentgateway_mode}"
  )
  [[ -z "${epp_secret}" ]] || render_args+=(--epp-auth-secret "${epp_secret}")
  [[ -z "${collector_namespace}" ]] || render_args+=(--collector-namespace "${collector_namespace}")
  [[ "${BENCHMARK_TREATMENT}" == "service" ]] || render_args+=(--epp-enabled)
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" \
    "${AGTW_BENCHMARKING_DIR}/scripts/render-gke-podmonitoring.py" \
    "${render_args[@]}"
  kubectl apply -f "${SPEC_DIR}/gke-podmonitoring.yaml" >/dev/null
  log "GKE monitoring: applied PodMonitoring resources at ${METRICS_INTERVAL}s intervals"
}

# First standup on a fresh cluster can take a while waiting on the harness
# pod, since it's pulling a ~5.7GB image for the first time - the default
# 120s wait isn't enough. --data-access-timeout raises that
# (llm-d/llm-d-benchmark#1696). GPU modelservice startup has a separate wait:
# two vLLM replicas cold-reading Qwen3-32B from Basic HDD Filestore can take
# well over 25 minutes, so GPU acceleration defaults that step to two hours.
# The GPU scenario's startup probe uses the same allowance to avoid restarting
# vLLM while it is still loading safetensors or compiling CUDA graphs.
record_deployment_metadata() {
  kubectl create configmap agentgateway-benchmark-wrapper \
    --namespace "${SCENARIO_NAME}" \
    --from-literal=config-sha256="${CONFIG_SHA256}" \
    --from-literal=campaign-id="${BENCHMARK_CAMPAIGN_ID}" \
    --from-literal=treatment-id="${TREATMENT_ID}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
}

monitor_model_startup() {
  while true; do
    local pod message
    while IFS= read -r pod; do
      [[ -n "${pod}" ]] || continue
      message="$(kubectl logs --namespace "${SCENARIO_NAME}" "${pod}" \
        --tail=300 2>/dev/null | tr '\r' '\n' | \
        grep -E 'Loading safetensors checkpoint shards|Starting to load model|Model loading took|Application startup complete' | \
        tail -n 1 || true)"
      if [[ -n "${message}" ]]; then
        log "model startup ${pod#pod/}: ${message}"
      fi
    done < <(
      kubectl get pods --namespace "${SCENARIO_NAME}" -o name 2>/dev/null | \
        grep -- '-decode-' || true
    )
    sleep 60
  done
}

standup_treatment() {
  local -a provider_args=()
  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" ]]; then
    # GKE Managed Prometheus uses monitoring.googleapis.com PodMonitoring
    # resources, not Prometheus Operator's monitoring.coreos.com CRDs that
    # llm-d-benchmark validates and renders. Keep the incompatible upstream
    # integration disabled until llm-d-benchmark#1766 is fixed. The wrapper
    # separately runs upstream's direct collector and applies GKE-native
    # PodMonitoring resources, so this does not disable runtime evidence.
    provider_args+=(--no-monitoring)
  fi
  if [[ "${BENCHMARK_ACCELERATOR_TYPE}" == "gpu" ]]; then
    monitor_model_startup &
    MONITOR_PID=$!
  fi
  if ! llmdbench --spec "${SPEC_DIR}/spec.yaml" --workspace "$(workspace_dir)" \
      standup -p "${SCENARIO_NAME}" --skip-smoketest \
      --data-access-timeout "${DATA_ACCESS_TIMEOUT}" \
      --modelservice-deploy-timeout "${MODELSERVICE_DEPLOY_TIMEOUT}" \
      "${provider_args[@]}"; then
    if [[ -n "${MONITOR_PID}" ]]; then
      kill "${MONITOR_PID}" >/dev/null 2>&1 || true
      wait "${MONITOR_PID}" >/dev/null 2>&1 || true
      MONITOR_PID=""
    fi
    # Record the intended configuration even when the upstream readiness wait
    # times out. The pods may finish loading afterwards, and a hash-validated
    # resume can rerun the idempotent standup without discarding that work.
    if kubectl get namespace "${SCENARIO_NAME}" >/dev/null 2>&1; then
      record_deployment_metadata
    fi
    return 1
  fi
  if [[ -n "${MONITOR_PID}" ]]; then
    kill "${MONITOR_PID}" >/dev/null 2>&1 || true
    wait "${MONITOR_PID}" >/dev/null 2>&1 || true
    MONITOR_PID=""
  fi
  record_deployment_metadata
}

teardown_treatment() {
  local status=0
  llmdbench --spec "${SPEC_DIR}/spec.yaml" --workspace "$(workspace_dir)" \
    teardown -p "${SCENARIO_NAME}" || status=$?
  cleanup_treatment_storage || status=$?
  return "${status}"
}

cleanup_treatment_storage() {
  [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" ]] || return
  kubectl get namespace "${SCENARIO_NAME}" >/dev/null 2>&1 || return

  local owner campaign
  owner="$(kubectl get namespace "${SCENARIO_NAME}" \
    -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}')"
  campaign="$(kubectl get namespace "${SCENARIO_NAME}" \
    -o jsonpath='{.metadata.annotations.benchmark\.agentgateway\.dev/campaign-id}')"
  if [[ "${owner}" != "${BENCHMARK_MANAGED_BY}" || \
        "${campaign}" != "${BENCHMARK_CAMPAIGN_ID}" ]]; then
    log "refusing storage cleanup for unowned namespace ${SCENARIO_NAME}"
    return 1
  fi

  local -a pv_names=() filestore_instances=()
  local pvc pv claim_namespace claim_name driver handle location instance
  for pvc in model-pvc workload-pvc; do
    pv="$(kubectl get pvc "${pvc}" --namespace "${SCENARIO_NAME}" \
      -o jsonpath='{.spec.volumeName}' 2>/dev/null || true)"
    [[ -n "${pv}" ]] || continue
    claim_namespace="$(kubectl get pv "${pv}" \
      -o jsonpath='{.spec.claimRef.namespace}')"
    claim_name="$(kubectl get pv "${pv}" -o jsonpath='{.spec.claimRef.name}')"
    driver="$(kubectl get pv "${pv}" -o jsonpath='{.spec.csi.driver}')"
    if [[ "${claim_namespace}/${claim_name}" != "${SCENARIO_NAME}/${pvc}" ]]; then
      log "refusing to delete PV ${pv}: claim ownership does not match ${SCENARIO_NAME}/${pvc}"
      return 1
    fi
    case "${driver}" in
      filestore.csi.storage.gke.io)
        handle="$(kubectl get pv "${pv}" -o jsonpath='{.spec.csi.volumeHandle}')"
        if [[ "${handle}" =~ ^modeInstance/([^/]+)/([^/]+)/[^/]+$ ]]; then
          location="${BASH_REMATCH[1]}"
          instance="${BASH_REMATCH[2]}"
          filestore_instances+=("${location}/${instance}")
        else
          log "refusing unrecognized Filestore volume handle for ${pv}: ${handle}"
          return 1
        fi
        ;;
      pd.csi.storage.gke.io) ;;
      *)
        log "refusing to delete PV ${pv} with unexpected CSI driver ${driver}"
        return 1
        ;;
    esac
    pv_names+=("${pv}")
  done

  if (( ${#pv_names[@]} == 0 )); then
    return
  fi
  # llm-d-benchmark can leave its data-access pod behind after a failed run.
  # That pod mounts workload-pvc, so Kubernetes' pvc-protection finalizer
  # correctly prevents the PVC, PV, and backing Filestore instance from being
  # deleted until the consumer is gone.
  kubectl delete pod access-to-harness-data-workload-pvc \
    --namespace "${SCENARIO_NAME}" --ignore-not-found --wait=true >/dev/null
  log "storage cleanup: deleting PVCs in ${SCENARIO_NAME}"
  kubectl delete pvc model-pvc workload-pvc --namespace "${SCENARIO_NAME}" \
    --ignore-not-found --wait=false >/dev/null

  local deadline=$((SECONDS + GKE_CLEANUP_TIMEOUT))
  for pv in "${pv_names[@]}"; do
    local remaining=$((deadline - SECONDS))
    (( remaining > 0 )) || remaining=1
    kubectl wait --for=delete "pv/${pv}" --timeout="${remaining}s" >/dev/null || {
      log "timed out or failed waiting for benchmark PV ${pv} to be deleted"
      return 1
    }
  done

  local item describe_output describe_status
  for item in "${filestore_instances[@]}"; do
    location="${item%%/*}"
    instance="${item#*/}"
    while true; do
      describe_status=0
      describe_output="$(gcloud filestore instances describe "${instance}" \
        --project "${GKE_PROJECT}" --location "${location}" 2>&1)" || \
        describe_status=$?
      if (( describe_status != 0 )); then
        if grep -Eqi 'NOT_FOUND|not found' <<<"${describe_output}"; then
          break
        fi
        log "could not verify Filestore instance ${location}/${instance}: ${describe_output}"
        return 1
      fi
      if (( SECONDS >= deadline )); then
        log "timed out waiting for Filestore instance ${location}/${instance} to be deleted"
        return 1
      fi
      sleep 10
    done
  done
  log "storage cleanup: verified ${#pv_names[@]} PV(s) and ${#filestore_instances[@]} Filestore instance(s) deleted"
}

prepare_treatment() {
  if ! kubectl get namespace "${SCENARIO_NAME}" >/dev/null 2>&1; then
    standup_treatment
    return
  fi

  if [[ "${BENCHMARK_RESUME}" == "true" ]]; then
    local deployed_hash
    deployed_hash="$(kubectl get configmap agentgateway-benchmark-wrapper \
      --namespace "${SCENARIO_NAME}" \
      -o jsonpath='{.data.config-sha256}' 2>/dev/null || true)"
    if [[ "${deployed_hash}" != "${CONFIG_SHA256}" ]]; then
      log "refusing to resume ${SCENARIO_NAME}: deployed configuration hash does not match"
      return 1
    fi
    log "resume: continuing matching deployment ${SCENARIO_NAME}"
    standup_treatment
    return
  fi

  log "teardown: replacing previous deployment ${SCENARIO_NAME}"
  teardown_treatment
  standup_treatment
}

detect_hardware_model() {
  if [[ -n "${BENCHMARK_HARDWARE_MODEL:-}" ]]; then
    echo "${BENCHMARK_HARDWARE_MODEL}"
    return
  fi

  if [[ "${BENCHMARK_ACCELERATOR_TYPE}" == "gpu" ]]; then
    if [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" ]]; then
      echo "${GKE_ACCELERATOR}"
      return
    fi
    local gpu_product
    gpu_product="$(kubectl get nodes \
      -o jsonpath='{range .items[*]}{.metadata.labels.nvidia\.com/gpu\.product}{"\n"}{end}' | \
      awk 'NF' | sort -u | paste -sd+ -)"
    echo "${gpu_product:-nvidia-gpu}"
    return
  fi

  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "gke" ]]; then
    local -a selector_args=(-l '!cloud.google.com/gke-accelerator')
    if [[ -n "${BENCHMARK_GKE_CPU_NODEPOOL:-}" ]]; then
      selector_args=(-l "cloud.google.com/gke-nodepool=${BENCHMARK_GKE_CPU_NODEPOOL}")
    fi
    local instance_types
    instance_types="$(kubectl get nodes "${selector_args[@]}" \
      -o jsonpath='{range .items[*]}{.metadata.labels.node\.kubernetes\.io/instance-type}{"\n"}{end}' | \
      awk 'NF' | sort -u | paste -sd+ -)"
    echo "${instance_types:-gke-cpu}"
    return
  fi

  local architectures
  architectures="$(kubectl get nodes \
    -o jsonpath='{range .items[*]}{.status.nodeInfo.architecture}{"\n"}{end}' | \
    awk 'NF' | sort -u | paste -sd+ -)"
  echo "kind-${architectures:-cpu}"
}

# Return the stage index embedded in a Benchmark Report v0.2 document. Prism
# treats each stage report as one point in a load curve, so filenames must not
# collapse a multi-stage run back to a single result.
report_stage() {
  local source_report="$1"
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - "${source_report}" <<'PY'
import sys

import yaml

with open(sys.argv[1], encoding="utf-8") as stream:
    report = yaml.safe_load(stream)

stage = (
    report.get("scenario", {})
    .get("load", {})
    .get("standardized", {})
    .get("stage")
)
if not isinstance(stage, int) or isinstance(stage, bool) or stage < 0:
    raise ValueError(f"Benchmark report has invalid stage index: {stage!r}")
print(stage)
PY
}

prepare_guidellm_reports() {
  # This helper is invoked after every successful harness run. Skipping the
  # GuideLLM-only conversion must return success; a bare `return` here would
  # preserve the failed conditional's status and make an inference-perf run
  # fail after its results were already collected.
  [[ "${BENCHMARK_HARNESS}" == "guidellm" ]] || return 0
  local -a results=()
  while IFS= read -r result; do
    results+=("${result}")
  done < <(
    find "$(workspace_dir)" -type f -path '*/results/guidellm-*/results.json' \
      -print | sort
  )
  if (( ${#results[@]} != 1 )); then
    log "expected one GuideLLM results.json, found ${#results[@]}"
    return 1
  fi
  local source_dir template
  source_dir="$(dirname "${results[0]}")"
  template="${source_dir}/benchmark_report_v0.2,_results.json.yaml"
  [[ -f "${template}" ]] || {
    log "missing GuideLLM point-0 Benchmark Report: ${template}"
    return 1
  }
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" \
    "${AGTW_BENCHMARKING_DIR}/scripts/export-guidellm-reports.py" \
    --results "${results[0]}" --template "${template}" \
    --output-dir "${source_dir}"
}

# llm-d-benchmark writes reports below its per-invocation temporary workspace.
# Persist enriched copies in a self-contained Prism bundle instead of publishing
# paths back into that temporary tree. Prism reads the v0.2 standardized stack
# metadata for model, hardware, components, replicas, and parallelism.
write_prism_report() {
  local treatment="$1"
  local source_report="$2"
  local hardware_model="$3"
  local output_report="$4"

  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - \
    "${source_report}" "${SPEC_DIR}/scenario.yaml" "${output_report}" \
    "${treatment}" "${BENCHMARK_ACCELERATOR_TYPE}" "${BENCHMARK_CLUSTER_PROVIDER}" \
    "${hardware_model}" "${BENCHMARK_GATEWAY_IMPLEMENTATION}" \
    "${BENCHMARK_ROUTER_MODE}" "${BENCHMARK_BACKEND_TYPE}" \
    "${BENCHMARK_ROUTING_POLICY}" <<'PY'
import copy
import hashlib
import sys

import yaml

(
    source_path,
    scenario_path,
    output_path,
    treatment,
    mode,
    provider,
    hardware_model,
    gateway_implementation,
    router_mode,
    backend_type,
    routing_policy,
) = sys.argv[1:]

with open(source_path, encoding="utf-8") as stream:
    report = yaml.safe_load(stream)
with open(scenario_path, encoding="utf-8") as stream:
    rendered = yaml.safe_load(stream)["scenario"][0]


def merge(left, right):
    result = copy.deepcopy(left)
    for key, value in right.items():
        if isinstance(value, dict) and isinstance(result.get(key), dict):
            result[key] = merge(result[key], value)
        else:
            result[key] = copy.deepcopy(value)
    return result


scenario = merge(rendered.get("common", {}), rendered.get("modelservice", {}))

decode = scenario["decode"]
parallelism = decode.get("parallelism", {})
replicas = int(decode.get("replicas", 1))
tp = int(parallelism.get("tensor", 1))
dp = int(parallelism.get("data", 1))
dp_local = int(parallelism.get("dataLocal", 1))
workers = int(parallelism.get("workers", 1))
accelerator_count = replicas * tp * dp_local if mode == "gpu" else 0

model_name = scenario["model"]["name"]
image = scenario.get("images", {}).get("vllm", {})
repository = image.get("repository", "")
tag = image.get("tag", "")
tool = "llm-d-inference-sim" if backend_type == "inference-sim" else backend_type
tool_version = f"{repository}:{tag}" if repository and tag else repository

run = report.setdefault("run", {})
run["description"] = f"{treatment}-{mode}-{provider}-{routing_policy}"

load_metadata = report.setdefault("scenario", {}).setdefault("load", {}).setdefault(
    "metadata", {}
)
load_metadata.setdefault(
    "description", f"{treatment} routing-policy={routing_policy} campaign treatment"
)

# Temporary downstream compatibility while llm-d-benchmark#1744 and
# llm-d-prism#120 are unresolved. The inference-perf BR v0.2 converter omits
# input_token_rate, while Prism currently drops total_token_rate. Keep both
# canonical fields in the wrapper output so Input Tok/s works with current
# Prism and Total Tok/s starts working when Prism consumes the existing field.
throughput = (
    report.setdefault("results", {})
    .setdefault("request_performance", {})
    .setdefault("aggregate", {})
    .setdefault("throughput", {})
)


def metric_mean(metric):
    if not isinstance(metric, dict):
        return None
    value = metric.get("mean")
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def derived_rate(mean, *references):
    units = next(
        (
            metric.get("units")
            for metric in references
            if isinstance(metric, dict) and metric.get("units")
        ),
        "tokens/s",
    )
    return {"units": units, "mean": mean}


input_rate = throughput.get("input_token_rate")
output_rate = throughput.get("output_token_rate")
total_rate = throughput.get("total_token_rate")
input_mean = metric_mean(input_rate)
output_mean = metric_mean(output_rate)
total_mean = metric_mean(total_rate)

if input_mean is None and output_mean is not None and total_mean is not None:
    input_mean = total_mean - output_mean
    throughput["input_token_rate"] = derived_rate(
        input_mean, total_rate, output_rate
    )
if total_mean is None and input_mean is not None and output_mean is not None:
    throughput["total_token_rate"] = derived_rate(
        input_mean + output_mean, input_rate, output_rate
    )

stack = report["scenario"].setdefault("stack", [])
for component in stack:
    standardized = component.get("standardized", {})
    if standardized.get("kind") != "inference_engine":
        continue
    if standardized.get("role") not in ("decode", "replica", "aggregate"):
        continue
    standardized["role"] = "decode"
    standardized["replicas"] = replicas
    standardized["tool"] = tool
    standardized["tool_version"] = tool_version
    standardized.setdefault("model", {})["name"] = model_name
    accelerator = standardized.setdefault("accelerator", {})
    accelerator["model"] = hardware_model
    accelerator["count"] = accelerator_count
    accelerator["parallelism"] = {
        "tp": tp,
        "dp": dp,
        "dp_local": dp_local,
        "workers": workers,
        "ep": 1,
        "pp": 1,
    }

if treatment != "service":
    proxy = scenario.get("router", {}).get("proxy", {})
    proxy_image = proxy.get("presets", {}).get(gateway_implementation, {}).get(
        "image", ""
    )
    has_gateway_component = any(
        "gateway"
        in " ".join(
            (
                str(component.get("metadata", {}).get("label", "")),
                str(component.get("standardized", {}).get("kind", "")),
                str(component.get("standardized", {}).get("tool", "")),
            )
        ).lower()
        for component in stack
    )
    if not has_gateway_component:
        native_config = copy.deepcopy(proxy)
        cfg_id = hashlib.sha256(
            yaml.safe_dump(native_config, sort_keys=True).encode("utf-8")
        ).hexdigest()
        stack.append(
            {
                "metadata": {
                    "schema_version": "0.0.1",
                    "label": f"{gateway_implementation}-{router_mode}",
                    "cfg_id": cfg_id,
                    "description": f"{gateway_implementation} inference gateway ({router_mode})",
                },
                "standardized": {
                    "kind": "generic",
                    "tool": gateway_implementation,
                    "tool_version": proxy_image,
                },
                "native": {"config": native_config},
            }
        )

with open(output_path, "w", encoding="utf-8") as stream:
    yaml.safe_dump(report, stream, sort_keys=False)
PY
}

# Prism treats manifests/configuration and evidence as attachments selected in
# its validation wizard; it does not discover them from a Benchmark Report v0.2
# document. Keep each treatment's supporting files together so the complete run
# can be uploaded without returning to llm-d-benchmark's temporary workspace.
write_prism_supporting_artifacts() {
  local treatment="$1"
  local source_report="$2"
  local artifact_dir="$3"
  local source_dir
  source_dir="$(dirname "${source_report}")"
  mkdir -p "${artifact_dir}"

  local source_file
  for source_file in \
    "${SPEC_DIR}/scenario.yaml" \
    "${source_dir}/config.yaml" \
    "${source_dir}/run_metadata.yaml" \
    "${source_dir}/stdout.log" \
    "${source_dir}/stderr.log"; do
    if [[ ! -f "${source_file}" ]]; then
      log "missing Prism supporting artifact for ${treatment}: ${source_file}"
      return 1
    fi
  done

  cp "${SPEC_DIR}/scenario.yaml" "${artifact_dir}/benchmark-scenario.yaml"
  cp "${source_dir}/config.yaml" "${artifact_dir}/config.yaml"
  cp "${source_dir}/run_metadata.yaml" "${artifact_dir}/run_metadata.yaml"
  cp "${source_dir}/stdout.log" "${artifact_dir}/${BENCHMARK_HARNESS}-stdout.log"
  cp "${source_dir}/stderr.log" "${artifact_dir}/${BENCHMARK_HARNESS}-stderr.log"
  if [[ "${BENCHMARK_HARNESS}" == "inference-perf" ]]; then
    [[ -f "${source_dir}/summary_lifecycle_metrics.json" ]] || {
      log "missing inference-perf summary: ${source_dir}/summary_lifecycle_metrics.json"
      return 1
    }
    cp "${source_dir}/summary_lifecycle_metrics.json" \
      "${artifact_dir}/summary_lifecycle_metrics.json"
    if [[ "${BENCHMARK_WORKLOAD_VARIANT}" == "upstream" ]]; then
      local per_request_source="${source_dir}/per_request_lifecycle_metrics.json"
      local per_request_archive="${artifact_dir}/per_request_lifecycle_metrics.json.gz"
      [[ -s "${per_request_source}" ]] || {
        log "missing or empty upstream per-request evidence: ${per_request_source}"
        return 1
      }
      # The upstream Qwen3-32B sweep can produce a 15+ GiB JSON document.
      # Preserve every request losslessly, but compress it so three-treatment
      # campaigns and their CI artifacts do not require tens of GiB each.
      gzip -c "${per_request_source}" > "${per_request_archive}"
      gzip -t "${per_request_archive}"
    fi
  else
    [[ -f "${source_dir}/results.json" ]] || {
      log "missing GuideLLM result: ${source_dir}/results.json"
      return 1
    }
    cp "${source_dir}/results.json" "${artifact_dir}/guidellm-results.json"
    [[ ! -f "${source_dir}/summary.txt" ]] || \
      cp "${source_dir}/summary.txt" "${artifact_dir}/guidellm-summary.txt"
  fi
  if [[ -f "${SPEC_DIR}/standalone-invariant.yaml" ]]; then
    cp "${SPEC_DIR}/standalone-invariant.yaml" \
      "${artifact_dir}/standalone-comparison-invariant.yaml"
  fi
  if [[ -n "${RUNTIME_METRICS_CAPTURE_DIR}" && \
      -d "${RUNTIME_METRICS_CAPTURE_DIR}" ]]; then
    cp -R "${RUNTIME_METRICS_CAPTURE_DIR}" "${artifact_dir}/runtime-metrics"
  fi
  local monitoring_artifact
  for monitoring_artifact in \
    "${SPEC_DIR}/gke-podmonitoring.yaml" \
    "${SPEC_DIR}/gke-podmonitoring-status.yaml" \
    "${GPU_RELEASE_EVIDENCE}"; do
    [[ ! -f "${monitoring_artifact}" ]] || \
      cp "${monitoring_artifact}" "${artifact_dir}/$(basename "${monitoring_artifact}")"
  done
}

cleanup_completed_work_dir() {
  local expected="${RESULTS_DIR}/.work/${TREATMENT_ID}/repetition-${BENCHMARK_REPETITION}"
  [[ "${SPEC_DIR}" == "${expected}" && -d "${SPEC_DIR}" && ! -L "${SPEC_DIR}" ]] || {
    log "refusing to delete unexpected completed work directory: ${SPEC_DIR}"
    return 1
  }
  # Native evidence needed by the published result has already been copied
  # above. Remove only this completed repetition's transient workspace; failed
  # repetitions remain available for diagnosis.
  find "${SPEC_DIR}" -xdev -depth -delete
  [[ ! -e "${SPEC_DIR}" ]]
}

publish_artifact_dir() {
  local staging="$1"
  local destination="$2"
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - \
    "${staging}" "${destination}" <<'PY'
import os
import shutil
import sys
from pathlib import Path

staging = Path(sys.argv[1])
destination = Path(sys.argv[2])
root = destination.resolve().parent
if staging.is_symlink() or not staging.is_dir():
    raise ValueError(f"invalid staging artifact directory: {staging}")
if staging.resolve().parent != root or destination.resolve().parent != root:
    raise ValueError("artifact paths must be direct children of the treatment directory")
if destination.is_symlink() or (destination.exists() and not destination.is_dir()):
    raise ValueError(f"invalid existing artifact path: {destination}")

backup = root / f".{destination.name}.backup"
if backup.exists() or backup.is_symlink():
    raise ValueError(f"refusing to overwrite stale artifact backup: {backup}")
if destination.exists():
    os.rename(destination, backup)
try:
    os.rename(staging, destination)
except BaseException:
    if backup.exists() and not destination.exists():
        os.rename(backup, destination)
    raise
if backup.exists():
    shutil.rmtree(backup)
PY
}

write_campaign_manifest() {
  local operation="${1:-write}"
  local metadata="${RESULTS_DIR}/campaign-manifest.yaml"
  "${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - \
    "${operation}" "${metadata}" "${BENCHMARK_CAMPAIGN_ID}" "${BENCHMARK_SUITE}" \
    "${BENCHMARK_SCENARIO}" \
    "${BENCHMARK_CLUSTER_PROVIDER}" "${BENCHMARK_ACCELERATOR_TYPE}" \
    "${BENCHMARK_ACCELERATOR_MODEL}" "${BENCHMARK_BACKEND_TYPE}" "${MODEL}" \
    "${REPLICAS}" "${TENSOR_PARALLELISM}" \
    "${BENCHMARK_ROUTING_POLICY}" "${BENCHMARK_WORKLOAD_VARIANT}" \
    "${BENCHMARK_HARNESS}" \
    "${WORKLOAD}" "${WORKLOAD_SHA256}" "${SCENARIO_SOURCE_SHA256}" \
    "${SHARED_CONFIG_SHA256}" \
    "${TREATMENT_ID}" "${BENCHMARK_REPETITION}" "${CONFIG_SHA256}" \
    "${MODEL_STORAGE_PROFILE}" "${MODEL_STORAGE_STRATEGY}" \
    "${MODEL_STORAGE_CLASS}" "${MODEL_STORAGE_SIZE}" \
    "${WORKLOAD_STORAGE_PROFILE}" "${WORKLOAD_STORAGE_CLASS}" \
    "${WORKLOAD_STORAGE_SIZE}" \
    "${BENCHMARK_ROUTER_CHART_VERSION}" "${ROUTER_CHART_DIGEST}" \
    "${STANDALONE_INVARIANT_SHA256}" "${RUNTIME_METRICS_ENABLED}" \
    "${METRICS_INTERVAL}" "${GKE_MONITORING_MODE}" "${FAST_COLLECT}" \
    "${GPU_RELEASE_POLICY}" "${GKE_GPU_NODEPOOL}" "${AGW_IMAGE}" \
    "${LLM_D_BENCHMARK_REF}" "${BENCHMARK_REFERENCE_PROFILE}" \
    "${BENCHMARK_ENDPOINT_PATH}" "${AGW_VERSION}" <<'PY'
import os
import sys
from pathlib import Path

import yaml

(
    operation,
    metadata_path,
    campaign_id,
    suite,
    scenario,
    provider,
    accelerator_type,
    accelerator_model,
    backend_type,
    model,
    replicas,
    tensor_parallelism,
    routing_policy,
    workload_variant,
    harness,
    workload,
    workload_sha256,
    scenario_source_sha256,
    shared_config_sha256,
    treatment,
    repetition,
    config_hash,
    model_storage_profile,
    model_storage_strategy,
    model_storage_class,
    model_storage_size,
    workload_storage_profile,
    workload_storage_class,
    workload_storage_size,
    router_chart_version,
    router_chart_digest,
    standalone_invariant_sha256,
    runtime_metrics_enabled,
    metrics_interval,
    gke_monitoring_mode,
    fast_collect,
    gpu_release_policy,
    gke_gpu_nodepool,
    gateway_image,
    llm_d_benchmark_ref,
    reference_profile,
    endpoint_path,
    agentgateway_version,
) = sys.argv[1:]

path = Path(metadata_path)
identity = {
    "scenario": scenario,
    "cluster_provider": provider,
    "accelerator_type": accelerator_type,
    "accelerator_model": accelerator_model,
    "backend_type": backend_type,
    "model": model,
    "replicas": int(replicas),
    "tensor_parallelism": int(tensor_parallelism),
    "routing_policy": routing_policy,
    "workload_variant": workload_variant,
    "harness": harness,
    "workload": workload,
    "workload_sha256": workload_sha256,
    "scenario_source_sha256": scenario_source_sha256,
    "shared_config_sha256": shared_config_sha256,
    "llm_d_benchmark_ref": llm_d_benchmark_ref,
    "reference_profile": reference_profile or "none",
    "endpoint_path": endpoint_path,
    "storage": {
        "model": {
            "profile": model_storage_profile,
            "strategy": model_storage_strategy,
            "class": model_storage_class or "adapter-default",
            "size": model_storage_size or "adapter-default",
        },
        "workload": {
            "profile": workload_storage_profile,
            "class": workload_storage_class or "adapter-default",
            "size": workload_storage_size or "adapter-default",
        },
    },
    "runtime_metrics": {
        "enabled": runtime_metrics_enabled == "true",
        "interval_seconds": int(metrics_interval),
        "gke_monitoring": gke_monitoring_mode,
    },
    "result_collection": {"fast_collect": fast_collect == "true"},
    "gpu_release": {
        "policy": gpu_release_policy,
        "node_pool": gke_gpu_nodepool if provider == "gke" else "not-applicable",
    },
}
document = {
    "schema_version": 1,
    "campaign_id": campaign_id,
    "suite": suite,
    "identity": identity,
    "treatments": {},
}
if path.exists():
    with path.open(encoding="utf-8") as stream:
        document = yaml.safe_load(stream) or {}
    if (
        document.get("campaign_id") != campaign_id
        or document.get("suite") != suite
        or document.get("identity") != identity
    ):
        raise ValueError(
            f"{path} belongs to an incompatible campaign; choose a new "
            "BENCHMARK_CAMPAIGN_ID"
        )

if operation == "validate":
    raise SystemExit(0)

treatment_entry = document.setdefault("treatments", {}).setdefault(
    treatment, {"repetitions": {}}
)
treatment_entry.setdefault("repetitions", {})[str(repetition)] = {
    "config_sha256": config_hash,
    "gateway": {
        "implementation": (
            "none" if treatment == "service" else treatment.rsplit("-", 1)[0]
        ),
        "mode": "none" if treatment == "service" else treatment.rsplit("-", 1)[1],
        "image": "not-applicable" if treatment == "service" else gateway_image,
        "agentgateway_version": (
            agentgateway_version if treatment.startswith("agentgateway-")
            else "not-applicable"
        ),
    },
    "storage": {
        "model": {
            "profile": model_storage_profile,
            "strategy": model_storage_strategy,
            "class": model_storage_class or "adapter-default",
            "size": model_storage_size or "adapter-default",
        },
        "workload": {
            "profile": workload_storage_profile,
            "class": workload_storage_class or "adapter-default",
            "size": workload_storage_size or "adapter-default",
        },
    },
    "router_chart_version": router_chart_version,
    "router_chart_digest": router_chart_digest,
    "standalone_invariant_sha256": standalone_invariant_sha256,
    "runtime_metrics": {
        "enabled": runtime_metrics_enabled == "true",
        "interval_seconds": int(metrics_interval),
        "gke_monitoring": gke_monitoring_mode,
    },
    "result_collection": {"fast_collect": fast_collect == "true"},
    "gpu_release": {
        "policy": gpu_release_policy,
        "node_pool": gke_gpu_nodepool if provider == "gke" else "not-applicable",
    },
}
temporary = path.with_suffix(path.suffix + ".tmp")
with temporary.open("w", encoding="utf-8") as stream:
    yaml.safe_dump(document, stream, sort_keys=False)
os.replace(temporary, path)
PY
}

# Removes the managed llm-d-benchmark clone (repo + venv + installed CLI/
# planner) from LLM_D_BENCHMARK_CACHE_DIR. Does nothing if LLM_D_BENCHMARK_DIR
# was set (that clone isn't ours to delete) or if there's no managed clone
# to remove. Doesn't touch the kind cluster or anything deployed to it -
# that's a separate concern, use kind-delete/teardown for that.
clean() {
  if [[ -n "${LLM_D_BENCHMARK_DIR:-}" ]]; then
    log "LLM_D_BENCHMARK_DIR is set, nothing managed by this script to clean up"
    return 0
  fi
  local target="${LLM_D_BENCHMARK_CACHE_DIR:?}"
  if [[ ! -e "${target}" ]]; then
    log "no managed clone at ${LLM_D_BENCHMARK_CACHE_DIR}, nothing to clean up"
    return 0
  fi
  if [[ -L "${target}" || ! -d "${target}" ]]; then
    log "refusing to remove a symlink or non-directory cache target: ${target}"
    return 1
  fi
  local resolved parent
  resolved="$(realpath "${target}")"
  parent="$(dirname "${resolved}")"
  if [[ "${resolved}" == "/" || "${resolved}" == "${HOME}" || \
      "$(basename "${resolved}")" != "llm-d-benchmark" || \
      ! -d "${resolved}/.git" || ! -f "${resolved}/install.sh" ]]; then
    log "refusing unexpected managed cache target: ${resolved}"
    return 1
  fi
  if [[ "$(basename "${parent}")" != "agentgateway-benchmark" && \
      ! -f "${resolved}/.agentgateway-benchmark-managed" ]]; then
    log "refusing cache without the wrapper ownership marker: ${resolved}"
    return 1
  fi
  log "removing managed checkout and venv at ${resolved}"
  rm -rf -- "${resolved}"
}

main() {
  parse_args "$@"
  validate_configuration

  mkdir -p "${RESULTS_DIR}" "${SPEC_DIR}"
  configure_provider
  resolve_gke_identity
  acquire_campaign_lock
  load_hf_token_from_cluster
  ensure_llm_d_benchmark
  render_bundled_upstream_workload
  resolve_campaign_input_hashes
  resolve_router_chart_digest
  if [[ "${BENCHMARK_CLUSTER_PROVIDER}" == "kind" ]]; then
    command -v skopeo >/dev/null || {
      log "skopeo is required for the kind provider"
      return 1
    }
    # Always import the exact image archive for kind, even if the node happens
    # to contain an older image with the same mutable tag.
    load_agentgateway_image
  fi

  render_scenario
  SCENARIO_NAME="$("${LLM_D_BENCHMARK_DIR}/.venv/bin/python" - "${SPEC_DIR}/scenario.yaml" <<'PY'
import sys
import yaml
with open(sys.argv[1], encoding="utf-8") as stream:
    print(yaml.safe_load(stream)["scenario"][0]["name"])
PY
)"
  write_spec
  write_campaign_manifest validate
  ensure_benchmark_namespace

  log "running ${TREATMENT_ID}: ${BENCHMARK_ACCELERATOR_TYPE}/${BENCHMARK_BACKEND_TYPE} on ${BENCHMARK_CLUSTER_PROVIDER}"
  log "standup: ${TREATMENT_ID} (${SCENARIO_NAME})"
  TREATMENT_ACTIVE=true
  ensure_agentgateway_gateway_controller
  prepare_treatment
  configure_agentgateway_gateway_image
  configure_gke_monitoring
  log "smoketest: ${TREATMENT_ID} (${SCENARIO_NAME})"
  llmdbench --spec "${SPEC_DIR}/spec.yaml" --workspace "$(workspace_dir)" \
    smoketest -p "${SCENARIO_NAME}"
  verify_reference_runtime
  verify_gke_monitoring
  start_runtime_metrics
  log "run: ${TREATMENT_ID} (${SCENARIO_NAME})"
  local -a run_args=(
    --spec "${SPEC_DIR}/spec.yaml"
    --workspace "$(workspace_dir)"
    run -p "${SCENARIO_NAME}" -l "${BENCHMARK_HARNESS}" -w "${WORKLOAD}"
  )
  if [[ "${BENCHMARK_ENDPOINT_PATH}" == "internal" ]]; then
    local internal_endpoint
    internal_endpoint="$(resolve_internal_endpoint)"
    run_args+=(--endpoint-url "${internal_endpoint}")
    log "run endpoint: ${internal_endpoint} (Service ClusterIP)"
  fi
  if [[ "${FAST_COLLECT}" == "true" ]]; then
    # Available on llm-d-benchmark main. It transfers the same files through a
    # compressed exec stream instead of the very slow API-server cp path.
    run_args+=(--fast-collect)
  fi
  if [[ -n "${WORKLOAD_FILE_PATH}" ]]; then
    run_args+=(--workload-file-path "${WORKLOAD_FILE_PATH}")
  fi
  local run_status=0 metrics_status=0 release_status=0 run_pid
  if [[ "${GPU_RELEASE_POLICY}" == "after-load" ]]; then
    local final_stage_id
    final_stage_id="$(expected_final_stage_id)"
    [[ "${final_stage_id}" =~ ^[0-9]+$ ]] || {
      log "after-load GPU release requires an inference-perf profile with explicit stages"
      return 1
    }
  fi

  llmdbench "${run_args[@]}" &
  run_pid=$!
  if [[ "${GPU_RELEASE_POLICY}" == "after-load" ]]; then
    if wait_for_traffic_complete "${run_pid}"; then
      request_runtime_metrics_stop
      release_gke_gpu_pool || release_status=$?
      # The collector waits only a bounded time for its copy acknowledgement.
      # Drain it now, while inference-perf performs CPU-only report generation,
      # instead of waiting for the harness process to return 30+ minutes later.
      stop_runtime_metrics || metrics_status=$?
    else
      release_status=$?
      log "GPU release: traffic-complete event was not observed"
    fi
  fi
  wait "${run_pid}" || run_status=$?
  if [[ "${GPU_RELEASE_POLICY}" == "after-run" && ${run_status} -eq 0 ]]; then
    TRAFFIC_COMPLETE_SOURCE=run-returned
    request_runtime_metrics_stop
    release_gke_gpu_pool || release_status=$?
  fi
  stop_runtime_metrics || metrics_status=$?
  if (( run_status != 0 )); then
    return "${run_status}"
  fi
  if (( release_status != 0 )); then
    log "GPU release failed"
    return "${release_status}"
  fi
  if (( metrics_status != 0 )); then
    log "runtime metrics collection failed"
    return "${metrics_status}"
  fi
  prepare_guidellm_reports

  log "writing standardized benchmark reports and native evidence"
  local hardware_model
  hardware_model="$(detect_hardware_model)"
  local treatment_dir="${RESULTS_DIR}/runs/${TREATMENT_ID}"
  local artifact_dir="${treatment_dir}/repetition-${BENCHMARK_REPETITION}"
  local staging_artifact_dir="${treatment_dir}/.staging-repetition-${BENCHMARK_REPETITION}-${RUN_ID}"
  local first_report=""
  local report_count=0
  if [[ -e "${staging_artifact_dir}" || -L "${staging_artifact_dir}" ]]; then
    log "refusing to overwrite staging artifact path: ${staging_artifact_dir}"
    return 1
  fi
  mkdir -p "${treatment_dir}" "${staging_artifact_dir}"

  local report
  while IFS= read -r report; do
    local report_name
    report_name="$(basename "${report}")"
    local point output_report
    if [[ "${BENCHMARK_HARNESS}" == "inference-perf" ]]; then
      [[ "${report_name}" =~ ^benchmark_report_v0\.2,_stage_[0-9]+_lifecycle_metrics\.json\.yaml$ ]] || continue
      point="$(report_stage "${report}")"
      output_report="${staging_artifact_dir}/benchmark_report_v0.2,_${TREATMENT_ID}_stage_${point}_lifecycle_metrics.json.yaml"
    else
      [[ "${report_name}" =~ ^benchmark_report_v0\.2,_guidellm_run_([0-9]+)\.yaml$ ]] || continue
      point="${BASH_REMATCH[1]}"
      output_report="${staging_artifact_dir}/benchmark_report_v0.2,_${TREATMENT_ID}_guidellm_run_${point}.yaml"
    fi
    if [[ -e "${output_report}" ]]; then
      log "multiple ${TREATMENT_ID} reports claim ${BENCHMARK_HARNESS} point ${point}"
      return 1
    fi
    write_prism_report "${TREATMENT_ID}" "${report}" "${hardware_model}" "${output_report}"
    [[ -n "${first_report}" ]] || first_report="${report}"
    report_count=$((report_count + 1))
  done < <(
    find "$(workspace_dir)" -type f -name 'benchmark_report_v0.2,*.yaml' \
      -print | sort
  )

  if (( report_count == 0 )); then
    log "couldn't find a benchmark report under $(workspace_dir) for ${TREATMENT_ID}"
    return 1
  fi
  write_prism_supporting_artifacts "${TREATMENT_ID}" "${first_report}" "${staging_artifact_dir}"
  publish_artifact_dir "${staging_artifact_dir}" "${artifact_dir}"
  log "wrote ${report_count} ${TREATMENT_ID} ${BENCHMARK_HARNESS} report(s) to ${artifact_dir}"
  write_campaign_manifest write

  if [[ "${MODEL_CACHE_POLICY}" == "ephemeral" ]]; then
    log "teardown: ${TREATMENT_ID} (${SCENARIO_NAME})"
    teardown_treatment
    TREATMENT_ACTIVE=false
    cleanup_completed_work_dir
  else
    log "retaining ${SCENARIO_NAME}; rerun with BENCHMARK_RESUME=true to reuse it"
  fi

  log "done, results in ${RESULTS_DIR}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
