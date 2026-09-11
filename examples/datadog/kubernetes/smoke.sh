#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "${script_dir}"

namespace=agentgateway-system
pids=()
cleanup() {
  if ((${#pids[@]})); then
    kill "${pids[@]}" 2>/dev/null || true
    wait "${pids[@]}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

kubectl port-forward -n "${namespace}" service/agentgateway-proxy 13000:80 >/dev/null 2>&1 &
pids+=("$!")
kubectl port-forward -n "${namespace}" deployment/agentgateway-proxy 18520:15020 >/dev/null 2>&1 &
pids+=("$!")
kubectl port-forward -n "${namespace}" service/datadog-fixture 18080:8080 >/dev/null 2>&1 &
pids+=("$!")
kubectl port-forward -n "${namespace}" service/agentgateway 19092:9092 >/dev/null 2>&1 &
pids+=("$!")

sleep 1
for pid in "${pids[@]}"; do
  if ! kill -0 "${pid}" 2>/dev/null; then
    echo "A port-forward failed. Ensure ports 13000, 18080, 18520, and 19092 are available." >&2
    exit 1
  fi
done

for _ in {1..30}; do
  if curl --fail --silent http://127.0.0.1:18520/metrics >/dev/null \
    && curl --fail --silent http://127.0.0.1:18080/health >/dev/null \
    && curl --fail --silent http://127.0.0.1:19092/metrics >/dev/null; then
    break
  fi
  sleep 1
done

curl --fail --silent http://127.0.0.1:19092/metrics \
  | grep --quiet '^agentgateway_controller_reconciliations'
uv run ../smoke.py
