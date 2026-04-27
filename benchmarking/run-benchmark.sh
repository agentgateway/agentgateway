#!/usr/bin/env bash

# environment variables:
#   NAMESPACE               namespace Kubernetes target (mandatory)
#   GCS_BUCKET              bucket GCS for results (optional)
#   WORKLOAD                workload subdir, e.g. single-workload (mandatory)
#
#   GW_NAME                 gateway resource name (default: inference-gateway)
#   BASELINE_SVC            baseline service name (default: llm-d-baseline)
#   GW_URL                  override gateway url (optional, discovered by default)
#   BASELINE_URL            override baseline url (optional, discovered by default)
#   HF_TOKEN                token hugging face (optional)
#   GCS_CREDENTIALS_SECRET  k8s secret name containing key.json (optional)
#   GCS_PROJECT             GCP project ID, sets GOOGLE_CLOUD_PROJECT in the pod (optional)
#   KSA_NAME                KSA kubernetes for workloqd identity (optional)
#   BENCHMARK_TIMEOUT       timeout for the job (default: 3600s)

set -euo pipefail

: "${NAMESPACE:?NAMESPACE is mandatory}"
: "${WORKLOAD:?WORKLOAD is mandatory (ex: single-workload)}"

GCS_BUCKET="${GCS_BUCKET:-}"
GCS_CREDENTIALS_SECRET="${GCS_CREDENTIALS_SECRET:-}"
GCS_PROJECT="${GCS_PROJECT:-}"
HF_TOKEN="${HF_TOKEN:-}"
KSA_NAME="${KSA_NAME:-}"
BENCHMARK_TIMEOUT="${BENCHMARK_TIMEOUT:-3600s}"
TIMESTAMP=$(date +"%Y-%m-%d-%H-%M-%S")

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHART_DIR="${SCRIPT_DIR}/inference-perf"
VALUES_DIR="${SCRIPT_DIR}/${WORKLOAD}"

GW_NAME="${GW_NAME:-inference-gateway}"
BASELINE_SVC="${BASELINE_SVC:-llm-d-baseline}"

# auto discovery (override GW_URL/BASELINE_URL to skip)
GW_URL="${GW_URL:-http://$(kubectl get gateway "${GW_NAME}" \
  -n "${NAMESPACE}" \
  -o jsonpath='{.status.addresses[0].value}'):8080}"

BASELINE_URL="${BASELINE_URL:-http://$(kubectl get svc "${BASELINE_SVC}" \
  -n "${NAMESPACE}" \
  -o jsonpath='{.spec.clusterIP}'):80}"

echo "Gateway URL: ${GW_URL}"
echo "Baseline URL: ${BASELINE_URL}"

# run benchmark then cleanup
run_benchmark() {
  local target="$1"   # agentgateway | baseline
  local base_url="$2"
  local release_name="${target}-benchmark"
  local job_name="${release_name}-inference-perf-job"
  local gcs_path="${target}/${WORKLOAD}/${TIMESTAMP}"

  echo "Benchmark: ${target}"
  echo "URL: ${base_url}"
  [[ -n "${GCS_BUCKET}" ]] && echo "GCS path: gs://${GCS_BUCKET}/${gcs_path}"

  local helm_args=(
    install "${release_name}" "${CHART_DIR}"
    -f "${SCRIPT_DIR}/values.yaml"
    -f "${VALUES_DIR}/${target}-values.yaml"
    --namespace "${NAMESPACE}"
    --set "config.server.base_url=${base_url}"
  )

  if [[ -n "${GCS_BUCKET}" ]]; then
    helm_args+=(
      --set "config.storage.google_cloud_storage.bucket_name=${GCS_BUCKET}"
      --set "config.storage.google_cloud_storage.path=${gcs_path}"
    )
  fi

  if [[ -n "${GCS_CREDENTIALS_SECRET}" ]]; then
    helm_args+=(--set "gcsCredentials.secretName=${GCS_CREDENTIALS_SECRET}")
  fi

  if [[ -n "${GCS_PROJECT}" ]]; then
    helm_args+=(--set "gcsProject=${GCS_PROJECT}")
  fi

  if [[ -n "${HF_TOKEN}" ]]; then
    helm_args+=(--set "token.hfToken=${HF_TOKEN}")
  fi

  if [[ -n "${KSA_NAME}" ]]; then
    helm_args+=(--set "job.serviceAccountName=${KSA_NAME}")
  fi

  helm "${helm_args[@]}"

  echo "Waiting for job ${job_name} to complete (timeout: ${BENCHMARK_TIMEOUT})..."
  if ! kubectl wait --for=condition=complete "job/${job_name}" \
       -n "${NAMESPACE}" \
       --timeout="${BENCHMARK_TIMEOUT}"; then
    echo "ERROR: job ${job_name} did not complete within the timeout." >&2
    kubectl describe job "${job_name}" -n "${NAMESPACE}" >&2
    kubectl logs -l "job-name=${job_name}" -n "${NAMESPACE}" --tail=50 >&2
    helm uninstall "${release_name}" -n "${NAMESPACE}" --ignore-not-found || true
    return 1
  fi

  echo "Job ${job_name} completed successfully."
  [[ -n "${GCS_BUCKET}" ]] && echo "Results : gs://${GCS_BUCKET}/${gcs_path}"

  helm uninstall "${release_name}" -n "${NAMESPACE}" --ignore-not-found
}

# sequential run: agentgateway then baseline
run_benchmark "agentgateway" "${GW_URL}"
run_benchmark "baseline"     "${BASELINE_URL}"

echo ""
echo "All benchmarks completed."
echo "Timestamp : ${TIMESTAMP}"
