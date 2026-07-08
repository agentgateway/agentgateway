# RFC 7523 JWT bearer grant (`backendAuth.oauth`)

The `backendAuth.oauth` policy makes the gateway exchange the inbound user
credential for a per-upstream token at an OAuth authorization server before
forwarding the request. This example runs the **RFC 7523 JWT bearer** grant
(`grantType: jwtBearer`): the inbound token is sent as the `assertion`. This is
also the shape Microsoft Entra "on-behalf-of" uses.

For the default **RFC 8693 token exchange** grant, see
[`oauth-rfc8693`](../oauth-rfc8693/README.md); for the hand-written `extAuthz` +
CEL approach, see [`extauthz`](../extauthz/README.md).

## Prerequisites

The commands below run the gateway with `cargo run`; `jq`, `docker`, and
`python3` are also used.

## Infrastructure

RFC 7523 requires the authorization server to **trust the issuer that signed
the assertion**. Keycloak implements this as its *JWT Authorization Grant*
feature — a **preview feature introduced in Keycloak 26.5** — so this example
runs Keycloak 26.5 with `--features=preview` and **two realms**:

- **`idp`** — the "external" provider. A user authenticates here and gets a JWT
  (this is the assertion).
- **`backend-oauth`** — the resource realm. It trusts `idp` via a *JWT
  Authorization Grant* Identity Provider and mints the upstream token.

Everything is baked into the realm import
([`jwtbearer-import/`](./jwtbearer-import/)), so it needs **zero manual
configuration**:

```bash
docker compose -f examples/traffic-token-exchange/jwt-authz-grant/docker-compose.yaml up -d
```

This also starts an **echo upstream** on `:18080` that reflects the request
headers it receives, so you can see the token the gateway forwarded.

Two of the routes point at a tiny **mock token endpoint** on `:7090`
([`mock_token.py`](./mock_token.py)). It logs the exact form body the gateway
POSTs and returns a Bearer token for any request — handy for inspecting
requests against providers you can't run locally (Entra, Okta, Google STS):

```bash
python3 examples/traffic-token-exchange/jwt-authz-grant/mock_token.py &
```

Start the gateway:

```bash
cargo run -- -f examples/traffic-token-exchange/jwt-authz-grant/config.yaml &
```

The gateway **hot-reloads** the config file on save, so you can edit routes and
params and re-`curl` without restarting it.

[`config.yaml`](./config.yaml) has three routes:

| Route | Token endpoint | Shows |
|---|---|---|
| `/jwt-bearer` | mock (`:7090`) | the basic jwt-bearer request shape |
| `/obo` | mock (`:7090`) | the Microsoft Entra on-behalf-of request shape |
| `/jwt-bearer-kc` | real Keycloak (`:7080`) | a full end-to-end exchange |

> **What to look at for the mock routes.** The gateway does a real
> exchange-and-forward on all three routes, but the mock always returns the same
> static placeholder (`mock-jwt-bearer-access-token`) regardless of input — so
> the upstream just receives `Authorization: Bearer mock-jwt-bearer-access-token`,
> which isn't meaningful to decode. For `/jwt-bearer` and `/obo` the thing to
> inspect is the **request the gateway sends to the token endpoint** (the form
> body the mock logs). Only `/jwt-bearer-kc` returns a real signed token you can
> decode at the upstream.

---

## Example 1 — jwt-bearer request shape (mock token endpoint)

Mint a user token from the `backend-oauth` realm:

```bash
SUBJECT_TOKEN="$(curl -s http://localhost:7080/realms/backend-oauth/protocol/openid-connect/token \
  -u initial-client:initial-secret -d grant_type=password \
  -d username=testuser -d password=testpass | jq -r .access_token)"

curl -s http://localhost:3000/jwt-bearer -H "authorization: Bearer $SUBJECT_TOKEN"
```

The mock logs the exact request the gateway sent (this is what to inspect — the
`Bearer mock-jwt-bearer-access-token` the upstream then receives is just the
mock's static placeholder response):

```
form={"grant_type": "urn:ietf:params:oauth:grant-type:jwt-bearer",
      "assertion": "<inbound token>", "audience": "target-client"}
```

Note `grantType: jwtBearer` sends the token as **`assertion`** (not
`subject_token`), and `requestedTokenType` / `actorToken` are rejected at config
load for this grant.

---

## Example 2 — Microsoft Entra on-behalf-of (OBO) shape

Entra OBO is jwt-bearer with provider-specific extras. The equivalent
hand-written request looks like:

```bash
curl -X POST "https://login.microsoftonline.com/<TENANT_ID>/oauth2/v2.0/token" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "client_id=<CLIENT_ID>" \
  -d "client_secret=<CLIENT_SECRET>" \
  -d "grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer" \
  -d "requested_token_use=on_behalf_of" \
  -d "scope=https://graph.microsoft.com/.default" \
  -d "assertion=<USER_ACCESS_TOKEN>"
```

The `/obo` route produces this same request (you can confirm the form body in
the mock's log):

```bash
curl -s http://localhost:3000/obo -H "authorization: Bearer $SUBJECT_TOKEN"
```

Production config, pointed at real Entra:

```yaml
backendAuth:
  oauthTokenExchange:
    host: login.microsoftonline.com:443            # :443 auto-enables backendTLS
    tokenEndpointPath: /<TENANT_ID>/oauth2/v2.0/token
    grantType: jwtBearer
    clientAuth:
      clientId: <CLIENT_ID>
      clientSecret: <CLIENT_SECRET>
      method: clientSecretPost                      # client_id/client_secret in the BODY
    scopes:
    - https://graph.microsoft.com/.default
    additionalParams:
      requested_token_use: '"on_behalf_of"'         # CEL expression -> literal string
```

Field-by-field mapping:

| Microsoft form field | Produced by |
|---|---|
| `grant_type=...jwt-bearer` | `grantType: jwtBearer` |
| `assertion=<user token>` | the inbound bearer (jwt-bearer sends the subject as `assertion`) |
| `scope=...` | `scopes:` (joined into one space-delimited `scope`) |
| `client_id` + `client_secret` (in body) | `clientAuth.method: clientSecretPost` |
| `requested_token_use=on_behalf_of` | `additionalParams` (CEL string literal) |

Gotchas:

- `requested_token_use` is a vendor extension, so it goes in `additionalParams`,
  not a dedicated field. Values are **CEL expressions** — a literal string needs
  inner quotes: `'"on_behalf_of"'`.
- Use `clientSecretPost` to put the client credentials in the body (matching the
  curl above). The default `clientSecretBasic` would send them as an
  `Authorization: Basic` header instead.

---

## Example 3 — full exchange against real Keycloak (two-realm)

A user authenticates against realm `idp` and gets a JWT; the gateway presents
it as the RFC 7523 assertion to realm `backend-oauth`, which trusts `idp` and
mints the upstream token:

```bash
# 1. Get an assertion from realm `idp`:
ASSERTION="$(curl -s http://localhost:7080/realms/idp/protocol/openid-connect/token \
  -u idp-app:idp-secret -d grant_type=password \
  -d username=idpuser -d password=idppass | jq -r .access_token)"

# 2. Present it to the gateway; it does the RFC 7523 exchange against `backend-oauth`:
curl -s http://localhost:3000/jwt-bearer-kc -H "authorization: Bearer $ASSERTION"
# -> HTTP 200; upstream receives a backend-oauth token (aud=target-client)
```

Identity transformation, assertion → exchanged token:

| | `iss` | `aud` | `azp` |
|---|---|---|---|
| assertion (realm `idp`) | `.../realms/idp` | `.../realms/backend-oauth` | `idp-app` |
| exchanged (upstream) | `.../realms/backend-oauth` | `target-client` | `requester-client` |

### Keycloak configuration requirements

All of this is pre-baked into [`jwtbearer-import/`](./jwtbearer-import/). If
you build your own realm instead, these are the settings Keycloak requires
before it will accept the grant:

1. **Preview feature** — start Keycloak with `--features=preview` (enables
   `jwt-authorization-grant:v1`).
2. **Identity Provider** — a `jwt-authorization-grant` type IdP in the resource
   realm, with `issuer` = the assertion's `iss`, `useJwksUrl`/`jwksUrl` +
   `validateSignature` for the assertion's signing keys, and
   `config.jwtAuthorizationGrantEnabled=true`.
3. **Client attributes** on the requesting (confidential) client — exact keys:
   - `oauth2.jwt.authorization.grant.enabled` = `"true"`
   - `oauth2.jwt.authorization.grant.idp` = the IdP alias (`idp-jwt`);
     multi-valued, `##`-separated.
4. **Assertion `aud`** must contain the resource realm's **issuer or token
   endpoint URL**. Here an audience protocol-mapper on `idp-app` adds
   `http://localhost:7080/realms/backend-oauth`.
5. **A pre-existing federated-identity link.** This grant is *non-interactive*:
   Keycloak looks up a local user by `(idp alias, assertion sub)` and fails with
   `User not found` if absent — it does **not** auto-provision. The import pins
   `idpuser`'s id in realm `idp` and gives the matching `backend-oauth` user a
   `federatedIdentities` entry pointing at it.
6. The assertion `sub`/`iss` must be present, `exp` within the IdP's max
   (default 5 min), and (by default) a `jti` for replay protection.

## Cleanup

```bash
pkill -f 'target/debug/agentgateway'    # stop the gateway (started with `cargo run`)
pkill -f mock_token.py                  # stop the mock
docker compose -f examples/traffic-token-exchange/jwt-authz-grant/docker-compose.yaml down
```
