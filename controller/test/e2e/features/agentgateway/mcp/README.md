# MCP Tests details

For the dynamic routing we use two MCP servers one "user" and other "admin".

Probably it can be simplified. For now we use two different docker images.

- User container uses `ghcr.io/peterj/mcp-website-fetcher:main` as a source and is copied to the CI registry on GitHub as `ghcr.io/kgateway-dev/mcp-website-fetcher:0.0.1`
- Admin container is built using upstream and placed `ghcr.io/kgateway-dev/mcp-admin-server:0.0.1` it can be rebuilt using Dockerfile in this directory. Use `docker build -t ghcr.io/kgateway-dev/mcp-admin-server:<version> .` and then push it to GH with `docker push ghcr.io/kgateway-dev/mcp-admin-server:<version>` command

## MCP Authentication with 0Auth mock server

Apply the configuration from the e2e tests `testdata/auth0-mock-server.yaml`, `testdata/common.yaml`, `testdata/static.yaml` and `remote-authn-auth0.yaml`.
The IDP server is a mock server that mocks the 0Auth 2.0 flows and returns a hard coded JWT token.

Port-forward the gateway:
```shell
kubectl port-forward svc/gw 8080:8080
```

You can manually test the mcp authentication is applied by sending a request with an invalid authorization header:
```shell
❯ curl -X POST http://localhost:8080 \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -H "mcp-protocol-version: 2025-06-18" \
  -H "Authorization: bearer fake" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2025-06-18",
      "capabilities": {},
      "clientInfo": {
        "name": "curl-client",
        "version": "1.0"
      }
    }
  }'
{"error":"unauthorized","error_description":"JWT token required"}%
```

Note: with optional mode, you should still get a response `mode: Optional`, but a request with an invalid token will fail.

Then with the correct token:
```shell
export TOKEN=eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzMzM3ODA2ODc1NTEwMzg2NTkifQ.eyJhdWQiOiJhY2NvdW50IiwiZXhwIjoyMDc5MTA1MTE0LCJpYXQiOjE3NjM3NDUxMTQsImlzcyI6Imh0dHBzOi8va2dhdGV3YXkuZGV2Iiwic3ViIjoidXNlckBrZ2F0ZXdheS5kZXYifQ.W0n1xEPD6dl5CYLi_TEMzqn9REGgN7-MIaivvmHHzUAqsD-Gox2NQ79KFEGMqlZwbfc0p34xloR2dJ616nU9NxqSyBssFgDhRDGnasSwHM6AvbpEEPEK7J_lCbfnaxqEQm8_AhXPgFEY4zbQq3WQ7OE7wQpSPjcAL1PB01SRE5UZsYW_bXqup_2MqVzahCFagrQtOPHN3sCUeLz8dm5DAPgat9WQmiDaUP-_yT_tk4J7MH6SolHBnxRwrP8nhUf9N9bi-hADnmCLTKO7BmP0xBQo-abRlu_5Ug6YAfMirHfrO09EvXDCVWuk1d35GCyApPxPhwtZg40kOq-BXaWwFw

❯ curl -X POST http://localhost:8080 \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -H "mcp-protocol-version: 2025-06-18" \
  -H "Authorization: bearer $TOKEN" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2025-06-18",
      "capabilities": {},
      "clientInfo": {
        "name": "curl-client",
        "version": "1.0"
      }
    }
  }'
```

And you should get a response:
```shell
data: {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"prompts":{},"resources":{},"tools":{}},"serverInfo":{"name":"rmcp","version":"0.8.5"},"instructions":"This server is a gateway to a set of mcp servers. It is responsible for routing requests to the correct server and aggregating the results."}}
```

You can also test with the MCP inspector via the Guided OAuth tool and the mock mcp inspector config. 

First make sure the idp listener is port-forward on the gateway:
```shell
kubectl port-forward svc/gw 8081:8081
```

Then click `Open Auth Settings` in the mcp inspector follow the guided refresh token flow to setup the tokens to use automatically for server requests.

## Keycloak MCP Authentication

For testing MCP Authentication with [keycloak](https://github.com/keycloak/keycloak), apply the idp configuration and the following policy
to the same mcp-backend from the previous Auth0 example:

Setup a simple keycloak instance:
```sh
kubectl apply -f - <<EOF
apiVersion: v1
kind: ConfigMap
metadata:
  name: keycloak-realm
data:
  mcp-realm.json: |
    {
      "realm": "mcp",
      "enabled": true,
      "clients": [
        {
          "clientId": "mcp_proxy",
          "publicClient": false,
          "serviceAccountsEnabled": true,
          "clientAuthenticatorType": "client-secret",
          "secret": "supersecret",
          "directAccessGrantsEnabled": true,
          "redirectUris": ["*"]
        }
      ]
    }
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: keycloak
  labels:
    app: keycloak
spec:
  replicas: 1
  selector:
    matchLabels:
      app: keycloak
  template:
    metadata:
      labels:
        app: keycloak
    spec:
      containers:
        - name: keycloak
          image: quay.io/keycloak/keycloak:26.3
          args:
            - start-dev
            - --import-realm
            - --http-port
            - "7080"
            - --https-port
            - "7443"
          ports:
            - name: http
              containerPort: 7080
            - name: https
              containerPort: 7443
          env:
            - name: KC_HOSTNAME
              value: keycloak
            - name: KC_HOSTNAME_PORT
              value: "7080"
            - name: KC_HOSTNAME_STRICT_BACKCHANNEL
              value: "true"
            - name: KEYCLOAK_ADMIN
              value: admin
            - name: KEYCLOAK_ADMIN_PASSWORD
              value: admin
            - name: KC_HEALTH_ENABLED
              value: "true"
            - name: KC_LOG_LEVEL
              value: info
          volumeMounts:
            - name: realm-import
              mountPath: /opt/keycloak/data/import
      volumes:
        - name: realm-import
          configMap:
            name: keycloak-realm
            items:
              - key: mcp-realm.json
                path: mcp-realm.json
---
apiVersion: v1
kind: Service
metadata:
  name: keycloak
spec:
  selector:
    app: keycloak
  ports:
    - name: http
      port: 7080
      targetPort: 7080
    - name: https
      port: 7443
      targetPort: 7443
  type: ClusterIP
EOF
```

Set up the MCP authentication policy:
```sh
kubectl apply -f - <<EOF
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayPolicy
metadata:
  name: keycloak-mcp-authn-policy
spec:
  targetRefs:
    - name: mcp-backend
      kind: AgentgatewayBackend
      group: agentgateway.dev
  backend:
    mcp:
      authentication:
        issuer: http://keycloak:7080/realms/mcp
        jwks:
          uri: http://keycloak.default.svc.cluster.local:7080/realms/mcp/protocol/openid-connect/certs
        audiences:
        - "account"
        provider: Keycloak
        resourceMetadata:
          resource: http://mcp-website-fetcher.default.svc.cluster.local/mcp
          scopesSupported: '["tools/call/fetch"]'
          bearerMethodsSupported: '["header"]'
          resourceDocumentation: http://mcp-website-fetcher.default.svc.cluster.local/docs
          resourcePolicyUri: http://mcp-website-fetcher.default.svc.cluster.local/policies
EOF
```

Port-forward the gateway:
```shell
kubectl port-forward svc/gw 8080:8080
```

Send a request with a fake authorization header:
```shell
curl -v -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Accept: application/json,text/event-stream" \
  -H "Authorization: Bearer fake" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2025-06-18",
      "capabilities": {},
      "clientInfo": {
        "name": "curl-client",
        "version": "1.0"
      }
    }
  }'
```

You should get a 401 Unauthorized response:
```shell
< HTTP/1.1 401 Unauthorized
< www-authenticate: Bearer resource_metadata="http://mcp-website-fetcher.default.svc.cluster.local/.well-known/oauth-protected-resource/mcp"
< content-type: application/json
< content-length: 65
< date: Thu, 20 Nov 2025 14:32:26 GMT
< 
* Connection #0 to host localhost left intact
{"error":"unauthorized","error_description":"JWT token required"}%
```

From the `curl` pod you should be able to get a token from keycloak under `access_token` and save it as `TOKEN`:
```shell
curl -s -X POST "http://keycloak.default:7080/realms/mcp/protocol/openid-connect/token" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  -d "client_id=mcp_proxy" \
  -d "client_secret=supersecret"
```

Then use the token to make the request:
```shell
curl -v -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Accept: application/json,text/event-stream" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2025-06-18",
      "capabilities": {},
      "clientInfo": {
        "name": "curl-client",
        "version": "1.0"
      }
    }
  }'
```

You can also test this with the MCP inspector:
1. Run `npx modelcontextprotocol/inspector#0.16.2`
2. Open the inspector UI
3. Attempt to connect to the port-forwarded gateway (`http://localhost:8080/`) without the token using Streamable HTTP
4. Set the `TOKEN` under the API Token Authentication field, then click Connect
5. Go to the `tools` tab and test the `fetch` tool with a URL of your choice

Note: By default, agentgateway will use `Optional` authentication mode, allowing requests without an authorization header.