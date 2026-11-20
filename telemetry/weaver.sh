#!/usr/bin/env bash

# Runs OpenTelemetry Weaver at the version pinned in telemetry/versions.env.
# A local weaver binary is used only when its version matches exactly, since a
# different version can give a different answer. Otherwise the pinned image runs.

set -o errexit -o nounset -o pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "${ROOT}"

# shellcheck source=telemetry/versions.env
source telemetry/versions.env

CONTAINER_RUNTIME="${CONTAINER_RUNTIME:-docker}"

if command -v weaver >/dev/null 2>&1; then
  local_version="v$(weaver --version | awk '{print $NF}')"
  if [[ "${local_version}" != "${WEAVER_VERSION}" ]]; then
    echo "FAIL: local weaver is ${local_version}, the pinned version is ${WEAVER_VERSION}." >&2
    echo "Install ${WEAVER_VERSION} or remove weaver from PATH to use the pinned image." >&2
    exit 1
  fi
  exec weaver "$@"
fi

if ! command -v "${CONTAINER_RUNTIME}" >/dev/null 2>&1; then
  echo "FAIL: needs weaver ${WEAVER_VERSION} or ${CONTAINER_RUNTIME}." >&2
  exit 1
fi

# WEAVER_CONTAINER_ARGS adds container options, such as published ports.
# shellcheck disable=SC2086
exec "${CONTAINER_RUNTIME}" run --rm ${WEAVER_CONTAINER_ARGS:-} \
  --user "$(id -u):$(id -g)" \
  --env HOME=/tmp \
  --volume "${ROOT}:/workspace" \
  --workdir /workspace \
  "${WEAVER_IMAGE}" \
  "$@"
