## OAuth2 Example

This example shows how to protect a route with agentgateway's built-in OAuth2/OIDC policy.

It supports `clientSecret` as inline or file-based local configuration.

### Running the example

1. Create an OAuth2/OIDC app in your identity provider.
2. Configure an allowed callback/redirect URI of:

```text
http://localhost:3000/_gateway/callback
```

3. Update `examples/oauth2/config.yaml` with your provider values:

- `issuer`
- `clientId`
- `clientSecret` (inline or file)
- `redirectUri` (recommended and deterministic in production)

4. Start a local upstream app:

```bash
python -m http.server 8080
```

5. Run agentgateway:

```bash
cargo run -- -f examples/oauth2/config.yaml
```

6. Open `http://localhost:3000` in your browser. You should be redirected to your IdP login flow.

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

### CEL identity claim examples

OAuth2 can inject verified `id_token` claims into the CEL `jwt` object.
You can use that in `authorization` and `transformations` policies.

### Optional hardening knobs

- `autoDetectRedirectUri: true` only when you intentionally want redirect inference from request headers.
- `trustedProxyCidrs` to allow `X-Forwarded-Host` and `X-Forwarded-Proto` from known proxy ranges.
- `denyRedirectMatchers` for API paths that should return `401` instead of browser redirects.
- `passAccessToken` defaults to `false` (explicitly set `true` only when upstream requires it).

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
