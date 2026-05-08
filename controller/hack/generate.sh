#!/bin/bash

# Generates deepcopy code, CRDs, and clients for the agentgateway API.
# In this project, clients are mostly used as fakes for testing.

set -o errexit
set -o nounset
set -o pipefail

set -x

readonly ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE}")"/.. && pwd)"
readonly REPO_ROOT="$(cd "${ROOT_DIR}/.." && pwd)"

# Add tools to PATH
export PATH="${REPO_ROOT}/tools:${PATH}"
readonly OUTPUT_PKG=github.com/agentgateway/agentgateway/controller/pkg/client
readonly APIS_PKG=github.com/agentgateway/agentgateway/controller
readonly CLIENTSET_NAME=versioned
readonly CLIENTSET_PKG_NAME=clientset
readonly VERSIONS=( v1alpha1 )

# well known dirs for codegen, should be cleaned before fresh gen
readonly OPENAPI_GEN_DIR=pkg/generated/openapi
readonly APPLY_CFG_DIR=api/applyconfiguration
readonly CLIENT_GEN_DIR=pkg/client
readonly AGENTGATEWAY_CRD_DIR=install/helm/agentgateway-crds/templates
readonly AGENTGATEWAY_MANIFESTS_DIR=install/helm/agentgateway/templates

echo "Generating clientset at ${OUTPUT_PKG}/${CLIENTSET_PKG_NAME} for versions:" "${VERSIONS[@]}"

# Build the client-gen input list for the agentgateway API packages.
API_INPUT_DIRS_SPACE=""
API_INPUT_DIRS_COMMA=""
for VERSION in "${VERSIONS[@]}"; do
  API_INPUT_DIRS_SPACE+="${APIS_PKG}/api/${VERSION}/agentgateway "
  API_INPUT_DIRS_COMMA+="${APIS_PKG}/api/${VERSION}/agentgateway,"
done
API_INPUT_DIRS_SPACE="${API_INPUT_DIRS_SPACE%,}" # drop trailing space
API_INPUT_DIRS_COMMA="${API_INPUT_DIRS_COMMA%,}" # drop trailing comma

(cd "${REPO_ROOT}" && register-gen --output-file zz_generated.register.go ${API_INPUT_DIRS_SPACE})

# replace version since kubebuilder will use the package name
for VERSION in "${VERSIONS[@]}"; do
  sed -i.bak -E "s/(Version: )\"agentgateway\"/\\1\"${VERSION}\"/" "${ROOT_DIR}/api/${VERSION}/agentgateway/zz_generated.register.go"
  rm -f "${ROOT_DIR}/api/${VERSION}/agentgateway/zz_generated.register.go.bak"
done

# Generate objects and RBAC with stock controller-gen.
(cd "${REPO_ROOT}" && controller-gen object paths="${APIS_PKG}/api/${VERSION}/agentgateway" paths="${APIS_PKG}/api/${VERSION}/shared")

# Generate CRDs with custom kubebuilder validation markers.
(cd "${REPO_ROOT}" && go run ./controller/hack/crdgen \
    --max-desc-len 50000 \
    --output-dir "${ROOT_DIR}/${AGENTGATEWAY_CRD_DIR}" \
    --path "${APIS_PKG}/api/${VERSION}/agentgateway" \
    --path "${APIS_PKG}/api/${VERSION}/shared")

# throw away
new_report="$(mktemp -t "$(basename "$0").api_violations.XXXXXX")"

(cd "${REPO_ROOT}" && client-gen \
  --clientset-name "versioned" \
  --input-base "${APIS_PKG}" \
  --input "${API_INPUT_DIRS_COMMA//${APIS_PKG}\//}" \
  --output-dir "${ROOT_DIR}/${CLIENT_GEN_DIR}/${CLIENTSET_PKG_NAME}" \
  --output-pkg "${OUTPUT_PKG}/${CLIENTSET_PKG_NAME}" \
  --plural-exceptions "AgentgatewayParameters:AgentgatewayParameters")

go generate ${ROOT_DIR}/pkg/...
