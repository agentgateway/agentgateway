# Backend auth with short-lived JWT signing

Some upstreams reject static credentials and require every request to carry a
short-lived JWT signed with a private key. The best-known example is the
[Snowflake SQL API](https://docs.snowflake.com/en/developer-guide/sql-api/authenticating#using-key-pair-authentication),
which caps token lifetime at one hour and derives the expected `iss`/`sub`
claims from the account, user, and public-key fingerprint.

The `backendAuth.jwtSign` policy keeps the private key at the gateway and mints
a short-lived token from configured static claims plus signer-controlled
`iat`/`exp`. The token is reused until shortly before expiry and written to the
`Authorization` header with a `Bearer ` prefix by default (configurable via
`location`).

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

The echoed `Authorization` header carries the signed JWT. Decode its claims:

```bash
payload=$(curl -s 127.0.0.1:3000/headers | jq -r '.headers.Authorization' | cut -d' ' -f2 | cut -d. -f2 | tr '_-' '/+')
case $(( ${#payload} % 4 )) in
  2) payload="${payload}==" ;;
  3) payload="${payload}=" ;;
esac
echo "$payload" | base64 -d | jq .
```

The payload is base64url-encoded (RFC 7515): unlike standard base64, it uses
`-`/`_` instead of `+`/`/` and omits padding, so `base64 -d` needs both fixed
up first.

Every request gets a valid token: `iat`/`exp` are set by the signer (`ttl`
controls the lifetime, 300s by default) and cannot be configured as claims,
nor can `nbf`. Tokens are refreshed 15 seconds before expiry. To tolerate clock
skew between the gateway and the upstream, `iat` is backdated and `exp`
extended by 10 seconds, mirroring Google's auth library.

### Notes

* The config inlines a throwaway demo key. For real use, generate your own key
  and reference it as a file (see the comment in `config.yaml`); the upstream
  is registered with the corresponding public key. File-based keys are watched,
  so rotating the key on disk reloads it without a restart.
* RSA (`RS256`/`RS384`/`RS512`) and EC (`ES256`/`ES384`) keys are supported.
* Static companion headers some upstreams expect next to the JWT (like
  Snowflake's `X-Snowflake-Authorization-Token-Type: KEYPAIR_JWT`) are plain
  request header modifications, shown in the config.
* On Kubernetes, the same policy is available as `backend.auth.jwtSign` in
  `AgentgatewayPolicy`/`AgentgatewayBackend`, with the PEM key provided through
  a Secret's `signingKey` data key.
