#!/usr/bin/env bash
set -euo pipefail

helm uninstall openshell --namespace openshell --ignore-not-found
kubectl delete namespace openshell --ignore-not-found

echo "The OpenShell example resources are removed."
