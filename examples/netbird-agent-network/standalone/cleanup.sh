#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "${SCRIPT_DIR}"

PURGE=false
if [[ ${1:-} == --volumes ]]; then
  PURGE=true
elif (($# > 0)); then
  echo "usage: ./cleanup.sh [--volumes]" >&2
  exit 1
fi

if [[ ! -f .env || ! -f runtime/generated.env ]]; then
  echo "nothing to clean up"
  exit 0
fi

COMPOSE=(docker compose --env-file .env --env-file runtime/generated.env)
if [[ ${PURGE} == true ]]; then
  "${COMPOSE[@]}" --profile configured --profile diagnostics down --volumes
  case "${SCRIPT_DIR}/runtime" in
    */examples/netbird-agent-network/standalone/runtime)
      rm -rf -- "${SCRIPT_DIR}/runtime"
      ;;
  esac
  echo "Containers, networks, volumes, generated credentials, and demo certificates are removed."
else
  "${COMPOSE[@]}" --profile configured --profile diagnostics down
  echo "Containers and networks are removed. Named volumes and runtime credentials are retained."
  echo "Run ./cleanup.sh --volumes for a complete reset."
fi
