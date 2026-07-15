#!/usr/bin/env bash
#
# One-shot Keycloak configuration for the ID-JAG demo: runs setup.sh (realm, user, the two
# clients / leg 1) then setup-leg2.sh (self-IdP, consumer config, federated link, scopes).
# Requires Keycloak already running (see start-keycloak.sh).
#   KCADM  path to kcadm.sh in the running dist (default: /tmp/kc-idjag/bin/kcadm.sh)
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
export KCADM="${KCADM:-/tmp/kc-idjag/bin/kcadm.sh}"

"$DIR/setup.sh"
"$DIR/setup-leg2.sh"

echo
echo "Keycloak configured:"
echo "  realm         : idjag-demo"
echo "  user          : alice / alice"
echo "  agent-client  : agent-secret     (requesting app; mints ID-JAG via token exchange)"
echo "  resource-client: resource-secret (resource AS; consumes ID-JAG via jwt-bearer)"
echo "  IdP           : self-idjag       (self-referential JWT_AUTHORIZATION_GRANT)"
