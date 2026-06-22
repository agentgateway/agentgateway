## RFC 8693 Token Exchange

This example uses `extAuthz` to exchange an incoming bearer token with Keycloak before forwarding the request upstream.

The flow is:

- `initial-client` issues the original token for `testuser`
- agentgateway exchanges it as `requester-client`
- the upstream receives a token for `target-client`

### Running the example

Start Keycloak and the demo upstream:

```bash
docker compose -f examples/token-exchange/docker-compose.yaml up -d
```

Run agentgateway:

```bash
cargo run -- -f examples/token-exchange/config.yaml
```

Get an initial token:

```bash
SUBJECT_TOKEN="$(curl -s http://localhost:7080/realms/agentgateway-token-exchange/protocol/openid-connect/token \
  -u initial-client:initial-secret \
  -H 'content-type: application/x-www-form-urlencoded' \
  -d grant_type=password \
  -d username=testuser \
  -d password=testpass \
  | jq -r .access_token)"
```

Send it through the gateway:

```bash
curl -i http://localhost:3000/ \
  -H "authorization: Bearer $SUBJECT_TOKEN"
```

The request is forwarded with the exchanged token. The access log includes:

```text
token_exchange.subject="..." token_exchange.audience="target-client"
```

Stop the demo:

```bash
docker compose -f examples/token-exchange/docker-compose.yaml down
```
