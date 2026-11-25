#!/usr/bin/env bash

# Checks that each agentgateway policy rejects its fixture. A fixture directory is
# named after the finding id it must produce.

set -o errexit -o nounset -o pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "${ROOT}"

failed=0
for fixture in telemetry/policies/testdata/*/; do
  id="$(basename "${fixture}")"
  if output="$(telemetry/weaver.sh registry check -r "${fixture}" --v2 --policy telemetry/policies --diagnostic-format json 2>&1)"; then
    echo "FAIL: ${id}: the check passed"
    failed=1
  elif ! grep -q "\"id\": \"${id}\"" <<<"${output}"; then
    echo "FAIL: ${id}: the check failed without that finding"
    echo "${output}"
    failed=1
  else
    echo "ok: ${id}"
  fi
done
exit "${failed}"
