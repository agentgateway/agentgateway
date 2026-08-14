# Identity Assertion Authorization Grant (ID-JAG / Cross App Access)

These examples show how agentgateway uses the **OAuth Identity Assertion Authorization Grant**
([draft-ietf-oauth-identity-assertion-authz-grant](https://datatracker.ietf.org/doc/draft-ietf-oauth-identity-assertion-authz-grant/),
also called "ID-JAG" or "Cross App Access") to call a downstream API *as the authenticated
end user*, without requiring the user to interactively log in to that downstream app.

## What is ID-JAG?

When a user is already signed in to one app (or an agent acts on their behalf), reaching a
*second* app in another trust domain traditionally means another login/consent screen. ID-JAG
removes that: the identity provider issues a short-lived, signed **assertion** that carries the
user's identity across to the resource app, which exchanges it for an access token — one login,
a cryptographic handoff at each boundary, no shared credentials.

agentgateway acts as the confidential OAuth client (the "requesting app") and performs a two-leg
exchange on each backend call:

1. **Authenticate the user.** The inbound request carries the user's OIDC **ID token**,
   validated by the `jwtAuth` policy. The validated token is the *subject* of the exchange.
2. **Token exchange (RFC 8693).** The gateway calls the user's IdP authorization server with
   `grant_type=urn:ietf:params:oauth:grant-type:token-exchange` and
   `requested_token_type=urn:ietf:params:oauth:token-type:id-jag`, receiving an **ID-JAG**
   assertion bound to the resource authorization server (`audience`).
3. **JWT-bearer grant (RFC 7523).** The gateway presents the ID-JAG to the resource's
   authorization server with `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer`,
   receiving a **Bearer access token** scoped to the downstream API.
4. **Attach + cache.** The Bearer token is added as `Authorization: Bearer <token>` to the
   upstream request and cached until shortly before it expires.

```
client ──ID token──▶ agentgateway ──(1) token-exchange──▶ IdP AS ──ID-JAG──▶ agentgateway
                                  ──(2) jwt-bearer grant──▶ Resource AS ──access token──▶ agentgateway
                                  ──Authorization: Bearer <access token>──▶ downstream API
```

## The three demos

Each subdirectory is a self-contained demo of the **same** `backendAuth.crossAppAccess` policy,
differing only in *who* plays the IdP and the resource authorization server. Each has its own
`README.md` with step-by-step instructions.

| Demo | IdP / Resource AS | External accounts | Best for |
|---|---|---|---|
| **[`keycloak/`](keycloak/)** | one local **Keycloak** plays *both* roles (container image provided) | none | **Start here** — fully local, one-command setup, runs end to end with no signups. |
| **[`xaa-dev/`](xaa-dev/)** | the public **[xaa.dev](https://xaa.dev)** playground (IdenX IdP + hosted resource API) | free xaa.dev app registration | Trying it against a real hosted IdP/resource server with minimal setup. |
| **[`okta-auth0/`](okta-auth0/)** | **Okta** IdP + **Auth0** resource AS | Okta (Cross App Access EA) + Auth0 (XAA beta) | The real cross-domain enterprise topology; needs both provider accounts. |

All three drive the identical flow — an inbound ID token becomes a backend access token minted
for the end user — so the agentgateway config is nearly identical across them; only the endpoints,
client ids/secrets, `audience`, and `resources` change.

## How the policy is configured

The feature is a focused `backendAuth.crossAppAccess` policy next to the `jwtAuth` policy that
authenticates the user:

- `identityProvider` — the user's IdP token endpoint. agentgateway sends the authenticated ID token as
  the RFC 8693 `subject_token` and asks for `urn:ietf:params:oauth:token-type:id-jag`.
- `resourceAuthorizationServer` — the resource authorization server token endpoint. This leg uses the
  RFC 7523 jwt-bearer grant; the ID-JAG from the IdP leg is sent as the `assertion`.
- `audience` — the resource authorization server identifier. The issued ID-JAG is bound to this value.
- `clientAuth` — supported methods are `clientSecretBasic`, `clientSecretPost`, and
  `privateKeyJwt`. `privateKeyJwt` requires an explicit `assertionAudience` because token
  endpoints are configured as backend references rather than raw URLs.
- `resources` (optional) — protected resource/API identifiers (RFC 8707). Configure these
  explicitly when the authorization server expects them.
- `scopes` (optional) — scopes to request; the authorization server may grant a subset.
- `cache.defaultTtl` (optional) — fallback TTL when the final token response omits `expires_in`.
  The cache is capped by the subject token's JWT `exp` when present.

By default, the subject ID token is read from the `Authorization: Bearer` header. To extract it
from another request value or a validated JWT claim, configure `subjectToken.source`:

```yaml
subjectToken:
  source:
    expression: jwt.the_id_token
```

Each endpoint (and the upstream backend) needs `backendTLS: {}` when it is HTTPS — see the demo
READMEs.

> The `jwtAuth` policy must validate an **OIDC ID token** (not an arbitrary access token), as
> that is what the IdP expects as the `subject_token`.
