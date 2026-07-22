# RFC 8693 token exchange (`backendAuth.oauth`)

The `backendAuth.oauth` policy makes the gateway exchange the inbound user
credential for a per-upstream token at an OAuth authorization server before
forwarding the request. This example runs the default grant — **RFC 8693 token
exchange** — against Keycloak: the inbound token is sent as `subject_token`,
and the response token is forwarded upstream.

The sibling [`extauthz`](../extauthz/README.md) example reaches the same goal a
different way — building the token request by hand with `extAuthz` + CEL. Use
`backendAuth.oauth` when you want the built-in policy rather than a
hand-written request. For the **RFC 7523 JWT bearer** grant (`grantType:
jwtBearer`, including Microsoft Entra on-behalf-of), see
[`jwt-authz-grant`](../jwt-authz-grant/README.md).

## Prerequisites

The commands below run the gateway with `cargo run`; `jq` and `docker` are also
used.

## Infrastructure

```bash
docker compose -f examples/traffic-token-exchange/oauth-rfc8693/docker-compose.yaml up -d
```

This starts:

- **Keycloak** on `:7080`, realm `backend-oauth`, pre-seeded with clients
  `initial-client`, `requester-client`, `target-client` and user
  `testuser` / `testpass`.
- An **echo upstream** on `:18080` that reflects the request headers it
  receives, so you can see the token the gateway forwarded.

The gateway **hot-reloads** the config file on save, so you can edit routes and
params and re-`curl` without restarting it.

## Run the example

The route reads the inbound credential from the `Authorization: Bearer` header
by default. Mint a user token from Keycloak:

```bash
SUBJECT_TOKEN="$(curl -s http://localhost:7080/realms/backend-oauth/protocol/openid-connect/token \
  -u initial-client:initial-secret -d grant_type=password \
  -d username=testuser -d password=testpass | jq -r .access_token)"
```

(Tokens expire; re-mint if you come back later or restart Keycloak.)

Start the gateway and send a request:

```bash
cargo run -- -f examples/traffic-token-exchange/oauth-rfc8693/config.yaml &

curl -s http://localhost:3000/exchange -H "authorization: Bearer $SUBJECT_TOKEN"
```

The gateway POSTs to Keycloak's token endpoint as the confidential client
`requester-client` (`clientSecretBasic`), exchanging the user's token for one
scoped to `audience=target-client`, and forwards *that* token upstream.
For RFC 8693 token exchange, the gateway sends
`requested_token_type=urn:ietf:params:oauth:token-type:access_token` unless you
configure a different `requestedTokenType`.

Decoded result (inbound vs forwarded):

| | `aud` | `azp` | `sub` |
|---|---|---|---|
| inbound (client → gateway) | `requester-client` | `initial-client` | — |
| exchanged (gateway → upstream) | `target-client` | `requester-client` | `<user id>` |

Config ([`config.yaml`](./config.yaml)):

```yaml
backendAuth:
  oauthTokenExchange:
    host: localhost:7080
    path: /realms/backend-oauth/protocol/openid-connect/token
    clientAuth:
      clientId: requester-client
      clientSecret: requester-secret
      method: clientSecretBasic
    audiences:
    - target-client
```

## Other knobs to try

Edit `config.yaml` and re-`curl` (hot-reloaded):

- **Output location** — `authorizationLocation: { header: { name: x-upstream-auth } }`
  puts the exchanged token in a custom header instead of `Authorization`.
- **Client auth method** — flip `clientSecretBasic` ↔ `clientSecretPost` and watch
  the credentials move between the `Authorization` header and the form body.
- **Caching** — `cache: { inMemory: { maxEntries: 0 } }` disables caching so every
  request hits the token endpoint; otherwise tokens are cached per request
  (subject + type + actor + extra params), TTL capped by the subject JWT `exp`.
- **Delegation** — add `actorToken: { source: { header: { name: x-actor-token } }, tokenType: ... }`
  to emit `actor_token` / `actor_token_type` (RFC 8693 delegation; the source
  must be set explicitly).
- **Custom subject source** — `subjectToken: { source: { ... }, tokenType: ... }`
  to read the subject from a non-default header, query param, cookie, or CEL
  expression.

## Cleanup

```bash
pkill -f 'target/debug/agentgateway'    # stop the gateway (started with `cargo run`)
docker compose -f examples/traffic-token-exchange/oauth-rfc8693/docker-compose.yaml down
```
