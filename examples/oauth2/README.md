## OAuth2 Example

This example shows how to protect a route with agentgateway's built-in OAuth2/OIDC policy.

It supports `clientSecret` as inline or file-based local configuration, and optional `providerBackend` for private IdP back-channel connectivity.

### Running the example

1. Create an OAuth2/OIDC app in your identity provider.
2. Configure an allowed callback/redirect URI of:

```text
http://localhost:3000/_gateway/callback
```

3. Update `examples/oauth2/config.yaml` with your provider values:

- `issuer`
- optionally `providerBackend` (for private IdP/back-channel routing)
- `clientId`
- `clientSecret` (inline or file)
- `redirectUri` (recommended and deterministic in production)
- optionally `postLogoutRedirectUri` (for IdP logout redirect after `signOutPath`)

4. Start a local upstream app:

```bash
python3 -m http.server 8080
```

5. Run agentgateway:

```bash
cargo run -- -f examples/oauth2/config.yaml
```

6. Open `http://localhost:3000` in your browser. You should be redirected to your IdP login flow.

### Logout behavior

- `signOutPath` clears the local gateway session cookie.
- If OIDC discovery exposes `end_session_endpoint`, agentgateway also performs RP-initiated logout using `id_token_hint`.
- `postLogoutRedirectUri` (optional) is forwarded to the IdP as `post_logout_redirect_uri`.
- If OIDC logout metadata is unavailable or invalid, logout still succeeds locally (session cleared, no IdP redirect).

### Secret options

Local/dev file-based:

```yaml
clientSecret:
  file: ./examples/oauth2/client-secret.txt
```

Inline:

```yaml
clientSecret: "your-client-secret"
```

### Private IdP Connectivity

Local config can route OIDC discovery/token/JWKS calls through a dedicated backend:

```yaml
policies:
  oauth2:
    issuer: "https://issuer.example.com"
    providerBackend:
      host: "idp.internal.example.com:443"
```

Controller/XDS (`AgentgatewayPolicy`) uses `providerBackendRef` for the same behavior:

```yaml
traffic:
  oauth2:
    issuer: https://issuer.example.com
    providerBackendRef:
      kind: Service
      name: oauth2-discovery
      port: 8080
```

### CEL identity claim examples

OAuth2 can inject verified `id_token` claims into the CEL `jwt` object.
You can use that in `authorization` and `transformations` policies.

### Optional hardening knobs

- `autoDetectRedirectUri: true` only when you intentionally want redirect inference from request headers.
- `providerBackend` when your issuer/JWKS/token endpoints are private or must be reached through a specific internal backend path.
- `trustedProxyCidrs` to allow `X-Forwarded-Host` and `X-Forwarded-Proto` from known proxy ranges.
- `denyRedirectMatchers` for API paths that should return `401` instead of browser redirects.
- `passAccessToken` defaults to `false` (explicitly set `true` only when upstream requires it).
- `postLogoutRedirectUri` must be `https` (or `http` on loopback), and cannot include URL fragments or userinfo.

Allow only one user (`sub`) on a protected route:

```yaml
policies:
  oauth2:
    issuer: "https://issuer.example.com"
    clientId: "replace-with-client-id"
    clientSecret:
      file: ./examples/oauth2/client-secret.txt
  authorization:
    rules:
    - 'jwt.sub == "user-123"'
```

Extract identity into upstream headers with CEL:

```yaml
policies:
  oauth2:
    issuer: "https://issuer.example.com"
    clientId: "replace-with-client-id"
    clientSecret:
      file: ./examples/oauth2/client-secret.txt
  transformations:
    request:
      set:
        x-user-sub: "jwt.sub"
```
