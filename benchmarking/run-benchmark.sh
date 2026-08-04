#!/usr/bin/env bash
#
# Runs the agentgateway-vs-plain-Service benchmark comparison end to end:
# standup both arms, smoketest, run the workload, then compare results with
# llm-d-benchmark's own cross_treatment.py. See README.md for the manual
# version of these same steps.
#
# Set LLM_D_BENCHMARK_DIR to your llm-d-benchmark clone before running this
# (CLI installed in .venv, see their quickstart). Needs skopeo too
# (brew install skopeo) - see the image-loading gotcha below.
# Optional: CLUSTER_NAME (default: kind), AGTW_BENCHMARKING_DIR (default: here)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

: "${LLM_D_BENCHMARK_DIR:?Set LLM_D_BENCHMARK_DIR to your llm-d-benchmark clone}"
CLUSTER_NAME="${CLUSTER_NAME:-kind}"
AGTW_BENCHMARKING_DIR="${AGTW_BENCHMARKING_DIR:-$SCRIPT_DIR}"

AGW_IMAGE="cr.agentgateway.dev/agentgateway:latest-dev"
WORKLOAD="sanity_random.yaml"
SPEC_DIR="$(mktemp -d)"
RESULTS_DIR="${AGTW_BENCHMARKING_DIR}/results"
STANDUP_RETRIES=2
STANDUP_RETRY_WAIT=600

declare -A ARM_SCENARIO=(
  [baseline]=plain-service-decode-only
  [agentgateway]=agentgateway-decode-only
)

log() { echo "[run-benchmark] $*"; }

# The router chart's agentgateway image preset points at a tag that doesn't
# exist upstream (see benchmarking/README.md), so this needs to be loaded
# into kind by hand before the agentgateway arm can come up.
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
  tar="$(mktemp -u).tar"
  skopeo copy --override-os linux --override-arch "${platform}" \
    "docker://${AGW_IMAGE}" "docker-archive:${tar}:${AGW_IMAGE}"
  kind load image-archive "${tar}" --name "${CLUSTER_NAME}"
  rm -f "${tar}"
}

write_spec() {
  local arm="$1"
  cat > "${SPEC_DIR}/spec-${arm}.yaml" <<EOF
base_dir: ${LLM_D_BENCHMARK_DIR}
values_file:
  path: ${LLM_D_BENCHMARK_DIR}/config/templates/values/defaults.yaml
template_dir:
  path: ${LLM_D_BENCHMARK_DIR}/config/templates/jinja
scenario_file:
  path: ${AGTW_BENCHMARKING_DIR}/scenarios/${arm}.yaml
EOF
}

# Pin each arm to its own workspace root so we know where to find its
# results afterwards, instead of the default (a fresh random temp dir
# printed to stdout on every invocation).
workspace_dir() {
  echo "${SPEC_DIR}/workspace-$1"
}

llmdbench() {
  (cd "${LLM_D_BENCHMARK_DIR}" && source .venv/bin/activate && llmdbenchmark "$@")
}

# First standup on a fresh cluster times out waiting on the harness pod since
# it's pulling a ~5.7GB image for the first time. Retry a couple times,
# waiting for the namespace's pods to go Ready in between.
standup_arm() {
  local arm="$1" scenario="${ARM_SCENARIO[$arm]}"
  local attempt=1
  while true; do
    if llmdbench --spec "${SPEC_DIR}/spec-${arm}.yaml" --workspace "$(workspace_dir "${arm}")" \
        standup -p "${scenario}" --skip-smoketest; then
      return 0
    fi
    if [[ "${attempt}" -ge "${STANDUP_RETRIES}" ]]; then
      log "standup for ${arm} failed after ${attempt} attempts"
      return 1
    fi
    log "standup for ${arm} timed out (likely still pulling images), waiting for pods then retrying"
    kubectl wait --for=condition=Ready pod --all -n "${scenario}" --timeout="${STANDUP_RETRY_WAIT}s" || true
    attempt=$((attempt + 1))
  done
}

main() {
  mkdir -p "${RESULTS_DIR}"
  load_agentgateway_image

  for arm in baseline agentgateway; do
    write_spec "${arm}"
  done

  for arm in baseline agentgateway; do
    scenario="${ARM_SCENARIO[$arm]}"
    log "standup: ${arm} (${scenario})"
    standup_arm "${arm}"
    log "smoketest: ${arm} (${scenario})"
    llmdbench --spec "${SPEC_DIR}/spec-${arm}.yaml" --workspace "$(workspace_dir "${arm}")" \
      smoketest -p "${scenario}"
    log "run: ${arm} (${scenario})"
    llmdbench --spec "${SPEC_DIR}/spec-${arm}.yaml" --workspace "$(workspace_dir "${arm}")" \
      run -p "${scenario}" -l inference-perf -w "${WORKLOAD}"
  done

  log "comparing results with cross_treatment.py"
  local comparison_input
  comparison_input="$(mktemp -d)"
  for arm in baseline agentgateway; do
    # cross_treatment.py wants one subdir per arm, each containing a
    # benchmark_report_v0.2*.yaml - find the one `run` just produced.
    local report treatment_dir
    report="$(find "$(workspace_dir "${arm}")" -name 'benchmark_report_v0.2*.yaml' | sort | tail -1)"
    if [[ -z "${report}" ]]; then
      log "couldn't find a benchmark report under $(workspace_dir "${arm}") for ${arm}, aborting comparison"
      exit 1
    fi
    treatment_dir="$(dirname "${report}")"
    ln -sf "${treatment_dir}" "${comparison_input}/${arm}"
  done

  (cd "${LLM_D_BENCHMARK_DIR}" && source .venv/bin/activate && python3 -c "
from pathlib import Path
from llmdbenchmark.analysis.cross_treatment import generate_cross_treatment_summary
generate_cross_treatment_summary(Path('${comparison_input}'), output_dir=Path('${RESULTS_DIR}'))
")

  log "done, results in ${RESULTS_DIR}"
}

main "$@"
