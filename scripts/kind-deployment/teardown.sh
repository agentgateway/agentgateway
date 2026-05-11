#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-agw-xds-test}"

if kind get clusters | grep -q "^${CLUSTER_NAME}$"; then
  echo "Deleting kind cluster '${CLUSTER_NAME}'..."
  kind delete cluster --name "${CLUSTER_NAME}"
else
  echo "Kind cluster '${CLUSTER_NAME}' does not exist."
fi
