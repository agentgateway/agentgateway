# ID-JAG / Cross App Access against the xaa.dev playground

An agentgateway Cross App Access demo / config that uses the public **xaa.dev** sandbox. 

Agentgateway is the requesting app; xaa.dev's **IdenX** IdP issues the ID-JAG, its resource authorization server issues the access token, and it hosts the protected **Todo0** API.

The fixed xaa.dev endpoints are:

| | value |
|---|---|
| IdP (issuer) | `https://idp.xaa.dev` (token `/token`, jwks `/jwks`) |
| Resource authz server | `https://auth.resource.xaa.dev` (token `/token`) |
| Resource API | `https://api.resource.xaa.dev` |
| Scope | `todos.read` |

## 1. Register your requesting app

Go to <https://xaa.dev/developer/register>, enter any email (no account is created; return with
the **same** email later to manage or delete your app), then **Register New App**:

- **Redirect URI**: `http://localhost:8500/callback`
- **Resource Connection**: add **Todo0 Resource App** (scope `todos.read`)

You'll get two sets of credentials (see the app card's **Integration Guide**):

- a **requesting-app** client — `client_id` + secret (used for leg 1, the IdP)
- a **resource** client — `client_id` (looks like `<your-client-id>-at-todo0`) + secret (leg 2)

Put your client ids into [`gateway.yaml`](gateway.yaml) (`identityProvider.clientAuth.clientId`,
`resourceAuthorizationServer.clientAuth.clientId`, and the `jwtAuth.audiences` entry) — replacing
the `REPLACE_WITH_YOUR_CLIENT_ID` placeholders.

> Tip: keep your filled-in config as **`gateway.local.yaml`** (git-ignored) so your real client
> ids stay out of version control: `cp gateway.yaml gateway.local.yaml`, edit that, and run with
> `-f ./gateway.local.yaml`.

## 2. Fill in the two client secrets

Reveal + copy each secret on the app card into a file. **Use `printf` (no trailing newline)** —
a trailing `\n` will cause `401 invalid_client`:

```bash
cd examples/traffic-cross-app-access/xaa-dev
printf '%s' 'secret_...' > secrets/idp-client-secret        # requesting-app (IdP) secret
printf '%s' 'secret_...' > secrets/resource-client-secret   # resource client secret
```
(`secrets/` is git-ignored.)

## 3. Get a user ID token from IdenX

IdenX uses interactive Auth Code + PKCE login, so run the helper and follow the prompts (sign in
with any email; the demo prompts for a 6-digit code — enter any 6 digits, e.g. `123456`):

```bash
./get-id-token.sh
# prints an authorize URL -> sign in -> copy the `code` from the redirected localhost:8500 URL
# -> prints an ID token
export ID_TOKEN='<the printed id_token>'
```

## 4. Start agentgateway

Pick any option below — all bind **`:3031`** (so it runs alongside the Keycloak demo's `:3030`).
Because `gateway.yaml` references secrets as `./secrets/...`, run it **from this directory** — or
point at a config with absolute paths (e.g. your `gateway.local.yaml`) to run from anywhere.

**a) From source with `cargo`** (from the agentgateway repo root):
```bash
# gateway.local.yaml uses absolute secret paths, so it resolves from the repo root
cargo run --release --bin agentgateway -- -f examples/traffic-cross-app-access/xaa-dev/gateway.local.yaml
```

**b) Released binary — [`v1.4.0-alpha.1`](https://github.com/agentgateway/agentgateway/releases/tag/v1.4.0-alpha.1) or later** (this release includes Cross App Access):
```bash
# download the binary for your platform, then run it from this directory.
# available: agentgateway-{darwin-arm64, linux-amd64, linux-arm64, windows-amd64.exe}
curl -L -o agentgateway \
  https://github.com/agentgateway/agentgateway/releases/download/v1.4.0-alpha.1/agentgateway-darwin-arm64
chmod +x agentgateway
./agentgateway -f ./gateway.yaml
```
> The v1.4.0-alpha.1 macOS binary is **arm64** (Apple Silicon) only; on Intel macOS use option (a) or (b).

## 5. Drive the flow

```bash
curl -s http://localhost:3031/ -H "Authorization: Bearer $ID_TOKEN"
```

The gateway validates the IdenX ID token, exchanges it for an ID-JAG at `idp.xaa.dev`, exchanges
that for an access token at `auth.resource.xaa.dev`, and calls the Todo0 API at
`api.resource.xaa.dev` with the access token — returning the protected todo data.

## Gotchas

- **TLS on every hop.** All xaa.dev endpoints are HTTPS on `:443`, but agentgateway does not
  auto-enable TLS for `:443` here — you must set `backendTLS: {}` in **three** places (already in
  `gateway.yaml`): on `identityProvider`, on `resourceAuthorizationServer`, and as a route-level
  policy for the resource API backend. Missing any one yields nginx's
  *"The plain HTTP request was sent to HTTPS port"*.
- **Secret files must have no trailing newline** (use `printf`, not `echo`) — otherwise leg 1
  fails `401 invalid_client`.
- **Two different clients / secrets.** Leg 1 uses the requesting-app client; leg 2 uses the
  resource client (`...-at-todo0`). Mixing them up gives `401 invalid_client` on leg 2.
- **`crossAppAccess.resources` is set here** (unlike the Keycloak demo): xaa.dev expects the
  RFC 8707 `resource` on the token-exchange leg, which agentgateway sends to the identity provider.
- Manage or delete the registered app at <https://xaa.dev/developer/register>; debug the live
  flow (service configs + event log) at <https://xaa.dev> → **Inspect**.

## Appendix — the whole flow as raw `curl` (the gateway's manual equivalent)

These reproduce, by hand, exactly what agentgateway's `crossAppAccess` policy does against
xaa.dev. Run from this directory. Set `CLIENT_ID`/`RESOURCE_CLIENT_ID` to your registered app's
values. Tokens are short-lived (the ID-JAG lasts ~300s), so run the chain promptly after step 0.

```bash
IDP=https://idp.xaa.dev
RS=https://auth.resource.xaa.dev
API=https://api.resource.xaa.dev
CLIENT_ID=<your-requesting-client-id>
RESOURCE_CLIENT_ID=<your-requesting-client-id>-at-todo0
IDP_SECRET=$(cat secrets/idp-client-secret)
RESOURCE_SECRET=$(cat secrets/resource-client-secret)
```

### 0. Get the user's ID token (interactive, Auth Code + PKCE)

Easiest: `./get-id-token.sh`. The raw equivalent it runs:

```bash
VERIFIER=$(openssl rand -base64 60 | tr -d '\n=+/' | cut -c1-64)
CHALLENGE=$(printf '%s' "$VERIFIER" | openssl dgst -binary -sha256 | openssl base64 | tr '+/' '-_' | tr -d '=\n')

# open this URL in a browser, sign in, copy the ?code=... from the (dead) localhost:8500 redirect
echo "$IDP/authorize?response_type=code&scope=openid%20email&redirect_uri=http://localhost:8500/callback&client_id=$CLIENT_ID&code_challenge_method=S256&code_challenge=$CHALLENGE&state=xyz"

CODE=...   # paste the code from the redirect
ID_TOKEN=$(curl -s -X POST "$IDP/token" \
  --data-urlencode "grant_type=authorization_code" \
  --data-urlencode "code=$CODE" \
  --data-urlencode "redirect_uri=http://localhost:8500/callback" \
  --data-urlencode "code_verifier=$VERIFIER" \
  --data-urlencode "client_id=$CLIENT_ID" \
  --data-urlencode "client_secret=$IDP_SECRET" \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["id_token"])')
```

### 1. Leg 1 — token exchange → ID-JAG  (RFC 8693, at the IdP)

```bash
IDJAG=$(curl -s -X POST "$IDP/token" \
  --data-urlencode "grant_type=urn:ietf:params:oauth:grant-type:token-exchange" \
  --data-urlencode "requested_token_type=urn:ietf:params:oauth:token-type:id-jag" \
  --data-urlencode "subject_token_type=urn:ietf:params:oauth:token-type:id_token" \
  --data-urlencode "subject_token=$ID_TOKEN" \
  --data-urlencode "audience=$RS" \
  --data-urlencode "resource=$API" \
  --data-urlencode "scope=todos.read" \
  --data-urlencode "client_id=$CLIENT_ID" \
  --data-urlencode "client_secret=$IDP_SECRET" \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["access_token"])')
# -> a JWT with header typ "oauth-id-jag+jwt", aud=$RS, resource=$API, scope=todos.read
```

### 2. Leg 2 — jwt-bearer → access token  (RFC 7523, at the resource authz server)

Authenticated as the **resource** client (`...-at-todo0`); sends only the assertion + scope.

```bash
ACCESS_TOKEN=$(curl -s -X POST "$RS/token" \
  --data-urlencode "grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer" \
  --data-urlencode "assertion=$IDJAG" \
  --data-urlencode "scope=todos.read" \
  --data-urlencode "client_id=$RESOURCE_CLIENT_ID" \
  --data-urlencode "client_secret=$RESOURCE_SECRET" \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["access_token"])')
```

### 3. Leg 4 — call the protected API with the access token  (RFC 6750)

```bash
curl -s "$API/" -H "Authorization: Bearer $ACCESS_TOKEN"
```

### How this maps to `gateway.yaml`

| curl step | `crossAppAccess` field |
|---|---|
| Leg 1 endpoint + `client_id`/`client_secret` | `identityProvider` (`idp.xaa.dev`, requesting client) |
| `audience` / `resource` / `scope` | `audience` / `resources` / `scopes` |
| Leg 2 endpoint + resource `client_id`/`client_secret` | `resourceAuthorizationServer` (`auth.resource.xaa.dev`, `...-at-todo0` client) |
| Leg 4 target | route `backends: - host: api.resource.xaa.dev:443` (+ route-level `backendTLS`) |
| the inbound `$ID_TOKEN` | validated by `jwtAuth`, then reused as the exchange subject |
