# Identity Assertion Authorization Grant (ID-JAG / Cross App Access)

This example shows how agentgateway uses the **OAuth Identity Assertion Authorization Grant**
([draft-ietf-oauth-identity-assertion-authz-grant](https://datatracker.ietf.org/doc/draft-ietf-oauth-identity-assertion-authz-grant/),
also called "ID-JAG" or "Cross App Access") to call a downstream API *as the authenticated
end user*, without requiring the user to interactively log in to that downstream app.

The gateway acts as a confidential OAuth client and performs a two-leg exchange on each
backend call:

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

## Prerequisites

ID-JAG is a cross-domain protocol, so a full end-to-end run needs:

- An **IdP authorization server** that supports the RFC 8693 token exchange and can issue the
  `urn:ietf:params:oauth:token-type:id-jag` token type.
- A **resource authorization server** that accepts the ID-JAG via the `jwt-bearer` grant.
- **Two client registrations** — one for the gateway at the IdP, and a separate one at the
  resource authorization server (each with its own `clientId` and credentials).

Because of those external dependencies, the [`config.yaml`](config.yaml) here is illustrative:
set `IDP_CLIENT_SECRET` and `RESOURCE_AUTHORIZATION_SERVER_CLIENT_SECRET`, then adapt the
example endpoints to values from your own IdP and resource server.

## Configuration walkthrough

The feature is configured as a focused `backendAuth.crossAppAccess` policy next to the `jwtAuth`
policy that authenticates the user:

- `identityProvider` — the user's IdP token endpoint. agentgateway sends the authenticated ID token as
  the RFC 8693 `subject_token` and asks for `urn:ietf:params:oauth:token-type:id-jag`.
- `resourceAuthorizationServer` — the resource authorization server token endpoint. This leg uses the
  RFC 7523 jwt-bearer grant; the ID-JAG from the IdP leg is sent as the `assertion`.
- `audience` — the resource authorization server identifier. The issued ID-JAG is bound
  to this value.
- `clientAuth` — supported methods are `clientSecretBasic`, `clientSecretPost`, and
  `privateKeyJwt`. `privateKeyJwt` requires an explicit `assertionAudience` because token
  endpoints are configured as backend references rather than raw URLs.
- `resources` (optional) — protected resource/API identifiers (RFC 8707). Configure these
  explicitly when the authorization server expects them.
- `scopes` (optional) — scopes to request; the authorization server may grant a subset.
- `cache.defaultTtl` (optional) — fallback TTL when the final token response omits
  `expires_in`. The cache is capped by the subject token's JWT `exp` when present.

> The `jwtAuth` policy must validate an **OIDC ID token** (not an arbitrary access token), as
> that is what the IdP expects as the `subject_token`.

## Not yet supported

The following parts of the draft are intentionally out of scope for this initial
implementation and are tracked as follow-ups:

- DPoP sender-constrained tokens (RFC 9449)
- `.well-known` endpoint discovery (RFC 8414) — endpoints must be configured explicitly
- SAML and refresh-token subject types (only OIDC ID tokens are used as the subject)
