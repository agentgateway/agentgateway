#!/usr/bin/env bash
#
# Build the ID-JAG Keycloak container image from the locally-built dist.
# Copies the dist tarball into the build context (ADD needs it local), builds, cleans up.
#
#   KC_VERSION    dist version                (default: 999.0.0-SNAPSHOT)
#   KC_DIST_SRC   path to the built tarball   (default: keycloak checkout dist)
#   IMAGE         image tag                   (default: keycloak-idjag:<version>)
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"

KC_VERSION="${KC_VERSION:-999.0.0-SNAPSHOT}"
KC_DIST_SRC="${KC_DIST_SRC:-$HOME/java/keycloak/quarkus/dist/target/keycloak-${KC_VERSION}.tar.gz}"
IMAGE="${IMAGE:-keycloak-idjag:${KC_VERSION}}"
TARBALL="keycloak-${KC_VERSION}.tar.gz"

[ -f "$KC_DIST_SRC" ] || { echo "ERROR: dist not found: $KC_DIST_SRC (build Keycloak first)"; exit 1; }

echo "==> staging $TARBALL into build context"
cp "$KC_DIST_SRC" "$DIR/$TARBALL"
trap 'rm -f "$DIR/$TARBALL"' EXIT

echo "==> docker build -t $IMAGE"
docker build -t "$IMAGE" \
  --build-arg KEYCLOAK_VERSION="$KC_VERSION" \
  --build-arg KEYCLOAK_DIST="$TARBALL" \
  "$DIR"

echo "==> built $IMAGE"
