# Backend auth with per-request JWT signing

Some upstreams reject static credentials and require every request to carry a
short-lived JWT signed with a private key. The best-known example is the
[Snowflake SQL API](https://docs.snowflake.com/en/developer-guide/sql-api/authenticating#using-key-pair-authentication),
which caps token lifetime at one hour and derives the expected `iss`/`sub`
claims from the account, user, and public-key fingerprint.

The `backendAuth.jwtSign` policy keeps the private key at the gateway and mints
a fresh token on each request: configured static claims plus signer-controlled
`iat`/`exp`, written to the `Authorization` header with a `Bearer ` prefix by
default (configurable via `location`).

### Running the example

Start an upstream that echoes request headers:

```bash
docker run --rm -p 8080:80 kennethreitz/httpbin
```

Run agentgateway:

```bash
cargo run -- -f examples/traffic-jwt-sign/config.yaml
```

Send a request and observe the signed token the upstream received:

```bash
curl -s 127.0.0.1:3000/headers
```

The echoed `Authorization` header carries a fresh JWT. Decode its claims:

```bash
curl -s 127.0.0.1:3000/headers \
  | jq -r '.headers.Authorization' \
  | cut -d' ' -f2 | cut -d. -f2 | base64 -d 2>/dev/null | jq .
```

Every request gets a new token: `iat`/`exp` are set by the signer (`ttl`
controls the lifetime, 300s by default) and cannot be configured as claims.

### Notes

* The config inlines a throwaway demo key. For real use, generate your own key
  and reference it as a file (see the comment in `config.yaml`); the upstream
  is registered with the corresponding public key.
* RSA (`RS256`/`RS384`/`RS512`) and EC (`ES256`/`ES384`) keys are supported.
* Static companion headers some upstreams expect next to the JWT (like
  Snowflake's `X-Snowflake-Authorization-Token-Type: KEYPAIR_JWT`) are plain
  request header modifications, shown in the config.
* On Kubernetes, the same policy is available as `backend.auth.jwtSign` in
  `AgentgatewayPolicy`/`AgentgatewayBackend`, with the PEM key provided through
  a Secret's `signingKey` data key.
