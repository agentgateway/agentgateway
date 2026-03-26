## OAuth2 Proxy integration

This example shows how to integrate with [OAuth2 Proxy](https://oauth2-proxy.github.io/oauth2-proxy/) for authorization.
For a gateway-native browser login flow, use the [oidc](../oidc/README.md) example instead.

In this example, we set up GitHub OAuth authentication. The same pattern can be used with other providers, or with other authentication sources.

### Running the example

First, create a [GitHub OAuth App](https://github.com/settings/applications/new).
Take note of the Client ID and Client Secret, and start OAuth2 Proxy locally:

```bash
export OAUTH2_PROXY_CLIENT_SECRET=...
export OAUTH2_PROXY_CLIENT_ID=...
export OAUTH2_PROXY_COOKIE_SECRET=`python -c 'import os,base64; print(base64.b64encode(os.urandom(16)).decode("ascii"))'`
docker compose up
```

Note: the example configuration of OAuth2 Proxy uses a minimal setup to get started.
Review the [OAuth2 Proxy documentation](https://oauth2-proxy.github.io/oauth2-proxy/configuration/overview) for real-world usage.

Then run agentgateway:

```bash
cargo run -- -f examples/oauth2-proxy/config.yaml
```

Requests to `http://localhost:3000` should automatically redirect to a GitHub sign-in page.
