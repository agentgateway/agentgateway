## OAuth2 Proxy integration

This example shows how to integrate with [OAuth2 Proxy](https://oauth2-proxy.github.io/oauth2-proxy/) for authorization.
For a gateway-native browser login flow, use the [oidc](../oidc/README.md) example instead.

In this example, we set up GitHub OAuth authentication. The same pattern can be used with other providers, or with other authentication sources.

### Running the example

First, create a [GitHub OAuth App](https://github.com/settings/applications/new).
Use `http://localhost:3000/oauth2/callback` as the callback URL. Then take note of the Client ID
and Client Secret, and start OAuth2 Proxy locally:

```bash
export OAUTH2_PROXY_CLIENT_SECRET=...
export OAUTH2_PROXY_CLIENT_ID=...
export OAUTH2_PROXY_COOKIE_SECRET=`python -c 'import os,base64; print(base64.b64encode(os.urandom(16)).decode("ascii"))'`
docker compose -f examples/oauth2-proxy/docker-compose.yaml up
```

Note: the example configuration of OAuth2 Proxy uses a minimal setup to get started.
Review the [OAuth2 Proxy documentation](https://oauth2-proxy.github.io/oauth2-proxy/configuration/overview) for real-world usage.

Then run agentgateway:

```bash
cargo run -- -f examples/oauth2-proxy/config.yaml
```

Requests to `http://localhost:3000` should automatically redirect to a GitHub sign-in page.

### Device authorization grant transforms

Some OAuth 2.0 Device Authorization Grant clients send empty
`application/x-www-form-urlencoded` POST bodies and expect the gateway to add
public-client fields before forwarding to the identity provider. A
`transformation` policy can parse the existing form body, merge required fields,
and re-encode it with `form.decode` and `form.encode`.

```yaml
policies:
  transformations:
    request:
      body: |
        request.path == "/oauth/devicecode" ?
          form.encode(form.decode(request.body).merge({
            "client_id": "00000000-0000-0000-0000-000000000000",
            "scope": "openid profile api://00000000-0000-0000-0000-000000000000/access_as_user"
          })) :
        request.path == "/oauth/token" ?
          form.encode(form.decode(request.body).merge({
            "client_id": "00000000-0000-0000-0000-000000000000"
          })) :
        request.body
    response:
      body: |
        request.path == "/oauth/devicecode" ?
          json(response.body).with(body,
            body.merge({
              "verification_uri": "https://gateway.example.com/oauth/verify",
              "verification_uri_complete": "https://gateway.example.com/oauth/verify?user_code=" + body.user_code
            })
          ) :
        response.body
```

This pairs with the existing route, `urlRewrite`, `directResponse`, or
`requestRedirect` policies used to proxy the authorization server metadata and
serve the gateway-origin verification URL.
