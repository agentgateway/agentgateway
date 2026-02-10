#!/bin/bash

# Based of of gateway-api codegen (https://github.com/kubernetes-sigs/gateway-api/blob/main/hack/update-codegen.sh)
# generate deep copy and clients for our api.
# In this project, clients mostly used as fakes for testing.

set -o errexit
set -o nounset
set -o pipefail

set -x

readonly ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE}")"/.. && pwd)"
readonly OUTPUT_PKG=github.com/kgateway-dev/kgateway/v2/pkg/client
readonly APIS_PKG=github.com/kgateway-dev/kgateway/v2
readonly CLIENTSET_NAME=versioned
readonly CLIENTSET_PKG_NAME=clientset
readonly VERSIONS=( v1alpha1 )

# well known dirs for codegen, should be cleaned before fresh gen
readonly OPENAPI_GEN_DIR=pkg/generated/openapi
readonly APPLY_CFG_DIR=api/applyconfiguration
readonly CLIENT_GEN_DIR=pkg/client
readonly KGATEWAY_CRD_DIR=install/helm/kgateway-crds/templates
readonly KGATEWAY_MANIFESTS_DIR=install/helm/kgateway/templates
readonly AGENTGATEWAY_CRD_DIR=install/helm/agentgateway-crds/templates
readonly AGENTGATEWAY_MANIFESTS_DIR=install/helm/agentgateway/templates

echo "Generating clientset at ${OUTPUT_PKG}/${CLIENTSET_PKG_NAME} for versions:" "${VERSIONS[@]}"

# Build combined input list for BOTH groups so a single clientset includes both
API_INPUT_DIRS_SPACE=""
API_INPUT_DIRS_COMMA=""
for VERSION in "${VERSIONS[@]}"; do
  API_INPUT_DIRS_SPACE+="${APIS_PKG}/api/${VERSION}/agentgateway "
  API_INPUT_DIRS_COMMA+="${APIS_PKG}/api/${VERSION}/agentgateway,"
done
API_INPUT_DIRS_SPACE="${API_INPUT_DIRS_SPACE%,}" # drop trailing space
API_INPUT_DIRS_COMMA="${API_INPUT_DIRS_COMMA%,}" # drop trailing comma

go tool register-gen --output-file zz_generated.register.go ${API_INPUT_DIRS_SPACE}

# replace version since kubebuilder will use the package name
for VERSION in "${VERSIONS[@]}"; do
  sed -i.bak -E "s/(Version: )\"agentgateway\"/\\1\"${VERSION}\"/" "${ROOT_DIR}/api/${VERSION}/agentgateway/zz_generated.register.go"
  rm -f "${ROOT_DIR}/api/${VERSION}/agentgateway/zz_generated.register.go.bak"
done

# Generate agentgateway CRDs and RBAC
go tool controller-gen crd:maxDescLen=50000 object rbac:roleName=agentgateway paths="${APIS_PKG}/api/${VERSION}/agentgateway" paths="${APIS_PKG}/api/${VERSION}/shared" \
    output:crd:artifacts:config=${ROOT_DIR}/${AGENTGATEWAY_CRD_DIR} output:rbac:artifacts:config=${ROOT_DIR}/${AGENTGATEWAY_MANIFESTS_DIR}
# Template the ClusterRole name to include the namespace
if [[ "$OSTYPE" == "darwin"* ]]; then
  # On macOS, prefer gsed (GNU sed) if available
  if command -v gsed &> /dev/null; then
    gsed -i 's/name: agentgateway/name: agentgateway-{{ .Release.Namespace }}/g' "${ROOT_DIR}/${AGENTGATEWAY_MANIFESTS_DIR}/role.yaml"
  else
    # Fallback to macOS's native sed
    sed -i '' 's/name: agentgateway/name: agentgateway-{{ .Release.Namespace }}/g' "${ROOT_DIR}/${AGENTGATEWAY_MANIFESTS_DIR}/role.yaml"
  fi
else
  # For other OSes like Linux
  sed -i 's/name: agentgateway/name: agentgateway-{{ .Release.Namespace }}/g' "${ROOT_DIR}/${AGENTGATEWAY_MANIFESTS_DIR}/role.yaml"
fi


# throw away
new_report="$(mktemp -t "$(basename "$0").api_violations.XXXXXX")"

go tool client-gen \
  --clientset-name "versioned" \
  --input-base "${APIS_PKG}" \
  --input "${API_INPUT_DIRS_COMMA//${APIS_PKG}\//}" \
  --output-dir "${ROOT_DIR}/${CLIENT_GEN_DIR}/${CLIENTSET_PKG_NAME}" \
  --output-pkg "${OUTPUT_PKG}/${CLIENTSET_PKG_NAME}" \
  --plural-exceptions "AgentgatewayParameters:AgentgatewayParameters"

go generate ${ROOT_DIR}/internal/...
go generate ${ROOT_DIR}/pkg/...

