#!/usr/bin/env bash
# teardown.sh — destroys the benchmark kind cluster entirely.
set -euo pipefail

CLUSTER_NAME="agentgateway-bench"

echo "[$(date +%H:%M:%S)] Deleting kind cluster '${CLUSTER_NAME}'..."
kind delete cluster --name "${CLUSTER_NAME}"
echo "[$(date +%H:%M:%S)] ✓ Cluster deleted."
