#!/usr/bin/env bash
set -e
if BUILD_GIT_REVISION=$(git rev-parse HEAD 2> /dev/null); then
  if [[ -z "${IGNORE_DIRTY_TREE}" ]] && [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
    BUILD_GIT_REVISION=${BUILD_GIT_REVISION}"-dirty"
  fi
else
  BUILD_GIT_REVISION=unknown
fi

echo "agentgateway.dev.buildVersion=${VERSION:-$BUILD_GIT_REVISION}"
echo "agentgateway.dev.buildGitRevision=${GIT_REVISION:-$BUILD_GIT_REVISION}"
echo "agentgateway.dev.buildOS=$(uname -s)"
echo "agentgateway.dev.buildArch=$(uname -m)"