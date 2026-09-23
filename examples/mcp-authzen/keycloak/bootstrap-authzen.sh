#!/usr/bin/env bash
# Adds the AuthZEN-specific configuration:
# roles, demo users, the two clients (mcp-client for MCP sessions, mcp_proxy as
# the AuthZEN resource server + token-exchange requester), the audience mapper,
# and the payments tool resources/scopes/policies/permissions.
set -euo pipefail

KCADM=/opt/keycloak/bin/kcadm.sh
SERVER=http://keycloak:7080
REALM=mcp

log() { echo "[$(date '+%H:%M:%S')] $*"; }

log "Authenticating to master realm"
$KCADM config credentials \
  --server "$SERVER" \
  --realm master \
  --user admin \
  --password admin

log "Creating realm"
$KCADM create realms -s realm=$REALM -s enabled=true 

log "Creating realm roles"
$KCADM create roles -r $REALM -s name=payments_user    -s 'description=Can list/view accounts and create payments'
$KCADM create roles -r $REALM -s name=payments_manager -s 'description=Can also approve/cancel payments'
$KCADM create roles -r $REALM -s name=payments_admin    -s 'description=Full access'

log "Creating user alice (role: payments_user)"
$KCADM create users -r $REALM \
  -s username=alice -s firstName=Alice -s lastName=User \
  -s email=alice@mcp.local -s emailVerified=true -s enabled=true
$KCADM set-password -r $REALM --username alice --new-password alice123
$KCADM add-roles    -r $REALM --uusername alice --rolename payments_user

log "Creating user bob (role: payments_manager)"
$KCADM create users -r $REALM \
  -s username=bob -s firstName=Bob -s lastName=Manager \
  -s email=bob@mcp.local -s emailVerified=true -s enabled=true
$KCADM set-password -r $REALM --username bob --new-password bob123
$KCADM add-roles    -r $REALM --uusername bob --rolename payments_manager

log "Creating user carol (role: payments_admin)"
$KCADM create users -r $REALM \
  -s username=carol -s firstName=Carol -s lastName=Admin \
  -s email=carol@mcp.local -s emailVerified=true -s enabled=true
$KCADM set-password -r $REALM --username carol --new-password carol123
$KCADM add-roles    -r $REALM --uusername carol --rolename payments_admin

log "Creating client 'mcp-client' (MCP sessions authenticate here)"
MCP_CLIENT_ID=$($KCADM create clients -r $REALM \
  -s clientId=mcp-client \
  -s secret=mcp-client-secret \
  -s enabled=true \
  -s publicClient=false \
  -s directAccessGrantsEnabled=true \
  -s serviceAccountsEnabled=false \
  -i)
log "mcp-client internal id: $MCP_CLIENT_ID"

log "Creating client 'mcp_proxy' (MCP resource server + AuthZEN PDP client + token-exchange requester)"
MCP_PROXY_ID=$($KCADM create clients -r $REALM \
  -s clientId=mcp_proxy \
  -s secret=mcp-proxy-secret \
  -s enabled=true \
  -s publicClient=false \
  -s directAccessGrantsEnabled=false \
  -s serviceAccountsEnabled=true \
  -s authorizationServicesEnabled=true \
  -i)
log "mcp_proxy internal id: $MCP_PROXY_ID"

log "Enabling standard token exchange on mcp_proxy"
$KCADM update "clients/$MCP_PROXY_ID" -r $REALM \
  -s 'attributes={"standard.token.exchange.enabled":"true"}'

log "Adding audience mapper (mcp_proxy) to mcp-client, so MCP session tokens pass jwtAuth's audience check"
$KCADM create "clients/$MCP_CLIENT_ID/protocol-mappers/models" -r $REALM \
  -s name=mcp-proxy-audience \
  -s protocol=openid-connect \
  -s protocolMapper=oidc-audience-mapper \
  -s 'config={"included.client.audience":"mcp_proxy","id.token.claim":"false","access.token.claim":"true"}'

log "Adding audience mapper (mcp_proxy) to mcp_proxy itself, so its self-exchanged tokens carry aud=mcp_proxy"
$KCADM create "clients/$MCP_PROXY_ID/protocol-mappers/models" -r $REALM \
  -s name=mcp-proxy-audience \
  -s protocol=openid-connect \
  -s protocolMapper=oidc-audience-mapper \
  -s 'config={"included.client.audience":"mcp_proxy","id.token.claim":"false","access.token.claim":"true"}'

log "Creating authorization scope 'tools/call'"
$KCADM create "clients/$MCP_PROXY_ID/authz/resource-server/scope" -r $REALM -s 'name=tools/call'

log "Creating tool resources"
for tool in list_accounts get_account create_payment approve_payment cancel_payment; do
  $KCADM create "clients/$MCP_PROXY_ID/authz/resource-server/resource" -r $REALM \
    -s name="$tool" \
    -s type=tool \
    -s 'scopes=[{"name":"tools/call"}]'
done

log "Creating role-based policies"
$KCADM create "clients/$MCP_PROXY_ID/authz/resource-server/policy/role" -r $REALM \
  -s name=payments-user-policy \
  -s 'roles=[{"id":"payments_user"}]'

$KCADM create "clients/$MCP_PROXY_ID/authz/resource-server/policy/role" -r $REALM \
  -s name=payments-manager-policy \
  -s 'roles=[{"id":"payments_manager"}]'

$KCADM create "clients/$MCP_PROXY_ID/authz/resource-server/policy/role" -r $REALM \
  -s name=payments-admin-policy \
  -s 'roles=[{"id":"payments_admin"}]'

log "Creating scope permissions"

# list_accounts, get_account, create_payment: any role
for tool in list_accounts get_account create_payment; do
  $KCADM create "clients/$MCP_PROXY_ID/authz/resource-server/permission/scope" -r $REALM \
    -s name="$tool-permission" \
    -s decisionStrategy=AFFIRMATIVE \
    -s "resources=[\"$tool\"]" \
    -s 'scopes=["tools/call"]' \
    -s 'policies=["payments-user-policy","payments-manager-policy","payments-admin-policy"]'
done

# approve_payment, cancel_payment: manager or admin only
for tool in approve_payment cancel_payment; do
  $KCADM create "clients/$MCP_PROXY_ID/authz/resource-server/permission/scope" -r $REALM \
    -s name="$tool-permission" \
    -s decisionStrategy=AFFIRMATIVE \
    -s "resources=[\"$tool\"]" \
    -s 'scopes=["tools/call"]' \
    -s 'policies=["payments-manager-policy","payments-admin-policy"]'
done

log "Bootstrap complete: realm '$REALM' with alice (payments_user), bob (payments_manager), carol (payments_admin), mcp-client, and mcp_proxy ready"
