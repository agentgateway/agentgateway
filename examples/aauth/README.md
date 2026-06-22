# AAuth (HTTP Message Signing)

This example shows agentgateway verifying inbound requests signed with AAuth — a per-request
HTTP message signature protocol designed for cross-organization agent traffic. Each request
carries three headers:

- `Signature-Input`: which components were signed (RFC 9421)
- `Signature`: the Ed25519 signature bytes
- `Signature-Key`: the public key, in one of three schemes

## Signature-key schemes

| Scheme     | Strength       | Use when                                                                    |
|------------|----------------|-----------------------------------------------------------------------------|
| `hwk`      | Pseudonymous   | abuse prevention without identity; key is inline in the header              |
| `jwks_uri` | Identified     | the agent is known by issuer URL; gateway fetches JWKS from `.well-known`   |
| `jwt`      | Authorized     | the signing key is bound to a JWT (`aa-agent+jwt` / `aa-auth+jwt`)          |

The schemes form a hierarchy: a request signed with `jwt` always satisfies a `jwks` or `hwk`
requirement, and `jwks` satisfies `hwk`. Configure `requiredScheme` to the minimum strength
the route accepts; stronger signatures always pass.

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

A reference client and Python validator live at
<https://github.com/christian-posta/extauth-aauth-resource>.

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
