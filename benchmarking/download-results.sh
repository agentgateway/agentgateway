#!/usr/bin/env bash

# downloads a full benchmark (agentgateway + baseline) from GCS (for now)
# ./download-results.sh <BUCKET> <TIMESTAMP> [WORKLOAD]

set -euo pipefail

: "${1:?Usage: $0 <BUCKET> <TIMESTAMP> [WORKLOAD]}"
: "${2:?Usage: $0 <BUCKET> <TIMESTAMP> [WORKLOAD]}"

BUCKET="$1"
TIMESTAMP="$2"
WORKLOAD="${3:-single-workload}"
output_dir="${OUTPUT_DIR:-output}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCAL_DIR="${SCRIPT_DIR}/${output_dir}/${TIMESTAMP}"

echo "Downloading run ${TIMESTAMP} (${WORKLOAD})..."

for target in agentgateway baseline; do
  gcs_path="gs://${BUCKET}/${target}/${WORKLOAD}/${TIMESTAMP}"
  local_path="${LOCAL_DIR}/${target}"
  echo "${target}: ${gcs_path} to ${local_path}"
  mkdir -p "${local_path}"
  gsutil cp -r "${gcs_path}/*" "${local_path}/"
done

echo ""
echo "Run downloaded to: ${LOCAL_DIR}"
echo "${LOCAL_DIR}/agentgateway/"
echo "${LOCAL_DIR}/baseline/"