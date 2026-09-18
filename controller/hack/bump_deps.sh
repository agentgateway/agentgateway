#!/usr/bin/env bash
set -euo pipefail
[ $# -eq 2 ] || { echo "usage: $0 {gie|gtw} REF" >&2; exit 2; }
cd "$(dirname "${BASH_SOURCE[0]}")/.."
case "$1" in
  gtw) kubectl kustomize "https://github.com/kubernetes-sigs/gateway-api/config/crd/experimental?ref=$2" > pkg/kgateway/crds/gateway-crds.yaml ;;
  gie)
    go get "sigs.k8s.io/gateway-api-inference-extension@$2" \
      "sigs.k8s.io/gateway-api-inference-extension/conformance@$2"
    go mod tidy
    ;;
  *) echo "usage: $0 {gie|gtw} REF" >&2; exit 2 ;;
esac
