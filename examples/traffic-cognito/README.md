## Amazon Cognito JWT Example

This example validates Amazon Cognito access tokens with `jwtAuth` and lets only members of the `admins` group reach `/admin`. It runs on the standalone binary or on Kubernetes, with the same policy in both.

### Prerequisites

- The AWS CLI, signed in with permission to manage Cognito user pools
- Standalone: `curl` and `python3`
- Kubernetes: a cluster with [agentgateway installed](https://agentgateway.dev/docs/kubernetes/latest/documentation/quickstart/install/), `kubectl`, and `envsubst` (from GNU gettext)

### Running the example

Create a user pool, an app client without a secret, and two users, `alice` and `bob`. Add `alice` to the `admins` group, then sign in to get tokens. Run the remaining steps in the same shell, because they read these variables:

```bash
export AWS_REGION='<your region>'
PASSWORD='<8+ characters with upper, lower, number and symbol; no commas>'
COGNITO_USER_POOL_ID=$(aws cognito-idp create-user-pool --pool-name agentgateway-demo \
  --admin-create-user-config AllowAdminCreateUserOnly=true --query UserPool.Id --output text)
COGNITO_CLIENT_ID=$(aws cognito-idp create-user-pool-client --user-pool-id $COGNITO_USER_POOL_ID \
  --client-name agentgateway-demo --no-generate-secret \
  --explicit-auth-flows ALLOW_USER_PASSWORD_AUTH ALLOW_REFRESH_TOKEN_AUTH \
  --query UserPoolClient.ClientId --output text)
export COGNITO_USER_POOL_ID COGNITO_CLIENT_ID

for user in alice bob; do
  aws cognito-idp admin-create-user --user-pool-id $COGNITO_USER_POOL_ID --username $user --message-action SUPPRESS
  aws cognito-idp admin-set-user-password --user-pool-id $COGNITO_USER_POOL_ID --username $user --password "$PASSWORD" --permanent
done
aws cognito-idp create-group --user-pool-id $COGNITO_USER_POOL_ID --group-name admins
aws cognito-idp admin-add-user-to-group --user-pool-id $COGNITO_USER_POOL_ID --username alice --group-name admins

token() {
  aws cognito-idp initiate-auth --auth-flow USER_PASSWORD_AUTH --client-id $COGNITO_CLIENT_ID \
    --auth-parameters USERNAME=$1,PASSWORD="$PASSWORD" \
    --query AuthenticationResult.${2:-AccessToken} --output text
}
ALICE=$(token alice)
BOB=$(token bob)
ALICE_ID=$(token alice IdToken)
```

Then start the gateway with one of the two options below. Both serve on `localhost:3000`.

#### Standalone

Start an upstream HTTP server and agentgateway in the background. The config reads `AWS_REGION`, `COGNITO_USER_POOL_ID`, and `COGNITO_CLIENT_ID` from the environment:

```bash
curl -sL https://agentgateway.dev/install | bash
python3 -m http.server 8080 &

cat > config.yaml <<'EOF'
binds:
- port: 3000
  listeners:
  - routes:
    - policies:
        jwtAuth:
          mode: strict
          issuer: https://cognito-idp.${AWS_REGION}.amazonaws.com/${COGNITO_USER_POOL_ID}
          # Cognito access tokens have no `aud` claim, so the client is checked below instead.
          jwks:
            url: https://cognito-idp.${AWS_REGION}.amazonaws.com/${COGNITO_USER_POOL_ID}/.well-known/jwks.json
        authorization:
          rules:
          - require: 'jwt.token_use == "access" && jwt.client_id == "${COGNITO_CLIENT_ID}"'
          - require: '!request.path.startsWith("/admin") || "admins" in jwt["cognito:groups"]'
      backends:
      - host: localhost:8080
EOF
agentgateway -f config.yaml &
```

#### Kubernetes

Substitute the same variables into the manifest, apply it, wait for the Gateway, and forward its port in the background:

```bash
envsubst <<'EOF' | kubectl apply -f -
apiVersion: v1
kind: Namespace
metadata:
  name: cognito-demo
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: httpbin
  namespace: cognito-demo
spec:
  selector:
    matchLabels:
      app: httpbin
  template:
    metadata:
      labels:
        app: httpbin
    spec:
      containers:
      - name: httpbin
        image: mccutchen/go-httpbin:2.26.0
        ports:
        - containerPort: 8080
---
apiVersion: v1
kind: Service
metadata:
  name: httpbin
  namespace: cognito-demo
spec:
  selector:
    app: httpbin
  ports:
  - port: 8080
---
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: cognito
  namespace: cognito-demo
spec:
  gatewayClassName: agentgateway
  listeners:
  - name: http
    port: 3000
    protocol: HTTP
---
apiVersion: gateway.networking.k8s.io/v1
kind: HTTPRoute
metadata:
  name: httpbin
  namespace: cognito-demo
spec:
  parentRefs:
  - name: cognito
  rules:
  - backendRefs:
    - name: httpbin
      port: 8080
---
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayPolicy
metadata:
  name: cognito-jwt
  namespace: cognito-demo
spec:
  targetRefs:
  - group: gateway.networking.k8s.io
    kind: HTTPRoute
    name: httpbin
  traffic:
    jwtAuthentication:
      mode: Strict
      providers:
      - issuer: https://cognito-idp.${AWS_REGION}.amazonaws.com/${COGNITO_USER_POOL_ID}
        # Cognito access tokens have no `aud` claim, so the client is checked below instead.
        jwks:
          remote:
            url: https://cognito-idp.${AWS_REGION}.amazonaws.com/${COGNITO_USER_POOL_ID}/.well-known/jwks.json
    authorization:
      action: Require
      policy:
        matchExpressions:
        - 'jwt.token_use == "access" && jwt.client_id == "${COGNITO_CLIENT_ID}"'
        - '!request.path.startsWith("/admin") || "admins" in jwt["cognito:groups"]'
EOF
kubectl -n cognito-demo wait --for=condition=Programmed gateway/cognito --timeout=120s
kubectl -n cognito-demo rollout status deploy/cognito deploy/httpbin --timeout=120s
kubectl -n cognito-demo port-forward deploy/cognito 3000:3000 &
```

### Sending requests

Send requests with no token, an invalid token, Bob's and Alice's access tokens, and Alice's ID token:

```bash
curl -i localhost:3000/
curl -i localhost:3000/ -H "Authorization: Bearer invalid"
curl -i localhost:3000/ -H "Authorization: Bearer $BOB"
curl -i localhost:3000/admin -H "Authorization: Bearer $BOB"
curl -i localhost:3000/admin -H "Authorization: Bearer $ALICE"
curl -i localhost:3000/ -H "Authorization: Bearer $ALICE_ID"
```

A request without a token, or with an invalid one, gets a `401`.
Bob gets a `200` on `/` and a `403` on `/admin`.
Alice passes both checks, so the gateway forwards her `/admin` request to the backend, which returns a `404` because the example backends have no `/admin` page. Alice's ID token gets a `403`.

### How the policy works

Cognito access tokens have no `aud` claim, so the config omits `audiences`; setting `audiences` to the client ID rejects every access token with a `401`. The first `require` rule checks `client_id` and `token_use` instead, which also rejects ID tokens.

`mode: strict` rejects a request that has no token with a `401` before the authorization rules run. Standalone `jwtAuth` defaults to `optional`, which passes that request on to the authorization rules, so it gets a `403` instead. Kubernetes `jwtAuthentication` defaults to `Strict`.

The group claim is named `cognito:groups`, so CEL reads it with index syntax: `jwt["cognito:groups"]`.

On Kubernetes, `jwks.remote.url` takes the full JWKS URL: the issuer followed by `/.well-known/jwks.json`. The policy's `authorization` has one `action` and a list of `matchExpressions`, and each expression is evaluated as its own rule. With `action: Require`, every expression has to match. With the default `Allow`, any one matching expression admits the request.

### Cleanup

Stop the background processes, delete the Kubernetes namespace if you used it, and delete the user pool:

```bash
kill $(jobs -p)
kubectl delete namespace cognito-demo
aws cognito-idp delete-user-pool --user-pool-id $COGNITO_USER_POOL_ID
```
