#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-agw-xds-test}"
GATEWAY_API_VERSION="${GATEWAY_API_VERSION:-v1.5.0}"
AGENTGATEWAY_VERSION="${AGENTGATEWAY_VERSION:-v1.1.0}"
SYSTEM_NAMESPACE="${SYSTEM_NAMESPACE:-agentgateway-system}"
TEST_NAMESPACE="${TEST_NAMESPACE:-agentgateway-test}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "==> 1/6 Ensuring kind cluster '${CLUSTER_NAME}' exists"
if ! kind get clusters | grep -q "^${CLUSTER_NAME}$"; then
  kind create cluster --name "${CLUSTER_NAME}"
else
  echo "    already exists, reusing"
fi
kubectl config use-context "kind-${CLUSTER_NAME}" >/dev/null

echo "==> 2/6 Installing Gateway API CRDs (${GATEWAY_API_VERSION})"
kubectl apply --server-side -f \
  "https://github.com/kubernetes-sigs/gateway-api/releases/download/${GATEWAY_API_VERSION}/standard-install.yaml"

echo "==> 3/6 Installing agentgateway-crds helm chart (${AGENTGATEWAY_VERSION})"
helm upgrade -i \
  --create-namespace \
  --namespace "${SYSTEM_NAMESPACE}" \
  --version "${AGENTGATEWAY_VERSION}" \
  agentgateway-crds \
  oci://cr.agentgateway.dev/charts/agentgateway-crds

echo "==> 4/6 Installing agentgateway helm chart (${AGENTGATEWAY_VERSION})"
helm upgrade -i \
  --namespace "${SYSTEM_NAMESPACE}" \
  --version "${AGENTGATEWAY_VERSION}" \
  agentgateway \
  oci://cr.agentgateway.dev/charts/agentgateway

echo "    waiting for the controller deployment to be ready..."
kubectl -n "${SYSTEM_NAMESPACE}" rollout status deployment/agentgateway --timeout=8m

echo "==> 5/6 Applying test namespace, backend, Gateway, and HTTPRoute"
kubectl apply -f "${SCRIPT_DIR}/manifests/test-app.yaml"

echo "    waiting for the gateway data plane pod to be created and ready..."
for _ in $(seq 1 30); do
  if kubectl -n "${TEST_NAMESPACE}" get deployment -l gateway.networking.k8s.io/gateway-name=test-gateway 2>/dev/null \
     | grep -q test-gateway; then
    break
  fi
  sleep 2
done
kubectl -n "${TEST_NAMESPACE}" wait --for=condition=available \
  deployment -l gateway.networking.k8s.io/gateway-name=test-gateway \
  --timeout=3m

GW_POD=$(kubectl -n "${TEST_NAMESPACE}" get pods \
  -l gateway.networking.k8s.io/gateway-name=test-gateway \
  -o jsonpath='{.items[0].metadata.name}')

echo "==> 6/6 Setup complete"
echo
echo "Gateway data plane pod: ${TEST_NAMESPACE}/${GW_POD}"
echo
echo "In a separate terminal, run:"
echo
echo "  kubectl -n ${TEST_NAMESPACE} port-forward pod/${GW_POD} 15000 8080 8621 8622"
echo
echo "Then start the dev UI (cd ui && yarn dev) and open http://localhost:5173."
