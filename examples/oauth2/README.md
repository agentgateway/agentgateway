## OAuth2 Example

This example shows how to protect a route with agentgateway's built-in OAuth2/OIDC policy.

It supports `clientSecret` as inline or file-based local configuration, and flat provider settings using either `issuer` or explicit endpoint fields.

### Running the example

Quick scripted browser demo (starts Keycloak + backend + gateway):

```bash
./examples/oauth2/run-browser-demo.sh start
```

For testing from another machine on your LAN:

```bash
PUBLIC_HOST=192.168.1.50 ./examples/oauth2/run-browser-demo.sh start
```

1. Create an OAuth2/OIDC app in your identity provider.
2. Configure an allowed callback/redirect URI of:

```text
http://localhost:3000/oauth2/callback
```

3. Update `examples/oauth2/config.yaml` with your provider values:

- `issuer` or `authorizationEndpoint`/`tokenEndpoint`
- optionally `providerBackend` (for private IdP/back-channel routing)
- `clientId`
- `clientSecret` (inline or file)
- `redirectUri` (required; should be explicit and deterministic in production)
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
  file: ./examples/oauth2/client-secret.example.txt
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
    clientId: "replace-with-client-id"
    clientSecret:
      file: ./examples/oauth2/client-secret.example.txt
    redirectUri: "http://localhost:3000/oauth2/callback"
    providerBackend:
      host: "idp.internal.example.com:443"
```

Controller/XDS (`AgentgatewayPolicy`) uses `backendRef` for the same behavior:

```yaml
traffic:
  oauth2:
    issuer: https://issuer.example.com
    clientId: replace-with-client-id
    clientSecret:
      secretRef:
        name: oauth2-client
        key: client-secret
    redirectUri: https://gateway.example.com/oauth2/callback
    backendRef:
      kind: Service
      name: oauth2-discovery
      port: 8080
```

Explicit non-OIDC OAuth2 providers can be configured without discovery:

```yaml
traffic:
  oauth2:
    authorizationEndpoint: https://provider.example.com/oauth/authorize
    tokenEndpoint: https://provider.example.com/oauth/token
    tokenEndpointAuthMethodsSupported:
    - client_secret_post
    clientId: replace-with-client-id
    clientSecret:
      secretRef:
        name: oauth2-client
        key: client-secret
    redirectUri: https://gateway.example.com/oauth2/callback
```

### CEL identity claim examples

OAuth2 can inject verified `id_token` claims into the CEL `jwt` object.
You can use that in `authorization` and `transformations` policies.

### Optional hardening knobs

- `providerBackend` (local config) when provider back-channel calls must go through a specific internal backend path.
- `backendRef` (controller/XDS `AgentgatewayPolicy`) for the same provider back-channel routing behavior.
- `backendAuth.passthrough` forwards the OAuth2 access token upstream when your backend requires it.
- `postLogoutRedirectUri` must be `https` (or `http` on loopback), and cannot include URL fragments or userinfo.

Allow only one user (`sub`) on a protected route:

```yaml
policies:
  oauth2:
    issuer: "https://issuer.example.com"
    clientId: "replace-with-client-id"
    clientSecret:
      file: ./examples/oauth2/client-secret.example.txt
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
      file: ./examples/oauth2/client-secret.example.txt
  transformations:
    request:
      set:
        x-user-sub: "jwt.sub"
```
