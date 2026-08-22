#!/usr/bin/env bash
# Run kcadm INSIDE the running container, so you don't need a Keycloak install on the host.
# Use it as the KCADM for the setup scripts, with the in-container server URL:
#
#   KCADM="$PWD/docker/kcadm.sh" SERVER=http://localhost:8080 ../configure-keycloak.sh
#
#   KC_CONTAINER   container name (default: kc-idjag)
exec docker exec -i "${KC_CONTAINER:-kc-idjag}" /opt/keycloak/bin/kcadm.sh "$@"
