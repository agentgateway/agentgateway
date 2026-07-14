#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
REPO_ROOT="${SCRIPT_DIR}/.."
CLUSTER_NAME="agentgateway-benchmark"
EPP_IMAGE_REGISTRY="ghcr.io/llm-d"
EPP_IMAGE_REPOSITORY="llm-d-router-endpoint-picker-dev"
EPP_IMAGE_TAG="main"
EPP_CHART_TMPDIR=""

cleanup() {
  if [[ -n "${EPP_CHART_TMPDIR}" && -d "${EPP_CHART_TMPDIR}" ]]; then
    rm -rf "${EPP_CHART_TMPDIR}"
  fi
}
trap cleanup EXIT

# deploys the fake vllm server - shared by both namespaces
deploy_model_simulator() {
  local ns="$1"

  kubectl apply -n "${ns}" -f - <<EOF
apiVersion: apps/v1
kind: Deployment
metadata:
  name: vllm-mock
spec:
  replicas: 1
  selector:
    matchLabels:
      app: vllm-mock
  template:
    metadata:
      labels:
        app: vllm-mock
    spec:
      containers:
      - name: simulator
        image: ghcr.io/llm-d/llm-d-inference-sim:v0.9.2
        args:
        - --model
        - mock-model
        - --port
        - "8000"
        # default 1024 is too small for both workloads, bumped to avoid silent 400s
        - --max-model-len
        - "16384"
        ports:
        - containerPort: 8000
---
apiVersion: v1
kind: Service
metadata:
  name: vllm-mock
spec:
  ports:
  - port: 8000
    protocol: TCP
    targetPort: 8000
  selector:
    app: vllm-mock
EOF
}

# plain service - just vllm-mock behind a bare Service, no EPP
deploy_baseline() {
  local ns="$1"

  echo "deploying plain-service baseline in ${ns}..."
  kubectl create namespace "${ns}" --dry-run=client -o yaml | kubectl apply -f -
  deploy_model_simulator "${ns}"
  kubectl rollout status deployment/vllm-mock -n "${ns}" --timeout=120s
}

# agentgateway + EPP setup
deploy_setup() {
  local ns="$1"
  local proxy_type="$2"
  local extra_args=("${@:3}")

  echo "deploying ${proxy_type} in ${ns}..."
  kubectl create namespace "${ns}" --dry-run=client -o yaml | kubectl apply -f -
  deploy_model_simulator "${ns}"

  helm install standalone-router "${EPP_CHART}" \
    -n "${ns}" \
    --set router.modelServers.matchLabels.app=vllm-mock \
    --set router.inferencePool.create=false \
    --set router.proxy.proxyType="${proxy_type}" \
    --set router.epp.image.registry="${EPP_IMAGE_REGISTRY}" \
    --set router.epp.image.repository="${EPP_IMAGE_REPOSITORY}" \
    --set router.epp.image.tag="${EPP_IMAGE_TAG}" \
    --set router.epp.image.pullPolicy=IfNotPresent \
    --set 'router.extraServicePorts[0].name=http' \
    --set 'router.extraServicePorts[0].port=8081' \
    --set 'router.extraServicePorts[0].protocol=TCP' \
    --set router.epp.resources.requests.cpu=100m \
    --set router.epp.resources.requests.memory=128Mi \
    --set router.proxy.resources.requests.cpu=100m \
    --set router.proxy.resources.requests.memory=128Mi \
    "${extra_args[@]}"

  kubectl rollout status deployment/vllm-mock -n "${ns}" --timeout=120s
  kubectl rollout status deployment/standalone-router-epp -n "${ns}" --timeout=120s
}

run_benchmark_job() {
  local name="$1"
  local ns="$2"
  local workload_values="$3"
  local base_url="$4"

  echo "running job ${name} against ${base_url}..."
  helm install "${name}" "${SCRIPT_DIR}/inference-perf" \
    -n "${ns}" \
    -f "${SCRIPT_DIR}/workloads/${workload_values}" \
    --set config.server.base_url="${base_url}" \
    --set job.image.repository="local-inference-perf" \
    --set job.image.tag="latest"
}

copy_results() {
  local job_name="$1"
  local ns="$2"
  local label_name="$3"

  local pod_name
  pod_name=$(kubectl get pods -n "${ns}" -l "job-name=${job_name}" -o jsonpath='{.items[0].metadata.name}')

  local out_path="${SCRIPT_DIR}/output/default-run/${label_name}/results/json"
  mkdir -p "${out_path}"

  kubectl cp -n "${ns}" "${pod_name}:/reports/." "${out_path}" || {
    echo "kubectl cp failed on completed pod, saving logs instead"
    kubectl logs -n "${ns}" "${pod_name}" > "${out_path}/fallback_logs.json"
  }
}

echo "=== starting benchmark run ==="

# 1. create Kind cluster if not already running
if ! kind get clusters | grep -q "^${CLUSTER_NAME}$"; then
  echo "creating Kind cluster..."
  kind create cluster --name "${CLUSTER_NAME}" --config - <<EOF
apiVersion: kind.x-k8s.io/v1alpha4
kind: Cluster
nodes:
- role: control-plane
EOF
else
  echo "cluster already exists, skipping create"
fi

# 2. fetch llm-d-router chart via sparse clone
echo "fetching llm-d-router chart..."
EPP_CHART_TMPDIR=$(mktemp -d)
git clone --depth=1 --filter=blob:none --sparse \
  https://github.com/llm-d/llm-d-router.git "${EPP_CHART_TMPDIR}/llm-d-router" 2>/dev/null
cd "${EPP_CHART_TMPDIR}/llm-d-router"
git sparse-checkout set config/charts >/dev/null 2>&1
cd "${SCRIPT_DIR}"
EPP_CHART="${EPP_CHART_TMPDIR}/llm-d-router/config/charts/llm-d-router-standalone"

# 3. build agentgateway image
echo "building agentgateway image..."
docker build \
  --build-arg PROFILE=quick-release \
  --build-arg VERSION=dev \
  --build-arg GIT_REVISION=dev \
  -t cr.agentgateway.dev/agentgateway:latest-dev \
  -f "${REPO_ROOT}/Dockerfile" \
  "${REPO_ROOT}"

# 4. pull EPP and simulator images
echo "pulling EPP image..."
docker pull "${EPP_IMAGE_REGISTRY}/${EPP_IMAGE_REPOSITORY}:${EPP_IMAGE_TAG}"

echo "pulling inference-sim..."
docker pull ghcr.io/llm-d/llm-d-inference-sim:v0.9.2 || true

# Kind can't pull multi-arch images directly, so re-tag via create+commit
echo "pulling inference-perf..."
docker pull --platform linux/amd64 quay.io/inference-perf/inference-perf:latest || true
docker rm -f temp-perf || true
docker create --name temp-perf quay.io/inference-perf/inference-perf:latest || true
docker commit temp-perf local-inference-perf:latest || true
docker rm -f temp-perf

# 5. load images into Kind
echo "loading images into cluster..."
kind load docker-image cr.agentgateway.dev/agentgateway:latest-dev --name "${CLUSTER_NAME}"
kind load docker-image "${EPP_IMAGE_REGISTRY}/${EPP_IMAGE_REPOSITORY}:${EPP_IMAGE_TAG}" --name "${CLUSTER_NAME}"
kind load docker-image local-inference-perf:latest --name "${CLUSTER_NAME}"

# 6. clean up any previous run
kubectl delete ns agentgateway-bench plain-service-bench --ignore-not-found=true || true

# 7. deploy both targets
deploy_baseline "plain-service-bench"
deploy_setup "agentgateway-bench" "agentgateway" \
  --set router.epp.flags.secure-serving=false \
  --set router.proxy.presets.agentgateway.image="cr.agentgateway.dev/agentgateway:latest-dev" \
  --set router.proxy.presets.agentgateway.pullPolicy=IfNotPresent \
  --set 'router.extraServicePorts[0].targetPort=http'

# give agentgateway a moment to discover endpoints
sleep 10

# 8. run inference-perf jobs
run_benchmark_job "plain-service-prefill" "plain-service-bench" "prefill-heavy-values.yaml" "http://vllm-mock:8000"
run_benchmark_job "plain-service-decode"  "plain-service-bench" "decode-heavy-values.yaml"  "http://vllm-mock:8000"
run_benchmark_job "agentgateway-prefill"  "agentgateway-bench"  "prefill-heavy-values.yaml" "http://standalone-router-epp:8081"
run_benchmark_job "agentgateway-decode"   "agentgateway-bench"  "decode-heavy-values.yaml"  "http://standalone-router-epp:8081"

# 9. wait for all jobs to finish
echo "waiting for jobs..."
kubectl wait --for=condition=complete --timeout=300s job/plain-service-prefill-inference-perf-job -n plain-service-bench
kubectl wait --for=condition=complete --timeout=300s job/plain-service-decode-inference-perf-job  -n plain-service-bench
kubectl wait --for=condition=complete --timeout=300s job/agentgateway-prefill-inference-perf-job  -n agentgateway-bench
kubectl wait --for=condition=complete --timeout=300s job/agentgateway-decode-inference-perf-job   -n agentgateway-bench

# 10. collect results
mkdir -p "${SCRIPT_DIR}/output/default-run"
copy_results "plain-service-prefill-inference-perf-job" "plain-service-bench" "plain-service-bench-prefill"
copy_results "plain-service-decode-inference-perf-job"  "plain-service-bench" "plain-service-bench-decode"
copy_results "agentgateway-prefill-inference-perf-job"  "agentgateway-bench"  "agentgateway-bench-prefill"
copy_results "agentgateway-decode-inference-perf-job"   "agentgateway-bench"  "agentgateway-bench-decode"

echo "=== done, results under ${SCRIPT_DIR}/output/default-run/ ==="
