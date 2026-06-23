#!/usr/bin/env bash
# run-benchmark.sh — runs the Standard scenario benchmark (agentgateway vs baseline).
#
# Usage:
#   bash benchmarks/scripts/run-benchmark.sh
#
# What it does:
#   1. Applies ConfigMaps and submits both inference-perf Jobs in parallel.
#   2. Waits for both Jobs to complete (timeout: 10 min).
#   3. Extracts JSON results from pod logs into benchmarks/results/.
#   4. Prints a summary table of key metrics.
#
# Prerequisites: the cluster must already be set up (install.sh phases 1+2).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
MANIFESTS="${BENCH_DIR}/manifests/inference-perf"
RESULTS_DIR="${BENCH_DIR}/results"
NS="inference-benchmark"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"

log()  { echo "[$(date +%H:%M:%S)] $*"; }
ok()   { echo "[$(date +%H:%M:%S)] ✓ $*"; }
fail() { echo "[$(date +%H:%M:%S)] ✗ $*" >&2; exit 1; }

# ── Pre-flight check ──────────────────────────────────────────────────────────
log "Checking cluster readiness..."
kubectl get gateway benchmark-gateway -n "${NS}" -o jsonpath='{.status.conditions[?(@.type=="Programmed")].status}' \
  | grep -q "True" || fail "Gateway 'benchmark-gateway' is not Programmed. Run install.sh first."
kubectl get pods -n "${NS}" -l app=epp --field-selector=status.phase=Running \
  | grep -q epp || fail "EPP pod is not running. Run install.sh first."
ok "Cluster ready."

# ── Clean up any previous job runs ───────────────────────────────────────────
for job in inference-perf-agentgateway inference-perf-baseline; do
  if kubectl get job "${job}" -n "${NS}" &>/dev/null; then
    log "Deleting previous job '${job}'..."
    kubectl delete job "${job}" -n "${NS}" --wait=true
  fi
done

# ── Apply ConfigMaps and submit Jobs ─────────────────────────────────────────
log "Applying inference-perf ConfigMaps..."
kubectl apply -f "${MANIFESTS}/configmap-agentgateway.yaml"
kubectl apply -f "${MANIFESTS}/configmap-baseline.yaml"
ok "ConfigMaps applied."

log "Submitting inference-perf Jobs (both in parallel)..."
kubectl apply -f "${MANIFESTS}/job-agentgateway.yaml"
kubectl apply -f "${MANIFESTS}/job-baseline.yaml"
ok "Jobs submitted."

# ── Wait for Jobs to complete ─────────────────────────────────────────────────
log "Waiting for both Jobs to complete (timeout: 10 min)..."
for job in inference-perf-agentgateway inference-perf-baseline; do
  log "  Waiting for ${job}..."
  kubectl wait job/"${job}" \
    --for=condition=complete \
    --namespace="${NS}" \
    --timeout=600s \
    || {
      log "Job ${job} did not complete. Logs:"
      POD=$(kubectl get pod -n "${NS}" -l "job-name=${job}" \
              -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true)
      [[ -n "$POD" ]] && kubectl logs -n "${NS}" "${POD}" || true
      fail "Job ${job} failed or timed out."
    }
  ok "${job} complete."
done

# ── Extract results from pod logs ────────────────────────────────────────────
# inference-perf writes JSON to /results/ inside the container and cats it to
# stdout at the end of the job. We extract it from kubectl logs because
# kubectl cp does not work on completed (Succeeded) pods.
mkdir -p "${RESULTS_DIR}/agentgateway/${TIMESTAMP}"
mkdir -p "${RESULTS_DIR}/baseline/${TIMESTAMP}"

AGW_POD=$(kubectl get pod -n "${NS}" -l "job-name=inference-perf-agentgateway" \
            -o jsonpath='{.items[0].metadata.name}')
BASE_POD=$(kubectl get pod -n "${NS}" -l "job-name=inference-perf-baseline" \
             -o jsonpath='{.items[0].metadata.name}')

log "Extracting results from pod logs..."

python3 - "${AGW_POD}" "${RESULTS_DIR}/agentgateway/${TIMESTAMP}" \
          "${BASE_POD}" "${RESULTS_DIR}/baseline/${TIMESTAMP}" << 'PYEOF'
import subprocess, re, os, sys

def extract(pod, outdir):
    logs = subprocess.check_output(
        ["kubectl", "logs", "-n", "inference-benchmark", pod]
    ).decode()
    # Normalise: "}--- /results/foo.json ---" → "}\n--- /results/foo.json ---"
    logs = re.sub(r"(})(--- /results/)", r"\1\n\2", logs)
    in_results = False
    current_file = None
    buf = []
    for line in logs.splitlines():
        if "=== RESULTS JSON ===" in line:
            in_results = True
            continue
        if not in_results:
            continue
        m = re.match(r"^--- /results/(.+?) ---$", line.strip())
        if m:
            if current_file and buf:
                with open(os.path.join(outdir, current_file), "w") as f:
                    f.write("\n".join(buf) + "\n")
            current_file = m.group(1)
            buf = []
        elif current_file:
            buf.append(line)
    if current_file and buf:
        with open(os.path.join(outdir, current_file), "w") as f:
            f.write("\n".join(buf) + "\n")

agw_pod, agw_dir, base_pod, base_dir = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
extract(agw_pod, agw_dir)
extract(base_pod, base_dir)
PYEOF

ok "Results extracted."

# ── Print quick comparison summary ───────────────────────────────────────────
python3 - "${RESULTS_DIR}/agentgateway/${TIMESTAMP}" \
          "${RESULTS_DIR}/baseline/${TIMESTAMP}" << 'PYEOF'
import json, os, sys

def load(d, stage):
    with open(os.path.join(d, f"stage_{stage}_lifecycle_metrics.json")) as f:
        return json.load(f)

agw_dir, base_dir = sys.argv[1], sys.argv[2]

print("\n  Standard scenario results (agentgateway vs baseline)")
print(f"  {'QPS':>4}  {'AGW p90 lat (ms)':>18}  {'Base p90 lat (ms)':>19}  {'EPP overhead (ms)':>18}  {'Failures':>8}")
for stage, qps in enumerate([10, 20, 30]):
    a = load(agw_dir, stage)
    b = load(base_dir, stage)
    a_p90 = (a["successes"]["latency"]["request_latency"] or {}).get("p90", 0) * 1000
    b_p90 = (b["successes"]["latency"]["request_latency"] or {}).get("p90", 0) * 1000
    fails = a["failures"]["count"] + b["failures"]["count"]
    print(f"  {qps:>4}  {a_p90:>18.1f}  {b_p90:>19.1f}  {a_p90-b_p90:>18.1f}  {fails:>8}")
PYEOF

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
ok "=== Benchmark complete ==="
echo ""
echo "Results saved to:"
echo "  agentgateway:  ${RESULTS_DIR}/agentgateway/${TIMESTAMP}/"
echo "  baseline:      ${RESULTS_DIR}/baseline/${TIMESTAMP}/"
echo ""
echo "To analyse results, open the notebook:"
echo "  jupyter notebook benchmarks/analysis/standard_scenario.ipynb"
