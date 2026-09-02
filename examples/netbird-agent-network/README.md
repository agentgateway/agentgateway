# NetBird Agent Network with agentgateway

This example places [agentgateway](https://agentgateway.dev/) behind a
[NetBird Agent Network](https://netbird.ai/) endpoint. NetBird authenticates
the caller, applies Agent Network policy, replaces identity headers with
trusted values, and forwards the request to a private agentgateway listener.
Agentgateway authenticates NetBird with a virtual API key and routes OpenAI
and Anthropic requests to their respective providers.

This document uses Kubernetes. See the
[standalone Docker Compose example](standalone/README.md) for a self-contained
deployment on one Docker host.

### Management traffic

```text
NetBird client
    |
    | management HTTPS
    v
public management agentgateway
    |-- HTTP/1.1 paths ----------------------\
    |                                         > private NetBird server
    `-- Management and Signal gRPC -- h2c ---/
```

The management agentgateway has a public LoadBalancer and terminates HTTPS for
NetBird clients. It sends the HTTP APIs and relay WebSocket endpoint over
HTTP/1.1, and Management and Signal gRPC over HTTP/2 cleartext, to two private
ClusterIP Services that select the same NetBird server pods. The protocol split
preserves the WebSocket upgrade headers while continuing to support management
and signal gRPC. A NetworkPolicy permits only the management agentgateway to
reach the server.

The two Services give agentgateway distinct backend protocol hints even though
both select the same pods and target port. `netbird-server-relay` preserves the
HTTP/1.1 upgrade required by the relay WebSocket, while `netbird-server` declares
h2c for Management and Signal gRPC. This example intentionally keeps relay on
the shared HTTPS endpoint; native QUIC would require a separately exposed UDP
relay. See NetBird's [external reverse proxy setup][netbird-reverse-proxy] for
the complete protocol and routing requirements.

[netbird-reverse-proxy]: https://docs.netbird.io/selfhosted/external-reverse-proxy

### Agent Network traffic

```text
NetBird client
    |
    | HTTPS over the encrypted NetBird tunnel
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

The NetBird Agent Network proxy has a public LoadBalancer and accepts requests
from authorized NetBird clients. It replaces caller-supplied identity headers
with trusted NetBird identity, adds the virtual API key, and forwards requests
to the private AI agentgateway Service. A NetworkPolicy permits only the
NetBird proxy to reach this gateway. Agentgateway routes Anthropic message
requests by path and sends all other requests to OpenAI.

Authorized clients reach the Agent Network proxy through NetBird's encrypted
WireGuard overlay. NetBird DNS resolves the generated endpoint to the proxy
peer's tunnel IP, while HTTPS provides an additional layer of transport
encryption and server authentication. The public LoadBalancer supports
certificate issuance and public-path denial checks; normal authorized requests
use the NetBird tunnel.

## Temporary NetBird images

This example temporarily uses NetBird server and proxy development images built
from the agentgateway implementation through commit `de8635e00`, plus a
dashboard image built from the dashboard integration branch:

```text
danehans/netbird-server:agw-e2e-de8635e00@sha256:7a284a036f7a3206b603848048ae5c312cdf006fe3d718e2262ad0433c815d68
danehans/netbird-proxy:agw-e2e-de8635e00@sha256:5e082ea45eecc78630e4a5a1a26708bf5fdadb831fef00433a5dc5628f19bc6d
danehans/netbird-dashboard:agw-e2e-2208b98d4-20260824-175445@sha256:f4ae567bb502c45b3c5384cf5d2af0539ffe00a0ffe12689a49ed9547d4fe3b0
```

These personal test images are not NetBird production releases. Replace them
in `versions.env` with the first official `netbirdio/netbird-server` and
`netbirdio/netbird-proxy` release that contains the fix for
[netbirdio/netbird#6970](https://github.com/netbirdio/netbird/issues/6970).
Replace the dashboard image with the first official NetBird dashboard release
that contains [netbirdio/dashboard#774][dashboard-pr].

[dashboard-pr]: https://github.com/netbirdio/dashboard/pull/774

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

The `1Gi` proxy certificate claim in `netbird.yaml` is a conservative,
portable default, not a NetBird capacity requirement. A small deployment
normally uses only tens of KiB for its ACME account and endpoint certificates.
You can request a smaller volume if your StorageClass supports it. For a
disposable test, you can remove the `netbird-proxy-certs` claim and replace the
proxy's `certs` volume with `emptyDir: {}`. An `emptyDir` cache is lost whenever
the pod is replaced or rescheduled, causing the proxy to request certificates
again and increasing the risk of ACME validation failures or rate limits.

The example was tested with agentgateway 1.5.0, cert-manager 1.21.1, Gateway
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
kubectl apply -f agent-network-gateway.yaml
envsubst < management-gateway.yaml | kubectl apply -f -
```

The proxy and client pods initially wait for secrets created by
`configure.sh`. The NetBird server can start independently.

### Optional dashboard

Deploy the NetBird dashboard if you want to inspect the API-created resources
or complete the provider and policy configuration through the UI:

```bash
envsubst < dashboard.yaml | kubectl apply -f -
kubectl rollout status deployment/netbird-dashboard \
  -n netbird-agent-network --timeout=5m
```

The dashboard reuses the management hostname, LoadBalancer, and certificate; it
does not require another public Service or DNS record. The base management
route explicitly sends NetBird HTTP and gRPC paths to the server. The optional
dashboard route handles the remaining paths on the same HTTPS listener. The
embedded NetBird IdP configuration in `secrets.example.yaml` registers the
dashboard's `/nb-auth` and `/nb-silent-auth` OAuth callback paths.

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

Repeated installations that reuse these DNS names may receive different
LoadBalancer addresses. DNS resolvers, operating systems, browsers, and ACME
certificate authorities can continue using the previous addresses until their
caches expire. Before requesting certificates or running verification, confirm
that the DNS names resolve to the addresses reported by the current Services.
Flush local DNS caches where appropriate, but allow time for the record TTL to
expire when an upstream resolver still has the old value. For frequently
recreated environments, consider reserving static LoadBalancer addresses or
lowering the DNS TTL before changing the records.

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

The two `kubectl wait` commands finish with `condition met` when cert-manager's
Issuer is ready and the management TLS Secret contains a valid certificate.
For a fresh database, the final request should return:

```json
{"setup_required":true}
```

This confirms that HTTPS routing reaches the NetBird management server and the
server is ready for the initial owner configuration in step 5. A retained or
previously initialized database returns `{"setup_required":false}` instead.

## 5. Configure NetBird

Use one of the following configuration paths. Both create the shared proxy
token, client group, and one-use setup key. They produce the same final
configuration and use the same verification script.

### Automated API configuration

The default mode performs all configuration through the management API:

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

The script is idempotent: rerunning it reuses resources with the expected
names. It does not overwrite an existing provider or policy. Run
`./configure.sh --check` to detect an existing resource whose important fields
do not match the example.

### Dashboard-assisted configuration

First deploy the optional dashboard described in step 3. Then run the shared
bootstrap without creating the Agent Network endpoint, provider, or policy:

```bash
./configure.sh --mode dashboard
```

Sign in at `https://${NETBIRD_MANAGEMENT_DOMAIN}` using the
`NETBIRD_ADMIN_EMAIL` and `NETBIRD_ADMIN_PASSWORD` values exported in step 1,
then complete these steps:

On the first sign-in, NetBird offers **Peer-to-Peer Network** and **Remote
Network Access** onboarding paths. Select **Skip to Dashboard** below those
options. Those paths configure general-purpose NetBird connectivity, while
`configure.sh --mode dashboard` has already prepared the proxy token, client
group, setup key, and Kubernetes workloads required by this example.

1. Open **Agent Network > Providers** and add a provider.
2. Select **agentgateway**, name it `agentgateway`, and set the upstream URL to
   `http://netbird-agentgateway.netbird-agent-network.svc.cluster.local`.
3. Enter the current `NETBIRD_VIRTUAL_KEY`, leave the model list empty to allow
   all models, keep the provider enabled, and keep identity metadata enabled.
   Saving the first provider also creates the generated Agent Network endpoint.
4. Open **Agent Network > Policies** and create an enabled policy named
   `Agentgateway access`. Select `agentgateway-clients` as its source group and
   `agentgateway` as its destination provider. Click **Continue** without making
   changes in the **Limits** and **Guardrails** steps.

Validate the result with a PAT created in or copied from the dashboard:

```bash
export NETBIRD_PAT=nbp_replace_me
./configure.sh --check
```

`--check` is read-only. It validates the endpoint, provider, group, policy, and
Kubernetes Secrets, then prints the generated endpoint to export for step 6.
Choose either the API or dashboard path for a given installation; using both to
create the same resources is unnecessary.

## 6. Verify the integration

The default verification is non-billable. It checks resource readiness, the
public management endpoint, the relay WebSocket upgrade, strict virtual-key
rejection, and that an unauthenticated public caller cannot bypass NetBird:

```bash
./verify.sh
```

Enable live provider calls after reviewing the selected model IDs and their
costs:

```bash
export RUN_LIVE_PROVIDER_TESTS=true
export OPENAI_MODEL=gpt-4o-mini
export ANTHROPIC_MODEL=claude-haiku-4-5
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

Call each configured model backend from the same NetBird client. These requests
use the upstream provider APIs and may incur charges:

```bash
export OPENAI_MODEL=${OPENAI_MODEL:-gpt-4o-mini}
openai_body=$(jq -cn --arg model "${OPENAI_MODEL}" '{
  model: $model,
  messages: [{role: "user", content: "Reply with the word connected."}],
  max_tokens: 16
}')
kubectl exec -n netbird-agent-network deployment/netbird-example-client \
  -c test -- curl -fsS \
  "https://${NETBIRD_AGENT_ENDPOINT}/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  --data-binary "${openai_body}" | jq

export ANTHROPIC_MODEL=${ANTHROPIC_MODEL:-claude-haiku-4-5}
anthropic_body=$(jq -cn --arg model "${ANTHROPIC_MODEL}" '{
  model: $model,
  max_tokens: 16,
  messages: [{role: "user", content: "Reply with the word connected."}]
}')
kubectl exec -n netbird-agent-network deployment/netbird-example-client \
  -c test -- curl -fsS \
  "https://${NETBIRD_AGENT_ENDPOINT}/v1/messages" \
  -H 'Content-Type: application/json' \
  --data-binary "${anthropic_body}" | jq
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

The `x-netbird-groups` header is a sorted CSV of display names for attribution.
It is not a delimiter-safe set of stable group IDs and must not be used as an
agentgateway authorization claim.

The private AI listener on the `netbird-agentgateway` Gateway must remain
unreachable except through the NetBird proxy. Strict API-key authentication
protects the hop, but the shared key by itself does not make caller-supplied
identity headers trustworthy. If your CNI does not enforce Kubernetes
NetworkPolicy, apply equivalent controls with a service mesh, firewall, or
private network.

**Production hardening:** This example uses HTTP between the NetBird Agent
Network proxy and the private AI agentgateway listener. NetworkPolicy restricts
that listener to the NetBird proxy but does not encrypt pod-to-pod traffic.
Production deployments that require encryption within the cluster should use
HTTPS, service-mesh mTLS, or encrypted pod networking. Do not expose the
listener until equivalent identity-header trust-boundary controls are in place.

## Pricing behavior

NetBird meters requests using the model name and pricing catalog it sends to
the proxy. Recognized upstream model IDs use NetBird catalog defaults. Custom
agentgateway aliases require explicit NetBird model rows and rates. An unknown
alias remains routable but records `unknown_model` with zero cost.

The example uses Anthropic's `claude-haiku-4-5` alias because it matches a
NetBird catalog entry. A pinned snapshot such as
`claude-haiku-4-5-20251001` remains routable, but requires an explicit NetBird
model row and rates for cost accounting.

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
- `Unregistered redirect_uri` on dashboard login means the embedded IdP was
  started without the dashboard callback URIs from `secrets.example.yaml`.
  Apply the rendered Secret, restart `netbird-server`, and clear any stale
  browser OAuth state before retrying.
- Inspect `AgentgatewayBackend`, `AgentgatewayPolicy`, `HTTPRoute`, and Gateway
  status conditions before looking at pod logs.

## Cleanup

For the normal disposable installation, delete the dedicated namespace and its
namespace-local NetBird database:

```bash
./cleanup.sh
```

Use `--management` only when the NetBird management database will survive
namespace deletion, such as an external PostgreSQL database or retained volume,
and the example-owned account configuration should also be removed:

```bash
export NETBIRD_PAT=nbp_replace_me
./cleanup.sh --management
```

This optional management cleanup stops the example proxy and client before
deleting the named Agent Network policy, provider, setup key, active proxy
token, and Agent Network settings. The order satisfies the provider-reference
and active-proxy deletion guards. It then deletes the dedicated namespace.

The `agentgateway-clients` group is retained because `configure.sh` reuses an
existing group with that name. The cleanup script cannot safely determine
whether the group was created for this example or shared with another setup.
Delete it separately only after confirming that nothing else uses it.

The script does not uninstall shared Gateway API, cert-manager, or agentgateway
control-plane components, and it does not remove DNS records. Deleting the
namespace deletes the example's PVC objects. With the usual `Delete` reclaim
policy, that also deletes the NetBird database. A retained volume or snapshot
can preserve the database independently of the namespace.

## Tracking

- [agentgateway/agentgateway#2757](https://github.com/agentgateway/agentgateway/issues/2757)
- [netbirdio/netbird#6970](https://github.com/netbirdio/netbird/issues/6970)

## Next Steps

This example maps NetBird's trusted identity headers to the `agentgateway.user`
and `agentgateway.group` request-log attributes, but it does not enable the
request-log database, model catalog, or Admin UI access needed for
agentgateway-side usage dashboards.

- Follow the Kubernetes [cost dashboard][agentgateway-cost-dashboard] guide to
  record request data, configure model pricing, and view requests, tokens, and
  cost by NetBird user or authorizing group. Without a model catalog, requests
  and tokens remain available but cost is reported as zero.
- Review the Kubernetes [Admin UI][agentgateway-admin-ui] guide before exposing
  the UI beyond a local port-forward or another private administrative path.

The `x-netbird-groups` value is stored as one CSV string. For example,
`Engineering,Platform` appears as one combined group dimension rather than two
separate groups.

[agentgateway-cost-dashboard]: https://agentgateway.dev/docs/kubernetes/latest/llm/cost-controls/dashboard/
[agentgateway-admin-ui]: https://agentgateway.dev/docs/kubernetes/latest/observability/ui/
