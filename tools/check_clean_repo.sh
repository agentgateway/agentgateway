#!/bin/bash
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

if git -C "$ROOT_DIR" rev-parse --show-toplevel >/dev/null 2>&1; then
  if [[ -n $(git -C "$ROOT_DIR" status --porcelain) ]]; then
    git -C "$ROOT_DIR" status
    git -C "$ROOT_DIR" diff
    echo "ERROR: Some files need to be updated, please run 'make gen' and include any changed files in your PR"
    exit 1
  fi
elif jj -R "$ROOT_DIR" workspace root >/dev/null 2>&1; then
  if [[ -n "$(cd "$ROOT_DIR" && jj diff --name-only)" ]]; then
    (cd "$ROOT_DIR" && jj status)
    (cd "$ROOT_DIR" && jj diff --git)
    echo "ERROR: Some files need to be updated, please run 'make gen' and include any changed files in your PR"
    exit 1
  fi
fi
