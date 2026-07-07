#!/usr/bin/env bash
#
# Leg 2 (consume ID-JAG -> access token) wiring. Run AFTER setup.sh.
#
# Consuming an ID-JAG rides Keycloak's JWT-bearer identity brokering, so this adds:
#   1. a self-referential Identity Provider of type `jwt-authorization-grant` whose issuer
#      == the realm issuer (the ID-JAG's `iss`), validating signatures via the realm JWKS;
#   2. resource-client (the leg-2 consumer) config: a secret, JWT-grant enabled, the IdP in
#      its allowed list, and a per-IdP audience override matching the ID-JAG `aud`;
#   3. a federated-identity link for alice keyed by the ID-JAG `sub` (= alice's user id),
#      so the brokered identity resolves to the local user.
set -euo pipefail

KCADM="${KCADM:-/tmp/kc-idjag/bin/kcadm.sh}"
SERVER="${SERVER:-http://localhost:8480}"
ADMIN_USER="${ADMIN_USER:-admin}"; ADMIN_PASS="${ADMIN_PASS:-admin}"
REALM="${REALM:-idjag-demo}"
RESOURCE_ID="${RESOURCE_ID:-https://resource.idjag.demo}"
RESOURCE_SECRET="${KC_RESOURCE_SECRET:-resource-secret}"
IDP_ALIAS="${IDP_ALIAS:-self-idjag}"
ISSUER="$SERVER/realms/$REALM"
JWKS_URL="$ISSUER/protocol/openid-connect/certs"

$KCADM config credentials --server "$SERVER" --realm master --user "$ADMIN_USER" --password "$ADMIN_PASS"

echo "== self-referential JWT_AUTHORIZATION_GRANT identity provider ($IDP_ALIAS) =="
$KCADM delete "identity-provider/instances/$IDP_ALIAS" -r "$REALM" 2>/dev/null || true
$KCADM create identity-provider/instances -r "$REALM" \
  -s alias="$IDP_ALIAS" \
  -s providerId=jwt-authorization-grant \
  -s enabled=true \
  -s "config.issuer=$ISSUER" \
  -s 'config.useJwksUrl=true' \
  -s "config.jwksUrl=$JWKS_URL" \
  -s 'config.jwtAuthorizationGrantEnabled=true' \
  -s 'config.jwtAuthorizationGrantAssertionSignatureAlg=RS256' \
  -s 'config.jwtAuthorizationGrantAllowedClockSkew=30' \
  -s 'config.jwtAuthorizationGrantMaxAllowedAssertionExpiration=3600'

echo "== resource-client: JWT-grant consumer config =="
# The audience attribute value is itself a JSON string, so merge via -f (kcadm's -s would
# try to parse it as a JSON node). Piped via stdin (-f -) so this works whether $KCADM is a
# host binary or a `docker exec -i ... kcadm.sh` wrapper (no host temp-file dependency).
RID=$($KCADM get clients -r "$REALM" -q clientId=resource-client --fields id --format csv --noquotes)
$KCADM update "clients/$RID" -r "$REALM" -f - <<JSON
{
  "attributes": {
    "idjag.resource.authorization.server.identifier": "$RESOURCE_ID",
    "oauth2.jwt.authorization.grant.enabled": "true",
    "oauth2.jwt.authorization.grant.idp": "$IDP_ALIAS",
    "oauth2.jwt.authorization.grant.audience": "[{\"key\":\"$IDP_ALIAS\",\"value\":\"$RESOURCE_ID\"}]"
  }
}
JSON

echo "== client scopes todos.read/todos.write (assigned to resource-client) =="
# Leg 2 issues a normal access token, so its scopes must be registered client scopes on
# the issuing (resource) client, unlike leg 1 which only filters against the attr allow-list.
for s in todos.read todos.write; do
  $KCADM create client-scopes -r "$REALM" -s name="$s" -s protocol=openid-connect \
    -s 'attributes."include.in.token.scope"=true' \
    -s 'attributes."display.on.consent.screen"=false' 2>/dev/null || true
  SID=$($KCADM get client-scopes -r "$REALM" --fields id,name --format csv --noquotes | awk -F, -v n="$s" '$2==n{print $1}')
  $KCADM update "clients/$RID/optional-client-scopes/$SID" -r "$REALM"
done

echo "== federated-identity link for alice (keyed by ID-JAG sub = alice's user id) =="
ALICE_ID=$($KCADM get users -r "$REALM" -q username=alice --fields id --format csv --noquotes)
$KCADM delete "users/$ALICE_ID/federated-identity/$IDP_ALIAS" -r "$REALM" 2>/dev/null || true
$KCADM create "users/$ALICE_ID/federated-identity/$IDP_ALIAS" -r "$REALM" \
  -s identityProvider="$IDP_ALIAS" \
  -s userId="$ALICE_ID" \
  -s userName=alice
echo "LEG2 SETUP DONE (alice user id: $ALICE_ID)"
