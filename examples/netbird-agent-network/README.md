# NetBird Agent Network with agentgateway

This example places [agentgateway](https://agentgateway.dev/) behind a
[NetBird Agent Network](https://netbird.ai/) endpoint. NetBird authenticates
the caller, applies Agent Network policy, replaces identity headers with
trusted values, and forwards the request to a private agentgateway listener.
Agentgateway authenticates NetBird with a virtual API key and routes OpenAI
and Anthropic requests to their respective providers.

```text
NetBird client
    |
    | management HTTPS
    v
public management agentgateway
    |
    | HTTP/2 cleartext inside the cluster
    v
private NetBird server

NetBird client
    |
    | generated Agent Network HTTPS endpoint
    v
NetBird proxy
    | Authorization: Bearer <virtual key>
    | x-netbird-user-id: <trusted identity>
    | x-netbird-groups: <trusted group display names>
    v
private agentgateway listener
    |-- /v1/messages ------------> Anthropic
    `-- all other paths ---------> OpenAI
```

The manifests use public LoadBalancer Services for the management agentgateway
and Agent Network proxy. The NetBird server and AI agentgateway Services are
ClusterIP-only. NetworkPolicies permit only the management agentgateway to
reach the server and only the NetBird proxy to reach the AI gateway.

## Temporary NetBird images

This example temporarily uses development images built from the NetBird
agentgateway implementation through commit `de8635e00`:

```text
danehans/netbird-server:agw-e2e-de8635e00@sha256:7a284a036f7a3206b603848048ae5c312cdf006fe3d718e2262ad0433c815d68
danehans/netbird-proxy:agw-e2e-de8635e00@sha256:5e082ea45eecc78630e4a5a1a26708bf5fdadb831fef00433a5dc5628f19bc6d
```

These personal test images are not NetBird production releases. Replace them
in `versions.env` with the first official `netbirdio/netbird-server` and
`netbirdio/netbird-proxy` release that contains the fix for
[netbirdio/netbird#6970](https://github.com/netbirdio/netbird/issues/6970).

## Prerequisites

- A Kubernetes cluster with a default StorageClass and LoadBalancer support.
  The StorageClass dynamically provisions volumes for NetBird server state and
  the Agent Network proxy's ACME certificate cache. Alternatively, set
  `storageClassName` on both claims in `netbird.yaml` or pre-provision matching
  volumes.
- `kubectl`, Helm, `curl`, `envsubst`, `jq`, and OpenSSL.
- Three DNS records in a domain you control.
- OpenAI and Anthropic API credentials.
- Nodes that expose `/dev/net/tun` and permit a privileged test pod. If this is
  not acceptable in your cluster, connect an external NetBird client and omit
  the `netbird-example-client` Deployment.

The example was tested with agentgateway 1.4.1, cert-manager 1.21.1, Gateway
API 1.6.0, and the NetBird 0.77.0 client. All versions are pinned in
`versions.env`.

## 1. Set variables

Run these commands from this directory:

```bash
set -a
source versions.env
set +a

export NETBIRD_MANAGEMENT_DOMAIN=netbird.example.com
export NETBIRD_PROXY_DOMAIN=agents.example.com
export NETBIRD_LETSENCRYPT_EMAIL=admin@example.com

export NETBIRD_ADMIN_EMAIL=admin@example.com
export NETBIRD_ADMIN_PASSWORD='replace-with-a-strong-password'
export OPENAI_API_KEY='replace-with-an-openai-key'
export ANTHROPIC_API_KEY='replace-with-an-anthropic-key'

export NETBIRD_AUTH_SECRET=$(openssl rand -base64 32)
export NETBIRD_SESSION_KEY=$(openssl rand -base64 32)
export NETBIRD_STORE_KEY=$(openssl rand -base64 32)
export NETBIRD_VIRTUAL_KEY=$(openssl rand -hex 32)
export NETBIRD_VIRTUAL_KEY_SHA256=$(printf '%s' "${NETBIRD_VIRTUAL_KEY}" \
  | openssl dgst -sha256 -r | awk '{print $1}')
```

Keep these values in a password manager. In particular, rerunning the example
with a different virtual key without updating both systems will cause
agentgateway to reject NetBird requests.

## 2. Install the controllers

Install Gateway API, cert-manager with Gateway API support, and the pinned
agentgateway charts:

```bash
kubectl apply --server-side --force-conflicts \
  -f "https://github.com/kubernetes-sigs/gateway-api/releases/download/${GATEWAY_API_VERSION}/standard-install.yaml"

helm upgrade -i cert-manager \
  oci://quay.io/jetstack/charts/cert-manager \
  --create-namespace \
  --namespace cert-manager \
  --version "${CERT_MANAGER_VERSION}" \
  --set crds.enabled=true \
  --set config.gatewayAPI.enabled=true \
  --wait

helm upgrade -i agentgateway-crds \
  oci://cr.agentgateway.dev/charts/agentgateway-crds \
  --create-namespace \
  --namespace agentgateway-system \
  --version "${AGENTGATEWAY_VERSION}"

helm upgrade -i agentgateway \
  oci://cr.agentgateway.dev/charts/agentgateway \
  --namespace agentgateway-system \
  --version "${AGENTGATEWAY_VERSION}" \
  --wait
```

## 3. Create secrets and workloads

`secrets.example.yaml` contains placeholders only. Render it directly to
`kubectl` so a populated file is not written to the repository:

```bash
kubectl create namespace netbird-agent-network \
  --dry-run=client -o yaml | kubectl apply -f -

envsubst < secrets.example.yaml | kubectl apply -f -
envsubst < netbird.yaml | kubectl apply -f -
kubectl apply -f agentgateway.yaml
envsubst < management-gateway.yaml | kubectl apply -f -
```

The proxy and client pods initially wait for secrets created by
`configure.sh`. The NetBird server can start independently.

Wait for the public addresses:

```bash
kubectl get service netbird-management netbird-proxy \
  -n netbird-agent-network --watch
```

## 4. Create DNS records

Create these records after the LoadBalancer addresses are assigned:

| Name | Target |
| --- | --- |
| `${NETBIRD_MANAGEMENT_DOMAIN}` | `netbird-management` LoadBalancer address |
| `${NETBIRD_PROXY_DOMAIN}` | `netbird-proxy` LoadBalancer address |
| `*.${NETBIRD_PROXY_DOMAIN}` | CNAME to `${NETBIRD_PROXY_DOMAIN}` |

cert-manager obtains the management certificate with an HTTP-01 challenge
through the management Gateway. The Agent Network proxy obtains its
certificate with a TLS-ALPN-01 challenge. Wait for the management certificate
and endpoint:

```bash
kubectl wait --for=condition=Ready issuer/netbird-letsencrypt \
  -n netbird-agent-network --timeout=5m
kubectl wait --for=condition=Ready certificate/netbird-management \
  -n netbird-agent-network --timeout=10m
curl -fsS "https://${NETBIRD_MANAGEMENT_DOMAIN}/api/instance"
```

## 5. Configure NetBird

The configuration script performs the API operations used in the validated
environment:

- Creates the initial owner and a 30-day setup PAT, unless `NETBIRD_PAT` is
  already set.
- Creates the account-scoped proxy token and Kubernetes Secret.
- Bootstraps a generated endpoint below the proxy domain.
- Creates the `agentgateway` Agent Network provider.
- Creates a source group and Agent Network access policy.
- Creates a one-use setup key that automatically adds the test peer to the
  authorized group.

```bash
./configure.sh
```

The script prints the generated hostname. Export it for verification:

```bash
export NETBIRD_AGENT_ENDPOINT=<generated-hostname>
```

If the NetBird instance was already initialized, omit the admin password and
set a PAT instead:

```bash
export NETBIRD_PAT=nbp_replace_me
./configure.sh
```

## 6. Verify the integration

The default verification is non-billable. It checks resource readiness,
strict virtual-key rejection, and that an unauthenticated public caller cannot
bypass NetBird:

```bash
./verify.sh
```

Enable live provider calls after reviewing the selected model IDs and their
costs:

```bash
export RUN_LIVE_PROVIDER_TESTS=true
export OPENAI_MODEL=gpt-4o-mini
export ANTHROPIC_MODEL=claude-haiku-4-5-20251001
./verify.sh
```

The live checks cover model listing, OpenAI Chat Completions, OpenAI SSE, and
Anthropic Messages through the generated NetBird endpoint.

### Manual requests

Run a request inside the authorized NetBird client pod:

```bash
kubectl exec -n netbird-agent-network deployment/netbird-example-client \
  -c test -- curl -fsS \
  "https://${NETBIRD_AGENT_ENDPOINT}/v1/models" | jq
```

An unauthenticated request from outside the NetBird client must be denied:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' \
  "https://${NETBIRD_AGENT_ENDPOINT}/v1/models"
# 403
```

The private agentgateway listener also rejects missing or invalid virtual
keys. Port-forwarding is intended only for this diagnostic:

```bash
kubectl port-forward -n netbird-agent-network \
  service/netbird-agentgateway 18080:80

curl -sS -o /dev/null -w '%{http_code}\n' \
  http://127.0.0.1:18080/v1/models
# 401
```

## Identity and trust boundary

NetBird removes caller-supplied `x-netbird-user-id` and `x-netbird-groups`
headers and adds values derived from the authenticated NetBird caller. The
AgentgatewayParameters resource maps these headers to the
`agentgateway.user` and `agentgateway.group` standard request-log attributes.

The groups header is a sorted CSV of display names for attribution. It is not
a delimiter-safe set of stable group IDs and must not be used as an
agentgateway authorization claim.

The agentgateway listener must remain unreachable except through the NetBird
proxy. Strict API-key authentication protects the hop, but the shared key by
itself does not make caller-supplied identity headers trustworthy. If your CNI
does not enforce Kubernetes NetworkPolicy, apply equivalent controls with a
service mesh, firewall, or private network.

## Pricing behavior

NetBird meters requests using the model name and pricing catalog it sends to
the proxy. Recognized upstream model IDs use NetBird catalog defaults. Custom
agentgateway aliases require explicit NetBird model rows and rates. An unknown
alias remains routable but records `unknown_model` with zero cost.

A single static NetBird price cannot exactly represent a dynamic alias that
load-balances among differently priced models. Use direct model names or an
operator-defined approximation when NetBird-side spend accounting must be
exact.

## Troubleshooting

- `401` from the private agentgateway listener usually means the raw virtual
  key stored by NetBird does not match the SHA-256 value in the agentgateway
  Secret.
- `403` from the public Agent Network endpoint means the request did not arrive
  from a peer authorized by the Agent Network policy.
- A pending NetBird client commonly means `/dev/net/tun` or privileged pods are
  unavailable. Use an external disposable peer in that case.
- Management certificate failures usually indicate that its A record does not
  point to the `netbird-management` LoadBalancer or TCP 80 is filtered. Agent
  Network proxy certificate failures usually indicate that TCP 443 is
  filtered or its DNS records point to the wrong LoadBalancer.
- Inspect `AgentgatewayBackend`, `AgentgatewayPolicy`, `HTTPRoute`, and Gateway
  status conditions before looking at pod logs.

## Cleanup

```bash
./cleanup.sh
```

The script removes the dedicated NetBird namespace and only the agentgateway
resources owned by this example. It does not uninstall shared Gateway API,
cert-manager, or agentgateway control-plane components, and it does not remove
DNS records. Deleting the namespace also deletes the example's PVCs and
NetBird database; that data is not recoverable unless the storage system
retains a snapshot.

## Tracking

- [agentgateway/agentgateway#2757](https://github.com/agentgateway/agentgateway/issues/2757)
- [netbirdio/netbird#6970](https://github.com/netbirdio/netbird/issues/6970)
