#!/usr/bin/env bash

set -o errexit
set -o pipefail
set -o nounset

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-kind}"
EXPECTED_IMAGES_FILE="${SCRIPT_DIR}/${TEST_MODE:?TEST_MODE must be set}-images.txt"

# Locally built images get a fresh tag each run, so only their repository is compared.
if ! diff -u "${EXPECTED_IMAGES_FILE}" <(
  docker exec "${CLUSTER_NAME}-control-plane" ctr --namespace k8s.io images list --quiet |
    grep -v 'sha256:' |
    sed -E 's#^(localhost:[0-9]+/agentgateway(-controller)?):.*#\1:*#' |
    LC_ALL=C sort -u
); then
  echo "Kind's images do not match ${EXPECTED_IMAGES_FILE}" >&2
  exit 1
fi
