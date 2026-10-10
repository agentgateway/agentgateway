#!/usr/bin/env bash
set -euo pipefail

NAMESPACE=openshell
GATEWAY_NAME=openshell-ingress
SANDBOX_NAME=agw-example
OPENSHELL_BIN=${OPENSHELL_BIN:-openshell}
LOCAL_PORT=${OPENSHELL_LOCAL_PORT:-18080}
POLICY_PROBE_NAMESPACE=openshell-policy-probe

for command in kubectl nc "${OPENSHELL_BIN}"; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "required command not found: ${command}" >&2
    exit 1
  fi
done

WORK_DIR=$(mktemp -d)
PORT_FORWARD_PID=
SANDBOX_CREATED=false

cleanup() {
  if [[ "${SANDBOX_CREATED}" == true ]]; then
    XDG_CONFIG_HOME="${WORK_DIR}/config" \
      "${OPENSHELL_BIN}" sandbox delete "${SANDBOX_NAME}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${PORT_FORWARD_PID}" ]]; then
    kill "${PORT_FORWARD_PID}" 2>/dev/null || true
  fi
  kubectl delete namespace "${POLICY_PROBE_NAMESPACE}" \
    --ignore-not-found --wait=false >/dev/null 2>&1 || true
  case "${WORK_DIR}" in
    /tmp/*|/private/var/folders/*|/var/folders/*)
      rm -rf -- "${WORK_DIR}"
      ;;
  esac
}
trap cleanup EXIT INT TERM

wait_for_failed_request() {
  local attempts=20
  for _ in $(seq 1 "${attempts}"); do
    if ! kubectl exec -n "${POLICY_PROBE_NAMESPACE}" policy-client -- \
      wget -q -T 2 -O /dev/null http://policy-server:8080 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_successful_request() {
  local attempts=20
  for _ in $(seq 1 "${attempts}"); do
    if kubectl exec -n "${POLICY_PROBE_NAMESPACE}" policy-client -- \
      wget -q -T 2 -O /dev/null http://policy-server:8080 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  return 1
}

echo "Checking ingress and egress NetworkPolicy enforcement"
kubectl create namespace "${POLICY_PROBE_NAMESPACE}"
kubectl run policy-server -n "${POLICY_PROBE_NAMESPACE}" \
  --image=busybox:1.37.0 --labels=app=policy-server \
  --command -- sh -c 'mkdir -p /www && echo ready >/www/index.html && httpd -f -p 8080 -h /www'
kubectl run policy-client -n "${POLICY_PROBE_NAMESPACE}" \
  --image=busybox:1.37.0 --labels=app=policy-client \
  --command -- sleep 3600
kubectl expose pod policy-server -n "${POLICY_PROBE_NAMESPACE}" \
  --port=8080 --target-port=8080
kubectl wait --for=condition=Ready pod/policy-server pod/policy-client \
  -n "${POLICY_PROBE_NAMESPACE}" --timeout=120s
kubectl exec -n "${POLICY_PROBE_NAMESPACE}" policy-client -- \
  wget -q -T 5 -O - http://policy-server:8080 | grep -Fx ready >/dev/null

kubectl apply -n "${POLICY_PROBE_NAMESPACE}" -f - <<'EOF'
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: deny-server-ingress
spec:
  podSelector:
    matchLabels:
      app: policy-server
  policyTypes:
    - Ingress
EOF
if ! wait_for_failed_request; then
  echo "the cluster CNI did not enforce the ingress NetworkPolicy" >&2
  exit 1
fi

kubectl delete networkpolicy deny-server-ingress \
  -n "${POLICY_PROBE_NAMESPACE}" --wait=true
if ! wait_for_successful_request; then
  echo "connectivity did not recover after removing the ingress policy" >&2
  exit 1
fi
kubectl apply -n "${POLICY_PROBE_NAMESPACE}" -f - <<'EOF'
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: deny-client-egress
spec:
  podSelector:
    matchLabels:
      app: policy-client
  policyTypes:
    - Egress
EOF
if ! wait_for_failed_request; then
  echo "the cluster CNI did not enforce the egress NetworkPolicy" >&2
  exit 1
fi
kubectl delete namespace "${POLICY_PROBE_NAMESPACE}" --wait=true

echo "Checking Gateway API resources"
kubectl wait --for=condition=Programmed "gateway/${GATEWAY_NAME}" \
  -n "${NAMESPACE}" --timeout=300s
kubectl wait \
  --for=jsonpath='{.status.parents[0].conditions[?(@.type=="Accepted")].status}'=True \
  grpcroute/openshell -n "${NAMESPACE}" --timeout=300s
kubectl wait \
  --for=jsonpath='{.status.parents[0].conditions[?(@.type=="ResolvedRefs")].status}'=True \
  grpcroute/openshell -n "${NAMESPACE}" --timeout=300s
kubectl rollout status "deployment/${GATEWAY_NAME}" \
  -n "${NAMESPACE}" --timeout=300s
kubectl rollout status statefulset/openshell \
  -n "${NAMESPACE}" --timeout=300s

kubectl port-forward -n "${NAMESPACE}" \
  "service/${GATEWAY_NAME}" "${LOCAL_PORT}:80" \
  >"${WORK_DIR}/port-forward.log" 2>&1 &
PORT_FORWARD_PID=$!

for _ in $(seq 1 30); do
  if nc -z 127.0.0.1 "${LOCAL_PORT}" 2>/dev/null; then
    break
  fi
  sleep 1
done
if ! nc -z 127.0.0.1 "${LOCAL_PORT}" 2>/dev/null; then
  cat "${WORK_DIR}/port-forward.log" >&2
  echo "timed out waiting for the agentgateway port forward" >&2
  exit 1
fi

echo "Checking OpenShell through agentgateway"
mkdir -p "${WORK_DIR}/config"
XDG_CONFIG_HOME="${WORK_DIR}/config" \
  "${OPENSHELL_BIN}" gateway add "http://127.0.0.1:${LOCAL_PORT}" \
  --local --name agentgateway-example
XDG_CONFIG_HOME="${WORK_DIR}/config" "${OPENSHELL_BIN}" status
XDG_CONFIG_HOME="${WORK_DIR}/config" \
  "${OPENSHELL_BIN}" sandbox create \
  --name "${SANDBOX_NAME}" --detach -- sleep 3600
SANDBOX_CREATED=true
actual=$(XDG_CONFIG_HOME="${WORK_DIR}/config" \
  "${OPENSHELL_BIN}" sandbox exec "${SANDBOX_NAME}" \
  --no-login-shell -- sh -c 'printf agentgateway-openshell')
if [[ "${actual}" != "agentgateway-openshell" ]]; then
  echo "unexpected sandbox output: ${actual}" >&2
  exit 1
fi
XDG_CONFIG_HOME="${WORK_DIR}/config" \
  "${OPENSHELL_BIN}" sandbox delete "${SANDBOX_NAME}"
SANDBOX_CREATED=false

echo "OpenShell integration verification passed"
