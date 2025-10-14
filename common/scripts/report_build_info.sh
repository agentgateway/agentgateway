#!/usr/bin/env bash
if BUILD_GIT_REVISION=$(git rev-parse HEAD 2> /dev/null); then
  if [[ -z "${IGNORE_DIRTY_TREE}" ]] && [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
    BUILD_GIT_REVISION=${BUILD_GIT_REVISION}"-dirty"
  fi
else
  BUILD_GIT_REVISION=unknown
fi

# Check for local changes
tree_status="Clean"
if [[ -z "${IGNORE_DIRTY_TREE}" ]] && ! git diff-index --quiet HEAD --; then
  tree_status="Modified"
fi

GIT_DESCRIBE_TAG=$(git describe --tags --always)
HUB=${HUB:-"ghcr.io/kagent-dev/mcp-relay"}

# used by common/scripts/gobuild.sh
echo "agentgateway.dev.buildVersion=${VERSION:-$BUILD_GIT_REVISION}"
echo "agentgateway.dev.buildGitRevision=${BUILD_GIT_REVISION}"
echo "agentgateway.dev.buildOS=$(uname -s)"
echo "agentgateway.dev.buildArch=$(uname -m)"
