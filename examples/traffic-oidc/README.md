## OIDC browser auth

This example shows the built-in `oidc` flow for browser authentication.

It uses:
- Keycloak as a local OIDC issuer
- one OIDC-protected route that serves both app traffic and the `/oauth/callback` path
- the standard JWT claims surface (`jwt.sub`, `jwt.email`) in access logs after login

### Running the example

Start the demo dependencies:

```bash
docker compose -f examples/traffic-oidc/docker-compose.yaml up -d
```

Export the required browser-auth cookie secret, then start agentgateway:

```bash
export OIDC_COOKIE_SECRET="$(python3 -c 'import os; print(os.urandom(32).hex())')"
cargo run -- -f examples/traffic-oidc/config.yaml
```

Open `http://localhost:3000` in a browser. The gateway redirects to Keycloak, completes the code flow itself, and then returns to the protected upstream app.

Use these demo credentials:

- username: `testuser`
- password: `testpass`

Configuration:

```yaml
frontendPolicies:
  accessLog:
    add:
      user.id: jwt.sub
      user.email: jwt.email
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: app
      matches:
      - path:
          pathPrefix: /
      backends:
      - host: localhost:18080
      policies:
        oidc:
          issuer: http://localhost:7080/realms/agentgateway
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
          scopes:
          - profile
          - email
```

Stop the demo with:

```bash
docker compose -f examples/traffic-oidc/docker-compose.yaml down
```

### Custom login page and logout

The OIDC policy can also protect an application that supplies its own login page.
Add these optional settings to `policies.oidc`:

```yaml
login:
  # Gateway endpoint that starts OAuth. The page's sign-in link points here.
  # Preserve the page's returnTo query parameter in that link, for example:
  # /auth/login?returnTo=%2Fapp
  path: /auth/login
  # BEFORE login: send unauthenticated browser navigations to this application page.
  # The gateway appends returnTo with the originally requested local path and query.
  # This is neither the provider callback (redirectURI) nor a post-login destination.
  redirect: /login
logout:
  # Gateway endpoint that clears this policy's cookies. Use a same-origin POST form.
  path: /auth/logout
  # AFTER logout: send the browser here with a 303 redirect.
  # Defaults to login.redirect when set, otherwise /.
  redirect: /login
```

Serve `/login` and any assets it needs on public routes, or bypass the OIDC policy
for those paths with a conditional policy. Setting `login.redirect` does not bypass
any authentication or authorization policy. Route the login-start, logout, and
callback endpoints through the same OIDC policy as the protected application.

The login page should render a sign-in link to `login.path`, carrying its `returnTo`
parameter. Only safe local return targets are accepted; missing or invalid targets
fall back to `/`. Fetch requests receive 401 instead of following OAuth redirects;
when `login.redirect` is set, the response includes it in the `Location` header so
the application can navigate the browser to the login page.

Submit logout using a form such as:

```html
<form method="post" action="/auth/logout">
  <button type="submit">Sign out</button>
</form>
```

Logout requires an `Origin` header matching the origin of `redirectURI`. It clears
the gateway session, including when that session has expired, but does not end the
identity provider's session or revoke tokens. Choose a public logout destination;
a protected destination may immediately start another SSO login.

`login` and `logout` are independent. Each requires `path` when present; its
`redirect` is optional. Omit `login.redirect` to keep automatic redirects to the
provider. Omit `logout` to disable the logout endpoint. All four values are local
paths; endpoint paths cannot contain queries, and endpoint paths must differ from
each other and the callback path.

The built-in UI manages its own login and logout endpoints. Omit `login` and
`logout` under `ui.policies.oidc`; configuring them there is rejected.
