#!/usr/bin/env bash
#
# Extracts (first run) and starts the ID-JAG-enabled Keycloak build in the foreground.
# Override paths/ports with env vars:
#   KC_DIST  path to the built keycloak-*.tar.gz  (default: keycloak checkout dist)
#   KC_HOME  where to extract/run it              (default: /tmp/kc-idjag)
#   KC_PORT  HTTP port                            (default: 8480)
set -euo pipefail

KC_DIST="${KC_DIST:-$HOME/java/keycloak/quarkus/dist/target/keycloak-999.0.0-SNAPSHOT.tar.gz}"
KC_HOME="${KC_HOME:-/tmp/kc-idjag}"
KC_PORT="${KC_PORT:-8480}"

if [ ! -x "$KC_HOME/bin/kc.sh" ]; then
  [ -f "$KC_DIST" ] || { echo "ERROR: dist not found: $KC_DIST (build it first, see README.md)"; exit 1; }
  echo "Extracting $KC_DIST -> $KC_HOME"
  mkdir -p "$KC_HOME"
  tar -xzf "$KC_DIST" -C "$KC_HOME" --strip-components=1
fi

echo "Starting Keycloak on :$KC_PORT (admin/admin) with feature identity-assertion-jwt ..."
exec env KC_BOOTSTRAP_ADMIN_USERNAME=admin KC_BOOTSTRAP_ADMIN_PASSWORD=admin \
  "$KC_HOME/bin/kc.sh" start-dev --features=identity-assertion-jwt --http-port="$KC_PORT"
