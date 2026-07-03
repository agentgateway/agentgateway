## Token exchange with `extAuthz`

This example uses `extAuthz` to get OAuth access tokens from Keycloak before forwarding requests upstream, building the token request by hand with CEL. For the built-in `backendAuth.oauth` policy that does the same job, see the sibling [oauth-rfc8693](../oauth-rfc8693/README.md) and [jwt-authz-grant](../jwt-authz-grant/README.md) examples.

It includes two backend auth flows:

- `/exchange` uses RFC 8693 token exchange with an incoming bearer token.
- `/client-credentials` uses the RFC 6749 client credentials grant.

### Running the example

Start Keycloak and the demo upstream:

```bash
docker compose -f examples/traffic-token-exchange/extauthz/docker-compose.yaml up -d
```

Run agentgateway:

```bash
cargo run -- -f examples/traffic-token-exchange/extauthz/config.yaml
```

Get an initial token for the token exchange flow:

```bash
SUBJECT_TOKEN="$(curl -s http://localhost:7080/realms/backend-oauth/protocol/openid-connect/token \
  -u initial-client:initial-secret \
  -H 'content-type: application/x-www-form-urlencoded' \
  -d grant_type=password \
  -d username=testuser \
  -d password=testpass \
  | jq -r .access_token)"
```

Use RFC 8693 token exchange:

```bash
curl -i http://localhost:3000/exchange \
  -H "authorization: Bearer $SUBJECT_TOKEN"
```

Use RFC 6749 client credentials:

```bash
curl -i http://localhost:3000/client-credentials
```

Both routes forward the request with the acquired token. The access log includes:

```text
backend_oauth.grant="..." backend_oauth.subject="..." backend_oauth.audience="target-client"
```

Stop the demo:

```bash
docker compose -f examples/traffic-token-exchange/extauthz/docker-compose.yaml down
```
