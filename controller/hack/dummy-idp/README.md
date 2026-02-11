Based on agentgateway auth0 example: https://github.com/agentgateway/agentgateway/tree/485a299d2199cf7f792a8f7e55677f1a4898013b/examples/mcp-authentication

## Building and Deploying

1. Build docker with the Make target 

```shell
VERSION=<version> make kind-build-and-load-dummy-idp
```

2. Apply the mock IDP deployment 

```shell
kubectl apply -f test/e2e/features/agentgateway/mcp/testdata/auth0-mock-server.yaml
```

## Running Locally

### Option 1: Using Docker (without Kubernetes)

Build and run the Docker container locally:

```shell
# Build the Go binary and Docker image (will rebuild if source changed)
make dummy-idp-docker

# Run the container
docker run -p 8443:8443 ghcr.io/kgateway-dev/dummy-idp:0.0.1
```

**Note:** If you see "Nothing to be done for 'dummy-idp'", that's normal - it means the binary is already built and up to date. The `dummy-idp-docker` target will automatically rebuild the binary if source files have changed.

**Alternative: Build with a custom tag**
```shell
# Build the binary first (or force rebuild by deleting _output/hack/dummy-idp/dummy-idp-linux-amd64)
make dummy-idp

# Build Docker image with custom tag (from the output directory)
docker build -f _output/hack/dummy-idp/Dockerfile.dummy-idp \
  --build-arg BASE_IMAGE=alpine:3.17.6 \
  --build-arg GOARCH=amd64 \
  -t dummy-idp:local \
  _output/hack/dummy-idp

# Run the container
docker run -p 8443:8443 dummy-idp:local
```

**Force a rebuild:** If you've made code changes and want to force a rebuild:
```shell
# Remove the binary to force rebuild
rm -f _output/hack/dummy-idp/dummy-idp-linux-<arch>
make dummy-idp-docker
```

**Quick run command:**
```shell
docker run -p 8443:8443 ghcr.io/kgateway-dev/dummy-idp:0.0.1
```

The server will be available at `https://localhost:8443`.

### Option 2: Using Kubernetes Port-Forward

To test the dummy-idp server locally via Kubernetes, port-forward the service:

```shell
kubectl port-forward svc/auth0-mock 8443:8443
```

The server runs on HTTPS port 8443 with a self-signed certificate. When using `curl`, use the `-k` flag to skip certificate verification.

## Testing Endpoints

### OAuth2 Discovery Endpoint

Get the OAuth2 discovery document:

```shell
curl -k https://localhost:8443/.well-known/oauth-authorization-server
```

### JWKS Endpoint

Get the JSON Web Key Set:

```shell
curl -k https://localhost:8443/.well-known/jwks.json
```

### Client Registration

Register a new OAuth2 client:

```shell
curl -k -X POST https://localhost:8443/register \
  -H "Content-Type: application/json"
```

### Authorization Endpoint

Request authorization (returns redirect URL with code):

```shell
curl -k "https://localhost:8443/authorize?client_id=mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo&redirect_uri=http://localhost:8081/callback"
```

### Token Endpoint

Exchange authorization code for access token:

```shell
curl -k -X POST https://localhost:8443/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code" \
  -d "client_id=mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo" \
  -d "code=fixed_auth_code_123" \
  -d "redirect_uri=http://localhost:8081/callback"
```

Or use Basic authentication:

```shell
curl -k -X POST https://localhost:8443/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -u "mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo:secret_2nGx_bjvo9z72Aw3-hKTWMusEo2-yTfH" \
  -d "grant_type=authorization_code" \
  -d "code=fixed_auth_code_123" \
  -d "redirect_uri=http://localhost:8081/callback"
```

### Refresh Token

A request to "refresh" an access token will return the same token since this is a mock IDP server for testing. You can try the following request to refresh the token:

```shell
curl -k -X POST https://localhost:8443/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -u "mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo:secret_2nGx_bjvo9z72Aw3-hKTWMusEo2-yTfH" \
  -d "grant_type=refresh_token" \
  -d "refresh_token=fixed_refresh_token_123"
```

### Organization-specific Endpoints

Get JWKS for different organizations:

```shell
curl -k https://localhost:8443/org-one/keys
curl -k https://localhost:8443/org-two/keys
curl -k https://localhost:8443/org-three/keys
```

Get JWT tokens for different organizations:

```shell
curl -k https://localhost:8443/org-one/jwt
curl -k https://localhost:8443/org-two/jwt
curl -k https://localhost:8443/org-three/jwt
```

## Hardcoded Values for MCP Auth Flow

The mock server will always return the same JWT token and key pair, which has a 10-year expiration date set. The values for 
the token returned are:
```json
{
  "iss": "https://kgateway.dev",
  "sub": "ignore@kgateway.dev",
  "exp": 2071163407,
  "nbf": 1763579407,
  "iat": 1763579407
}
```

**Hardcoded OAuth2 values:**
- Client ID: `mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo`
- Client Secret: `secret_2nGx_bjvo9z72Aw3-hKTWMusEo2-yTfH`
- Authorization Code: `fixed_auth_code_123`
- Refresh Token: `fixed_refresh_token_123`
- Redirect URI: `http://localhost:8081/callback`

The mock IDP provider will use a hard coded code `fixed_auth_code_123` for the `handle_authorize` endpoint. The mock
server also sets the redirect url to `http://localhost:8081/callback`, so when testing the MCP Inspector locally, 
make sure to expose this through the gateway and route configuration.  