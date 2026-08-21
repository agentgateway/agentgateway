#!/usr/bin/env bash
set -euo pipefail

kubectl delete namespace netbird-agent-network --ignore-not-found

echo "The Kubernetes resources are removed."
echo "Remove the example provider, policy, proxy token, and Agent Network"
echo "settings from NetBird if you are keeping the NetBird data volume."
