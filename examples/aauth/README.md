# AAuth (HTTP Message Signing)

This example shows agentgateway verifying inbound requests signed with AAuth — a per-request
HTTP message signature protocol designed for cross-organization agent traffic. Each request
carries three headers:

- `Signature-Input`: which components were signed (RFC 9421)
- `Signature`: the Ed25519 signature bytes
- `Signature-Key`: the public key, in one of three schemes

## Signature-key schemes and `requiredScheme` tiers

The wire-level signature-key scheme (in the `Signature-Key` header) is one of three values
from draft-hardt-httpbis-signature-key:

| Wire scheme | Description |
|-------------|-------------|
| `hwk`       | Inline JWK (pseudonymous; no verified identity). |
| `jwks_uri`  | Identifier + `kid`; verifier fetches JWKS from `{id}/.well-known/{dwk}`. |
| `jwt`       | Signing key bound to a JWT (`aa-agent+jwt` or `aa-auth+jwt`); the JWT's `cnf.jwk` carries the HTTP-signing public key (RFC 7800). |

The policy's `requiredScheme` is a three-tier ladder mapping those wire schemes to a
minimum strength the route accepts:

| `requiredScheme` | Accepts                                                  | Spec `AAuth-Requirement` on rejection |
|------------------|----------------------------------------------------------|---------------------------------------|
| `hwk`            | any signed request                                       | `requirement=pseudonym`               |
| `jwks`           | `jwks_uri` or stronger                                   | `requirement=identity`                |
| `agentJwt`       | `aa-agent+jwt` or `aa-auth+jwt` (verified agent identity)| `requirement=agent-token`             |

Use `agentJwt` when you want a verified agent identity — that's the strongest level this
gateway enforces. `aa-auth+jwt` (authorization token) requests are accepted because they
also carry agent identity, but the gateway does not currently *require* an authorization
token; if you need that, see the limitations section below.

## Wire format

```
Signature-Input: sig1=("@method" "@authority" "@path" "signature-key");created=1730217600
Signature:       sig1=:<base64-ed25519-signature>:
Signature-Key:   sig1=hwk;kty="OKP";crv="Ed25519";x="<base64url-public-key>"
```

The covered components MUST include `@method`, `@authority`, `@path`, and `signature-key`. The
gateway verifies the signature, then makes the verified claims available to CEL via `aauth.*`:

| Field                | Always set?        | Description                                                 |
|----------------------|--------------------|-------------------------------------------------------------|
| `aauth.scheme`       | yes                | `"hwk"`, `"jwks_uri"`, or `"jwt"`                           |
| `aauth.thumbprint`   | yes                | RFC 7638 thumbprint of the signing key                      |
| `aauth.agent`        | jwks_uri, jwt only | the agent's identifier (issuer URL or `agent` claim)        |
| `aauth.agent_delegate` | jwt agent only   | stable agent ID from the agent token's `sub`                |
| `aauth.user`         | auth-token only    | the user the agent acts for                                 |
| `aauth.scope`        | auth-token only    | array of granted scopes                                     |
| `aauth.token_type`   | jwt only           | `"agent+jwt"` or `"auth+jwt"`                               |
| `aauth.jwt_claims`   | jwt only           | full JWT claims object for advanced authorization rules     |

## Running

There are two configs in this directory:

| File              | Purpose                                                                    |
|-------------------|----------------------------------------------------------------------------|
| `config.yaml`     | Production-shaped example with `requiredScheme: agentJwt`, strict HTTPS issuers, and a CEL authorization rule pinning to a specific agent identity. |
| `config.dev.yaml` | Walkthrough config paired with the Go reference client below. Loopback HTTP issuers allowed; no identity pin. **Do not use in production.** |

To launch the gateway with the production-shaped example:

```sh
agentgateway -f examples/aauth/config.yaml
```

A signed request looks like:

```sh
curl -X GET http://localhost:3000/api/data \
  -H 'Signature-Input: sig1=("@method" "@authority" "@path" "signature-key");created=...' \
  -H 'Signature: sig1=:<base64>:' \
  -H 'Signature-Key: sig1=hwk;kty="OKP";crv="Ed25519";x="<base64url>"'
```

## End-to-end demo with the Go reference

A Go reference client and resource server live at
<https://github.com/christian-posta/extauth-aauth-resource>. It ships two binaries
relevant to this demo:

- **`sign-request`** — a one-shot CLI that generates an ephemeral Ed25519 key, signs a
  request with the `hwk` scheme, and prints the equivalent `curl` command.
- **`agent-client`** — a longer-lived demo agent that *also* hosts its own
  `/.well-known/aauth-agent.json` and `/echo` endpoints, so it can serve as both the
  JWKS-publishing issuer and the upstream backend for the gateway. It runs in two modes:
  - `-mode jwks_uri` — signs with a key discovered via the agent's `jwks_uri`.
  - `-mode agent-jwt` — signs with an ephemeral key bound to an `aa-agent+jwt` token
    via `cnf.jwk` (the strongest scheme; demonstrated below).

The walkthrough below uses `config.dev.yaml` and the `aa-agent+jwt` scheme. It exercises
the full path: JWT signature verification (against the agent's JWKS), `cnf.jwk`
extraction, and HTTP Message Signature verification with that bound key.

### 1. Clone and build the Go reference

```sh
git clone https://github.com/christian-posta/extauth-aauth-resource.git
cd extauth-aauth-resource
go build ./cmd/agent-client ./cmd/sign-request
```

The two binaries are produced in the repo root.

### 2. Start the agent-client in serve mode

This launches the agent's well-known + JWKS endpoints on `127.0.0.1:9099`, and the
`/echo` endpoint that will be the gateway's upstream backend:

```sh
./agent-client -serve \
  -listen 127.0.0.1:9099 \
  -issuer http://127.0.0.1:9099 \
  -keystore /tmp/aauth-walkthrough/agent-keys.json
```

Leave this running in its own terminal. The `keystore` file is where the demo persists
its long-lived agent-server signing key and short-lived request-signing key — delete it
to rotate.

Sanity check the well-known metadata in another terminal:

```sh
curl -s http://127.0.0.1:9099/.well-known/aauth-agent.json | jq .
```

### 3. Start agentgateway with the dev config

In another terminal, from the agentgateway repo root:

```sh
agentgateway -f examples/aauth/config.dev.yaml
```

This binds `127.0.0.1:3000`, routes `/echo` to `127.0.0.1:9099`, and enforces
`requiredScheme: agentJwt`. Unsigned requests should now return 401:

```sh
curl -si http://127.0.0.1:3000/echo
# HTTP/1.1 401 Unauthorized
# {"error":"invalid_signature","error_description":"missing signature headers"}
```

### 4. Drive a signed request in `aa-agent+jwt` mode

Back in the terminal where you built the Go reference:

```sh
./agent-client \
  -mode agent-jwt \
  -listen 127.0.0.1:9099 \
  -issuer http://127.0.0.1:9099 \
  -target http://127.0.0.1:3000/echo \
  -authority 127.0.0.1:3000 \
  -keystore /tmp/aauth-walkthrough/agent-keys.json
```

(The `-serve` instance is still running and holds 9099; agent-client detects this and
reuses it as the issuer rather than trying to start a second server.)

Expected output ends with `HTTP/1.1 200 OK` and the echo backend reflecting the signed
headers back:

```
=== signed request ===
GET http://127.0.0.1:3000/echo
signature-key: sig=jwt;jwt="eyJhbGciOiJFZERTQSIsImtpZCI6ImFnZW50c2VydmVyLS4uIiwidHlwIjoiYWEtYWdlbnQranlt..."
signature-input: sig=("@method" "@authority" "@path" "signature-key");created=...;alg="ed25519";keyid="req-..."
signature: sig=:<base64-ed25519-signature>:

=== response ===
HTTP/1.1 200 OK
{"headers":{...},"host":"127.0.0.1:3000","method":"GET","path":"/echo"}
```

What just happened, step by step:

1. The client minted an `aa-agent+jwt` (typ=`aa-agent+jwt`) signed by the agent-server
   key. The JWT's `cnf.jwk` carries a fresh request-signing public key.
2. The client signed the HTTP request with the matching request-signing private key, and
   embedded the JWT verbatim in `Signature-Key: sig=jwt;jwt="..."`.
3. agentgateway parsed `Signature-Key`, decoded the JWT header to get `kid` and `iss`,
   fetched `http://127.0.0.1:9099/.well-known/aauth-agent.json`, followed `jwks_uri`,
   loaded the agent-server JWKS, and verified the JWT's EdDSA signature.
4. With the JWT validated, agentgateway extracted `cnf.jwk` and used it to verify the
   request's HTTP Message Signature.
5. The verified claims (`aauth.scheme`, `aauth.agent`, `aauth.agent_delegate`, full
   `aauth.jwt_claims`) were attached to the request and exposed to CEL before forwarding
   to `/echo`.

### 5. Confirm tamper-rejection

Re-send the same request with the JWT body corrupted — the gateway must reject:

```sh
# Capture a fresh signed-curl example
SIGNED=$(./agent-client -mode agent-jwt -listen 127.0.0.1:9099 \
  -issuer http://127.0.0.1:9099 -target http://127.0.0.1:3000/echo \
  -authority 127.0.0.1:3000 \
  -keystore /tmp/aauth-walkthrough/agent-keys.json)
SIG_KEY=$(printf '%s\n' "$SIGNED" | grep '^signature-key:' | sed 's/^signature-key: //')
SIG_INPUT=$(printf '%s\n' "$SIGNED" | grep '^signature-input:' | sed 's/^signature-input: //')
SIG=$(printf '%s\n' "$SIGNED" | grep '^signature:' | grep -v '^signature-' | head -1 | sed 's/^signature: //')

# Corrupt one base64url char inside the JWT claims segment:
TAMPERED=$(printf '%s' "$SIG_KEY" | sed 's/eyJjbmY/eyZjbmY/')

curl -si -X GET http://127.0.0.1:3000/echo \
  -H "signature-key: $TAMPERED" \
  -H "signature-input: $SIG_INPUT" \
  -H "signature: $SIG"
# HTTP/1.1 401 Unauthorized
# {"error":"invalid_auth_token","error_description":"invalid JWT claims: ..."}
```

### Trying the other schemes

Switch the dev config's `requiredScheme` to `hwk` or `jwks` and rerun with the
appropriate client mode:

- **`hwk`** — `./sign-request -authority 127.0.0.1:3000 -path /echo` produces a signed
  `curl` command you can `eval`. The Signature-Key embeds an ephemeral public key inline.
- **`jwks`** — `./agent-client -mode jwks_uri -listen 127.0.0.1:9099 -target
  http://127.0.0.1:3000/echo -authority 127.0.0.1:3000 ...` — same flow as
  `agent-jwt` but without the JWT layer.

### Ports

If `3000` or `9099` are taken on your machine (Docker port forwards are a common
culprit), change them in `config.dev.yaml` and the `agent-client -listen` / `-target`
flags to match. The `-issuer` and `-authority` arguments must align with the listener
host:port the client is actually addressing.

## Modes

- `strict` (default): reject any request that doesn't pass verification at the required scheme.
- `optional`: allow unsigned requests through; verify signed ones strictly.
- `permissive`: never reject; useful for logging/observability rollout without enforcement.
  No `aauth.*` claims are populated when the signature is invalid.

## Limitations

- `Content-Digest` is verified at the signature-base level (a tampered digest header breaks the
  signature) but the request body is not re-hashed inside the gateway. Backends that care about
  body integrity should re-verify `Content-Digest` themselves, or sit behind a `buffer` policy.
- Only Ed25519 signatures are supported for HTTP message signing today; the JWT layer accepts
  RSA / EC / EdDSA via the issuer's JWKS.
- **Authorization tokens are out of scope.** The gateway verifies agent identity
  (`aa-agent+jwt`) and accepts `aa-auth+jwt` because it carries agent identity too, but it
  does not enforce the `requirement=auth-token` challenge flow from
  draft-hardt-oauth-aauth-protocol §6.6 (which would involve minting resource tokens and
  advertising an authorization server). If your route needs scopes / user delegation /
  authorization-server attestation, layer a separate policy on top.
