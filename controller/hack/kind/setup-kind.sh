#!/bin/bash -ex

# 0. Assign default values to some of our environment variables
# Get directory this script is located in to access script local files
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
# The name of the kind cluster to deploy to
CLUSTER_NAME="${CLUSTER_NAME:-kind}"
# The version of the Node Docker image to use for booting the cluster: https://hub.docker.com/r/kindest/node/tags
# This version should stay in sync with `../../Makefile`.
CLUSTER_NODE_VERSION="${CLUSTER_NODE_VERSION:-v1.35.0@sha256:452d707d4862f52530247495d180205e029056831160e22870e37e3f6c1ac31f}"
# The version used to tag images
VERSION="${VERSION:-v1.0.0-ci1}"
# Skip building docker images if we are testing a released version
SKIP_DOCKER="${SKIP_DOCKER:-false}"
# Stop after creating the kind cluster
JUST_KIND="${JUST_KIND:-false}"
# The version of the k8s gateway api conformance tests to run.
CONFORMANCE_VERSION="${CONFORMANCE_VERSION:-$(go list -m sigs.k8s.io/gateway-api | awk '{print $2}')}"
# The channel of the k8s gateway api conformance tests to run.
CONFORMANCE_CHANNEL="${CONFORMANCE_CHANNEL:-"experimental"}"
# The version of the k8s gateway api inference extension CRDs to install. Managed by `make bump-gie`.
GIE_CRD_VERSION="v1.1.0"
# The kind CLI to use. Defaults to the latest version from the kind repo.
KIND="${KIND:-go tool kind}"
# The helm CLI to use. Defaults to the latest version from the helm repo.
HELM="${HELM:-go tool helm}"
# If true, use localstack for lambda functions
LOCALSTACK="${LOCALSTACK:-false}"
# Registry cache reference for envoyinit Docker build (optional)
ENVOYINIT_CACHE_REF="${ENVOYINIT_CACHE_REF:-}"
# If true, build and load agentgateway images instead of envoy
AGENTGATEWAY="${AGENTGATEWAY:-false}"

# Export the variables so they are available in the environment
export VERSION CLUSTER_NAME ENVOYINIT_CACHE_REF

function create_kind_cluster_or_skip() {
  activeClusters=$($KIND get clusters)

  # if the kind cluster exists already, return
  if [[ "$activeClusters" =~ .*"$CLUSTER_NAME".* ]]; then
    echo "cluster exists, skipping cluster creation"
    return
  fi

  echo "creating cluster ${CLUSTER_NAME}"
  $KIND create cluster \
    --name "$CLUSTER_NAME" \
    --image "kindest/node:$CLUSTER_NODE_VERSION"
  echo "Finished setting up cluster $CLUSTER_NAME"

  # so that you can just build the kind image alone if needed
  if [[ $JUST_KIND == 'true' ]]; then
    echo "JUST_KIND=true, not building images"
    exit
  fi
}

function create_and_setup() {
  create_kind_cluster_or_skip

  # 5. Apply the Kubernetes Gateway API CRDs
  # Use release URL for version tags (faster, avoiding 27s timeout), but use
  # kustomize for commit SHAs -- this is needed to run conformance tests from
  # main when either dependency references a pseudo-version instead of a
  # release.
  if [[ $CONFORMANCE_VERSION =~ ^v[0-9] ]]; then
    kubectl apply --server-side -f "https://github.com/kubernetes-sigs/gateway-api/releases/download/$CONFORMANCE_VERSION/$CONFORMANCE_CHANNEL-install.yaml"
  elif [[ $CONFORMANCE_CHANNEL == "standard" ]]; then
    kubectl apply --server-side --kustomize "https://github.com/kubernetes-sigs/gateway-api/config/crd?ref=$CONFORMANCE_VERSION"
  else
    kubectl apply --server-side --kustomize "https://github.com/kubernetes-sigs/gateway-api/config/crd/$CONFORMANCE_CHANNEL?ref=$CONFORMANCE_VERSION"
  fi

  # 6. Apply the Kubernetes Gateway API Inference Extension CRDs
  make gie-crds

  # TODO: extract metallb install to a diff function so we can let it run in the background
  . $SCRIPT_DIR/setup-metalllb-on-kind.sh
}

# 1. Create a kind cluster (or skip creation if a cluster with name=CLUSTER_NAME already exists)
# This config is roughly based on: https://kind.sigs.k8s.io/docs/user/ingress/
create_and_setup

if [[ $SKIP_DOCKER == 'true' ]]; then
  # TODO(tim): refactor the Makefile & CI scripts so we're loading local
  # charts to real helm repos, and then we can remove this block.
  echo "SKIP_DOCKER=true, not building images or chart"
else
  # 2. Make all the docker images and load them to the kind cluster
  if [[ $AGENTGATEWAY == 'true' ]]; then
    # Skip expensive envoy build
    VERSION=$VERSION CLUSTER_NAME=$CLUSTER_NAME make kind-build-and-load-agentgateway-controller kind-build-and-load-dummy-idp
  else
    VERSION=$VERSION CLUSTER_NAME=$CLUSTER_NAME make kind-build-and-load kind-build-and-load-dummy-idp
  fi

  VERSION=$VERSION make package-kgateway-charts package-agentgateway-charts
fi

# 7. Setup localstack
if [[ $LOCALSTACK == "true" ]]; then
  echo "Setting up localstack"
  . $SCRIPT_DIR/setup-localstack.sh
fi
