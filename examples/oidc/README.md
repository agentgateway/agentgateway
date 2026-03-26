## OIDC browser auth

This example shows the built-in `oidc` flow for browser authentication.

It uses:
- Keycloak as a local OIDC issuer
- a normal callback route at `/oauth/callback`
- a self-contained `policies.oidc` payload on both the callback and application routes
- the standard JWT claims surface (`jwt.sub`, `jwt.email`) in access logs after login

### Running the example

Start the demo dependencies:

```bash
docker compose -f examples/oidc/docker-compose.yaml up -d
```

Export the required browser-auth cookie secret, then start agentgateway:

```bash
export OIDC_COOKIE_SECRET="$(python3 -c 'import os; print(os.urandom(32).hex())')"
cargo run -- -f examples/oidc/config.yaml
```

Open `http://localhost:3000` in a browser. The gateway redirects to Keycloak, completes the code flow itself, and then returns to the protected upstream app.

Use these demo credentials:

- username: `testuser`
- password: `testpass`

Configuration:

```yaml
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: callback
      matches:
      - path:
          exact: /oauth/callback
      policies:
        oidc:
          issuer: http://localhost:7080/realms/agentgateway
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
          scopes: [profile, email]
    - name: application
      matches:
      - path:
          pathPrefix: /
      policies:
        oidc:
          issuer: http://localhost:7080/realms/agentgateway
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
          scopes: [profile, email]
```

Stop the demo with:

```bash
docker compose -f examples/oidc/docker-compose.yaml down
```
