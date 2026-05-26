#!/usr/bin/env bash
# install.sh — sets up the full benchmark environment in phases.
# Run with: bash benchmarks/setup/install.sh [--phase N]
# Default: runs all phases.
# Usage: bash benchmarks/setup/install.sh --phase 1
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CLUSTER_NAME="agentgateway-bench"

# ─── helpers ────────────────────────────────────────────────────────────────
log()  { echo "[$(date +%H:%M:%S)] $*"; }
ok()   { echo "[$(date +%H:%M:%S)] ✓ $*"; }
fail() { echo "[$(date +%H:%M:%S)] ✗ $*" >&2; exit 1; }

wait_for_pods() {
  local label="$1" ns="$2" expected="$3"
  log "Waiting for $expected pod(s) with label '$label' in namespace '$ns'..."
  kubectl wait pod \
    --for=condition=Ready \
    --selector="$label" \
    --namespace="$ns" \
    --timeout=120s
  ok "$expected pod(s) ready."
}

# ─── argument parsing ────────────────────────────────────────────────────────
PHASE="all"
while [[ $# -gt 0 ]]; do
  case $1 in
    --phase) PHASE="$2"; shift 2 ;;
    *) fail "Unknown argument: $1" ;;
  esac
done

# ════════════════════════════════════════════════════════════════════════════
# PHASE 0 — host prerequisites (inotify limits for kind)
# ════════════════════════════════════════════════════════════════════════════
# kind clusters with many pods (kube-proxy, coredns, controllers) quickly
# exhaust the default inotify limits (max_user_instances=128). When they are
# too low, kube-proxy crashes with "too many open files", which breaks
# ClusterIP routing and prevents in-pod k8s API calls from working.
check_inotify() {
  local watches instances
  watches=$(cat /proc/sys/fs/inotify/max_user_watches 2>/dev/null || echo 0)
  instances=$(cat /proc/sys/fs/inotify/max_user_instances 2>/dev/null || echo 0)
  if [[ "$watches" -lt 524288 ]] || [[ "$instances" -lt 512 ]]; then
    log "Raising inotify limits (current: watches=${watches}, instances=${instances})..."
    sudo sysctl fs.inotify.max_user_watches=524288 \
      && sudo sysctl fs.inotify.max_user_instances=512 \
      || fail "Could not raise inotify limits. Run manually: sudo sysctl fs.inotify.max_user_watches=524288 fs.inotify.max_user_instances=512"
    ok "inotify limits raised."
  else
    ok "inotify limits OK (watches=${watches}, instances=${instances})."
  fi
}

# ════════════════════════════════════════════════════════════════════════════
# PHASE 1 — kind cluster + inference-sim
# ════════════════════════════════════════════════════════════════════════════
phase1() {
  log "=== PHASE 1: kind cluster + inference-sim ==="

  check_inotify

  # ── 1.1  Install helm if missing ─────────────────────────────────────────
  if ! command -v helm &>/dev/null; then
    log "helm not found — installing to ~/.local/bin (no sudo required)..."
    mkdir -p "${HOME}/.local/bin"
    HELM_VERSION="v3.20.1"
    HELM_TAR="/tmp/helm-${HELM_VERSION}.tar.gz"
    curl -fsSL "https://get.helm.sh/helm-${HELM_VERSION}-linux-amd64.tar.gz" -o "${HELM_TAR}"
    tar -xzf "${HELM_TAR}" -C /tmp linux-amd64/helm
    mv /tmp/linux-amd64/helm "${HOME}/.local/bin/helm"
    rm -f "${HELM_TAR}"
    export PATH="${HOME}/.local/bin:${PATH}"
    ok "helm installed: $(helm version --short)"
  else
    ok "helm already present: $(helm version --short)"
  fi

  # ── 1.2  Create kind cluster (skip if already exists) ────────────────────
  if kind get clusters 2>/dev/null | grep -q "^${CLUSTER_NAME}$"; then
    log "kind cluster '${CLUSTER_NAME}' already exists — skipping creation."
  else
    log "Creating kind cluster '${CLUSTER_NAME}'..."
    kind create cluster \
      --config "${SCRIPT_DIR}/kind-config.yaml" \
      --name "${CLUSTER_NAME}"
    ok "kind cluster created."
  fi

  kubectl cluster-info --context "kind-${CLUSTER_NAME}" >/dev/null
  ok "kubectl context set to kind-${CLUSTER_NAME}."

  # ── 1.3  Apply namespace ──────────────────────────────────────────────────
  log "Applying namespace..."
  kubectl apply -f "${BENCH_DIR}/manifests/00-namespace.yaml"
  ok "Namespace 'inference-benchmark' ready."

  # ── 1.4  Deploy inference-sim ─────────────────────────────────────────────
  log "Deploying llm-d-inference-sim (3 replicas)..."
  kubectl apply -f "${BENCH_DIR}/manifests/inference-sim/deployment.yaml"
  kubectl apply -f "${BENCH_DIR}/manifests/inference-sim/service.yaml"

  wait_for_pods "app=inference-sim" "inference-benchmark" "3"

  # ── 1.5  Smoke test: hit /v1/chat/completions via port-forward ────────────
  log "Smoke-testing inference-sim via port-forward..."
  # Pick the first ready pod
  POD=$(kubectl get pod -n inference-benchmark -l app=inference-sim \
          -o jsonpath='{.items[0].metadata.name}')
  kubectl port-forward "pod/${POD}" 18000:8000 -n inference-benchmark &
  PF_PID=$!
  sleep 2

  HTTP_CODE=$(curl -s -o /tmp/sim-response.json -w "%{http_code}" \
    -X POST http://localhost:18000/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{"model":"meta-llama/Llama-3.1-8B-Instruct","messages":[{"role":"user","content":"hello"}],"max_tokens":10}')

  kill $PF_PID 2>/dev/null || true

  if [[ "$HTTP_CODE" == "200" ]]; then
    ok "inference-sim smoke test passed (HTTP 200)."
    log "Sample response: $(cat /tmp/sim-response.json | head -c 200)"
  else
    fail "inference-sim smoke test failed (HTTP ${HTTP_CODE}). Check pod logs: kubectl logs -n inference-benchmark -l app=inference-sim"
  fi

  ok "=== PHASE 1 COMPLETE ==="
}

# ════════════════════════════════════════════════════════════════════════════
# PHASE 2 — agentgateway controller + EPP + Gateway + InferencePool + HTTPRoute
# ════════════════════════════════════════════════════════════════════════════
phase2() {
  log "=== PHASE 2: agentgateway + EPP + InferencePool ==="

  REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

  # ── 2.1  Gateway API CRDs (v1.5.1) ────────────────────────────���──────────
  log "Installing Gateway API CRDs v1.5.1..."
  kubectl apply -f https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.5.1/standard-install.yaml
  ok "Gateway API CRDs installed."

  # ── 2.1b  Istio CRDs (ServiceEntry, WorkloadEntry, etc.) ─────────────────
  # agentgateway's controller uses Istio's KRT framework, which registers informers
  # for Istio resource types (ServiceEntry, WorkloadEntry, etc.). If those CRDs don't
  # exist before the controller starts, the KRT cache sync never completes.
  ISTIO_VERSION="1.25.1"
  ISTIO_CRD_URL="https://raw.githubusercontent.com/istio/istio/${ISTIO_VERSION}/manifests/charts/base/files/crd-all.gen.yaml"
  log "Installing Istio base CRDs (v${ISTIO_VERSION})..."
  curl -fsSL "${ISTIO_CRD_URL}" | kubectl apply -f - 2>&1 | grep -E "created|configured" | tail -5 || true
  ok "Istio CRDs installed."

  # ── 2.2  agentgateway CRDs (local Helm chart) ────────────────────────────
  log "Installing agentgateway CRDs..."
  helm upgrade --install agentgateway-crds \
    "${REPO_ROOT}/controller/install/helm/agentgateway-crds" \
    --namespace agentgateway-system \
    --create-namespace \
    --wait
  ok "agentgateway CRDs installed."

  # ── 2.3  Inference Extension CRDs (InferencePool) ────────────────────────
  log "Installing Inference Extension CRDs..."
  kubectl apply -f "${REPO_ROOT}/controller/pkg/kgateway/crds/inference-crds.yaml"
  ok "Inference Extension CRDs installed."

  # ── 2.4  agentgateway controller (Helm) ───────────────────────────���──────
  log "Installing agentgateway controller v1.0.1..."
  # Do NOT use --wait: the Istio KRT informers can take >2 min to complete
  # their initial LIST on a local kind cluster. We patch the startup probe
  # to give the pod 10 min, then wait for Ready separately.
  helm upgrade --install agentgateway \
    "${REPO_ROOT}/controller/install/helm/agentgateway" \
    --namespace agentgateway-system \
    --create-namespace \
    --set "image.registry=ghcr.io/agentgateway" \
    --set "image.tag=latest-dev" \
    --set "inferenceExtension.enabled=true"
  ok "agentgateway controller Helm release created."

  log "Patching controller startup probe (failureThreshold=600, 10 min)..."
  kubectl patch deployment agentgateway \
    -n agentgateway-system \
    --type=json \
    -p='[{"op":"replace","path":"/spec/template/spec/containers/0/startupProbe/failureThreshold","value":600}]'
  ok "Startup probe patched."

  # ── 2.5  Wait for GatewayClass to be created by the controller ───────────
  log "Waiting for GatewayClass 'agentgateway' to appear..."
  for i in $(seq 1 120); do
    if kubectl get gatewayclass agentgateway &>/dev/null; then
      ok "GatewayClass 'agentgateway' exists."
      break
    fi
    if [[ $i -eq 120 ]]; then
      fail "GatewayClass 'agentgateway' not found after 600s. Check controller logs."
    fi
    sleep 5
  done

  # ── 2.6  Deploy EPP ───────────────────────────���──────────────────────────��
  log "Deploying EPP (endpoint picker) v1.4.0..."
  kubectl apply -f "${BENCH_DIR}/manifests/agentgateway/epp/serviceaccount.yaml"
  kubectl apply -f "${BENCH_DIR}/manifests/agentgateway/epp/rbac.yaml"
  kubectl apply -f "${BENCH_DIR}/manifests/agentgateway/epp/deployment.yaml"
  kubectl apply -f "${BENCH_DIR}/manifests/agentgateway/epp/service.yaml"
  ok "EPP manifests applied."

  # ── 2.7  Apply Gateway → InferencePool → HTTPRoute ───────────────────────
  log "Applying Gateway, InferencePool, HTTPRoute..."
  kubectl apply -f "${BENCH_DIR}/manifests/agentgateway/gateway.yaml"
  kubectl apply -f "${BENCH_DIR}/manifests/agentgateway/inferencepool.yaml"
  kubectl apply -f "${BENCH_DIR}/manifests/agentgateway/httproute.yaml"
  ok "Gateway resources applied."

  # ── 2.8  Wait for EPP and data plane pods ─────────────────────────���──────
  wait_for_pods "app=epp" "inference-benchmark" "1"

  log "Waiting for agentgateway data plane pod 'benchmark-gateway'..."
  for i in $(seq 1 120); do
    DPOD=$(kubectl get pod -n inference-benchmark \
             -l "gateway.networking.k8s.io/gateway-name=benchmark-gateway" \
             -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true)
    if [[ -n "$DPOD" ]]; then
      kubectl wait pod "$DPOD" \
        --for=condition=Ready \
        --namespace=inference-benchmark \
        --timeout=60s && break
    fi
    if [[ $i -eq 120 ]]; then
      fail "Data plane pod for 'benchmark-gateway' not found after 600s."
    fi
    sleep 5
  done
  ok "Data plane pod ready."

  # ── 2.9  Smoke test: send a request through the full agentgateway stack ──
  log "Smoke-testing agentgateway routing via port-forward on :18080..."
  GW_SVC="benchmark-gateway"
  kubectl port-forward "svc/${GW_SVC}" 18080:8080 -n inference-benchmark &
  PF_PID=$!
  sleep 3

  HTTP_CODE=$(curl -s -o /tmp/gw-response.json -w "%{http_code}" \
    -X POST http://localhost:18080/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{"model":"meta-llama/Llama-3.1-8B-Instruct","messages":[{"role":"user","content":"hello"}],"max_tokens":10}')

  kill $PF_PID 2>/dev/null || true

  if [[ "$HTTP_CODE" == "200" ]]; then
    ok "agentgateway smoke test passed (HTTP 200)."
    log "Sample response: $(cat /tmp/gw-response.json | head -c 200)"
  else
    log "Response body: $(cat /tmp/gw-response.json 2>/dev/null)"
    fail "agentgateway smoke test failed (HTTP ${HTTP_CODE}). Check: kubectl logs -n inference-benchmark -l gateway.networking.k8s.io/gateway-name=benchmark-gateway"
  fi

  ok "=== PHASE 2 COMPLETE ==="
}

# ════════════════════════��════════════════════════════��══════════════════════
# Run selected phases
# ════════════════════════════════════════════���═══════════════════════════════
case "$PHASE" in
  1)        phase1 ;;
  2)        phase2 ;;
  "all")    phase1; phase2 ;;
  *) fail "Unknown phase: $PHASE. Valid values: 1, 2, all" ;;
esac
