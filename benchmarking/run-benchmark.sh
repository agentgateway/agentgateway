#!/usr/bin/env bash
#
# Runs the agentgateway-vs-plain-Service benchmark comparison end to end:
# standup both arms, smoketest, run the workload, then compare results with
# llm-d-benchmark's own cross_treatment.py. See README.md for the manual
# version of these same steps.
#
# Clones and manages its own llm-d-benchmark checkout - no setup needed
# beyond skopeo (brew install skopeo, see the image-loading gotcha below).
# Needs llm-d-benchmark#1696 or later for --data-access-timeout on standup,
# which is why LLM_D_BENCHMARK_REF defaults to main instead of a tag.
#
# Optional: CLUSTER_NAME (default: kind), AGTW_BENCHMARKING_DIR (default: here),
# LLM_D_BENCHMARK_REF (default: main - branch, tag, or commit),
# LLM_D_BENCHMARK_DIR (skip the managed clone entirely and use this checkout
# instead - your responsibility to keep it up to date then),
# LLM_D_BENCHMARK_CACHE_DIR (where the managed clone lives, default: see below)
#
# Usage: ./run-benchmark.sh          run the comparison
#        ./run-benchmark.sh --clean  remove the managed llm-d-benchmark clone

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CLUSTER_NAME="${CLUSTER_NAME:-kind}"
AGTW_BENCHMARKING_DIR="${AGTW_BENCHMARKING_DIR:-$SCRIPT_DIR}"
LLM_D_BENCHMARK_REF="${LLM_D_BENCHMARK_REF:-main}"
LLM_D_BENCHMARK_CACHE_DIR="${LLM_D_BENCHMARK_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/agentgateway-benchmark/llm-d-benchmark}"

AGW_IMAGE="cr.agentgateway.dev/agentgateway:latest-dev"
WORKLOAD="sanity_random.yaml"
SPEC_DIR="$(mktemp -d)"
RESULTS_DIR="${AGTW_BENCHMARKING_DIR}/results"
DATA_ACCESS_TIMEOUT=600

declare -A ARM_SCENARIO=(
  [baseline]=plain-service-decode-only
  [agentgateway]=agentgateway-decode-only
)

log() { echo "[run-benchmark] $*"; }

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
    return 0
  fi

  LLM_D_BENCHMARK_DIR="${LLM_D_BENCHMARK_CACHE_DIR}"

  if [[ ! -d "${LLM_D_BENCHMARK_DIR}/.git" ]]; then
    log "cloning llm-d-benchmark into ${LLM_D_BENCHMARK_DIR}"
    mkdir -p "$(dirname "${LLM_D_BENCHMARK_DIR}")"
    git clone https://github.com/llm-d/llm-d-benchmark.git "${LLM_D_BENCHMARK_DIR}"
  fi

  log "checking out llm-d-benchmark @ ${LLM_D_BENCHMARK_REF}"
  (cd "${LLM_D_BENCHMARK_DIR}" && git fetch origin "${LLM_D_BENCHMARK_REF}" && git checkout FETCH_HEAD)

  # --uv, not --no-uv/-y: this machine's plain `python3` is often an old
  # system Python (e.g. macOS's 3.9), and -y forces using it as-is with no
  # version check. --uv lets install.sh download and use a real 3.11+
  # itself instead of failing on whatever `python3` happens to resolve to.
  log "running llm-d-benchmark's install.sh"
  (cd "${LLM_D_BENCHMARK_DIR}" && ./install.sh --uv)
}

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

# First standup on a fresh cluster can take a while waiting on the harness
# pod, since it's pulling a ~5.7GB image for the first time - the default
# 120s wait isn't enough. --data-access-timeout raises that
# (llm-d/llm-d-benchmark#1696).
standup_arm() {
  local arm="$1" scenario="${ARM_SCENARIO[$arm]}"
  llmdbench --spec "${SPEC_DIR}/spec-${arm}.yaml" --workspace "$(workspace_dir "${arm}")" \
    standup -p "${scenario}" --skip-smoketest --data-access-timeout "${DATA_ACCESS_TIMEOUT}"
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
  if [[ ! -d "${LLM_D_BENCHMARK_CACHE_DIR}" ]]; then
    log "no managed clone at ${LLM_D_BENCHMARK_CACHE_DIR}, nothing to clean up"
    return 0
  fi
  log "removing ${LLM_D_BENCHMARK_CACHE_DIR}"
  rm -rf "${LLM_D_BENCHMARK_CACHE_DIR}"
}

main() {
  if [[ "${1:-}" == "--clean" ]]; then
    clean
    exit 0
  fi

  declare -A report_paths
  mkdir -p "${RESULTS_DIR}"
  ensure_llm_d_benchmark
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
    report_paths["${arm}"]="${report}"
  done

  (cd "${LLM_D_BENCHMARK_DIR}" && source .venv/bin/activate && python3 -c "
from pathlib import Path
from llmdbenchmark.analysis.cross_treatment import generate_cross_treatment_summary
generate_cross_treatment_summary(Path('${comparison_input}'), output_dir=Path('${RESULTS_DIR}'))
")

  # So callers (e.g. the regression check in CI) can find each arm's raw
  # report without re-deriving these temp paths themselves.
  {
    echo "{"
    echo "  \"baseline\": \"${report_paths[baseline]}\","
    echo "  \"agentgateway\": \"${report_paths[agentgateway]}\""
    echo "}"
  } > "${RESULTS_DIR}/report-paths.json"

  log "done, results in ${RESULTS_DIR}"
}

main "$@"
