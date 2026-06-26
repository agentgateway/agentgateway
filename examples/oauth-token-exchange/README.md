## OAuth Token Exchange

These examples show backend `oauth` token exchange policies for common OAuth providers.
Start with `tokenEndpoint`, then add only the fields your provider requires. By default, the gateway performs RFC 8693 token exchange using the incoming `Authorization: Bearer ...` token as the subject token and forwards the exchanged token to the upstream in the same header.

They include:

- [entra-obo.yaml](entra-obo.yaml): Microsoft Entra on-behalf-of flow using the RFC 7523 JWT bearer grant and `requested_token_use=on_behalf_of`
- [okta-rfc8693.yaml](okta-rfc8693.yaml): Okta RFC 8693 token exchange with a custom authorization server audience
- [keycloak-rfc8693.yaml](keycloak-rfc8693.yaml): Keycloak RFC 8693 token exchange, including repeated `audience` values for downscoping
- [google-sts.yaml](google-sts.yaml): Google STS workload identity federation without client authentication, using `audience`, `scope`, and an ID-token subject
- [zitadel-rfc8693.yaml](zitadel-rfc8693.yaml): ZITADEL RFC 8693 token exchange using a project audience and JWT requested token type

Examples intentionally rely on defaults where possible. `subjectToken` is only shown when a provider needs a non-default token type, and `requestedTokenType` is only shown when the provider expects it.

### Running the examples

Replace the placeholder tenant, issuer, client, secret, audience, and upstream values in the example you want to try.

Run agentgateway with one of the configs:

```bash
cargo run -- -f examples/oauth-token-exchange/entra-obo.yaml
```

Send a request with an incoming subject token:

```bash
curl -i http://localhost:3000/entra \
  -H "authorization: Bearer $SUBJECT_TOKEN"
```

Use the matching route prefix for the selected example: `/entra`, `/okta`, `/keycloak`, `/google-sts`, or `/zitadel`.

### Configuration

The examples rely on the `oauth` backend auth policy:

```yaml
backendAuth:
  oauth:
    tokenEndpoint:
      backend: token-endpoint
    audiences:
    - example-audience
```

Common provider-specific fields:

- `audiences`: OAuth token exchange `audience` parameters, commonly used by Google STS and provider-specific STS flows.
- `resources`: RFC 8707 `resource` indicators for providers that model target APIs as resource URIs.
- `scopes`: requested scopes, joined into one space-delimited `scope` form parameter.
- `additionalParams`: extra token-endpoint form fields for provider extensions, with values evaluated as CEL expressions.
- `actorToken`: optional RFC 8693 delegation token. Its source must be explicit.
