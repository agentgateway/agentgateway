#!/usr/bin/env bash
#
# ID-JAG issuance (leg 1) — two-client setup for a single Keycloak with ID-JAG support.
# Creates a realm, a user, a "requesting" client (the agent/gateway) and a "resource"
# client (the resource authorization server), wiring the client attributes that control
# ID-JAG issuance.
#
# Prereqs: an ID-JAG-capable Keycloak running (see README.md).
set -euo pipefail

KCADM="${KCADM:-/tmp/kc-idjag/bin/kcadm.sh}"   # path to kcadm.sh in the built dist
SERVER="${SERVER:-http://localhost:8480}"
ADMIN_USER="${ADMIN_USER:-admin}"
ADMIN_PASS="${ADMIN_PASS:-admin}"
REALM="${REALM:-idjag-demo}"

# The audience string the agent sends on leg 1. Must equal the resource client's
# `idjag.resource.authorization.server.identifier` attribute (set below). The minted
# ID-JAG's `aud` claim is bound to this value.
RESOURCE_ID="${RESOURCE_ID:-https://resource.idjag.demo}"
# Same env vars the gateway config reads (default to the demo values). Export them once to
# drive both this setup and the gateway: export KC_AGENT_SECRET=... KC_RESOURCE_SECRET=...
KC_AGENT_SECRET="${KC_AGENT_SECRET:-agent-secret}"
KC_RESOURCE_SECRET="${KC_RESOURCE_SECRET:-resource-secret}"

echo "== login as $ADMIN_USER =="
$KCADM config credentials --server "$SERVER" --realm master --user "$ADMIN_USER" --password "$ADMIN_PASS"

echo "== (re)create realm $REALM =="
$KCADM delete "realms/$REALM" 2>/dev/null || true
$KCADM create realms -s realm="$REALM" -s enabled=true

echo "== user alice / alice =="
$KCADM create users -r "$REALM" -s username=alice -s enabled=true \
  -s email=alice@example.com -s firstName=Alice -s lastName=Example
$KCADM set-password -r "$REALM" --username alice --new-password alice

echo "== resource client (represents the resource AS / target API) =="
# The audience identifier attribute is how the provider resolves this client from
# the leg-1 `audience` parameter.
$KCADM create clients -r "$REALM" \
  -s clientId=resource-client \
  -s enabled=true \
  -s publicClient=false \
  -s secret="$KC_RESOURCE_SECRET" \
  -s "attributes.\"idjag.resource.authorization.server.identifier\"=$RESOURCE_ID"

echo "== requesting client (the agent/gateway) =="
# - standard.token.exchange.enabled : required for standard token exchange (v2)
# - idjag.clientid.at.<resourceClientId> : value placed in the ID-JAG `client_id`
#     claim; leg 2 (consume) requires it to equal the client authenticating that leg.
# - idjag.permitted.scopes.at.<resourceClientId> : space-separated allow-list; the
#     leg-1 `scope` is filtered against it.
$KCADM create clients -r "$REALM" \
  -s clientId=agent-client \
  -s enabled=true \
  -s publicClient=false \
  -s directAccessGrantsEnabled=true \
  -s standardFlowEnabled=true \
  -s secret="$KC_AGENT_SECRET" \
  -s 'attributes."standard.token.exchange.enabled"=true' \
  -s 'attributes."idjag.clientid.at.resource-client"=resource-client' \
  -s 'attributes."idjag.permitted.scopes.at.resource-client"=todos.read todos.write'

echo "== agent-client secret =="
AID=$($KCADM get clients -r "$REALM" -q clientId=agent-client --fields id --format csv --noquotes)
$KCADM get "clients/$AID/client-secret" -r "$REALM"
echo "SETUP DONE"
