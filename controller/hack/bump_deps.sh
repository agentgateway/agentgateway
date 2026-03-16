#!/usr/bin/env bash
set -euo pipefail
[ $# -eq 2 ] || { echo "usage: $0 {gie|gtw} REF" >&2; exit 2; }
cd "$(dirname "${BASH_SOURCE[0]}")/.."
case "$1" in
  gtw) kubectl kustomize "https://github.com/kubernetes-sigs/gateway-api/config/crd/experimental?ref=$2" > pkg/kgateway/crds/gateway-crds.yaml ;;
  gie) kubectl kustomize "https://github.com/kubernetes-sigs/gateway-api-inference-extension/config/crd?ref=$2" > pkg/kgateway/crds/inference-crds.yaml ;;
  *) echo "usage: $0 {gie|gtw} REF" >&2; exit 2 ;;
esac
