# Semantic Cache with Redis Open Source

This example combines agentgateway, [vLLM Semantic Router
(vSR)](https://vllm-sr.ai/), and Redis Open Source to reuse responses for
semantically equivalent product-support requests. It runs entirely in a local
kind cluster and does not require an LLM provider credential.

The deterministic HomeHub support backend makes cache behavior observable:

- The first HomeHub X2 factory-reset request reaches the backend.
- An exact repeat and an X2 paraphrase return the cached response.
- An outdoor-use request misses and reaches the backend.
- Optionally, another vSR replica can return an entry written before that
  replica started.
- Optionally, the Redis pod can restart and recover the cache from its volume.

vSR supports multiple semantic-cache backends, including its default in-memory
store. The in-memory backend is useful for development because it requires no
external service, but its entries are local to one vSR process and disappear
when that process restarts. This example chooses Redis as a production-oriented
backend because it provides shared, persistent cache state and can fit an
existing Redis operational model. Redis is also used by other agentgateway
features; for example, the [agentgateway global rate-limiting
guide](https://agentgateway.dev/docs/kubernetes/main/security/rate-limit-global/)
deploys a Redis-backed rate-limit service.

See the [vSR semantic-cache documentation](https://vllm-semantic-router.com/docs/tutorials/plugin/semantic-cache/),
[Redis vector-search documentation](https://redis.io/docs/latest/develop/interact/search-and-query/query/vector-search/),
and [Redis persistence documentation](https://redis.io/docs/latest/operate/oss_and_stack/management/persistence/)
for details. This remains a local functional example, not a highly available
production Redis deployment.

## Request Flow

```text
Client
  |
  v
agentgateway
  |
  +----> vSR ExtProc ----> Redis semantic-cache lookup
  |                          |
  |                          `---- hit: cached response
  |
  `---- miss: deterministic support backend
             |
             `---- response passes through vSR and is stored in Redis
```

agentgateway invokes vSR as an ExtProc before selecting and calling the
configured HomeHub backend. On a cache hit, vSR returns the cached completion
as an immediate ExtProc response. agentgateway sends that response to the
client and stops upstream processing, so the HomeHub Service and its
`AgentgatewayBackend` are not called.

On a cache miss, vSR allows upstream processing to continue and agentgateway
routes the request to the deterministic HomeHub backend. agentgateway sends the
completed backend response through vSR, which stores it in Redis before the
response is returned to the client. The verification script demonstrates this
behavior by warming the cache once, then confirming that exact and paraphrased
cache hits return successfully without increasing the HomeHub backend's
invocation counter.

The cache plugin is attached only to the factory-reset decision; other support
intents select an uncached decision.

## Before You Begin

Install these tools:

- Docker
- kind 0.29.0 or later
- kubectl
- Helm
- curl
- jq

The example uses Kubernetes 1.36, agentgateway 1.4.1, the current vSR chart and
image, and Redis Open Source 8.10.0. See the [agentgateway version-support
reference](https://agentgateway.dev/docs/kubernetes/main/reference/versions/)
for the supported Kubernetes and Gateway API versions. vSR downloads an
embedding model on its first startup. Allocate at least 6 CPUs, 10 GiB of
memory, and 15 GiB of free disk space to Docker.

This local kind setup uses cleartext connections from agentgateway to vSR's
ExtProc endpoint and from vSR to Redis. Production deployments should use TLS
for both connections:

- Serve the vSR ExtProc gRPC endpoint with TLS, either directly or through a
  TLS-terminating sidecar, and configure agentgateway to originate TLS to the
  `semantic-router` Service. See the agentgateway [ExtProc
  guide](https://agentgateway.dev/docs/kubernetes/main/traffic-management/extproc/)
  and [BackendTLS
  guide](https://agentgateway.dev/docs/kubernetes/main/security/backendtls/).
- Configure Redis with a server certificate, private key, and trusted CA as
  described in the [Redis Open Source TLS
  documentation](https://redis.io/docs/latest/operate/oss_and_stack/management/security/encryption/).
  Mount the required CA and client certificate material into vSR, then enable
  TLS in the semantic-cache Redis connection. See the vSR [Redis semantic-cache
  configuration](https://vllm-semantic-router.com/docs/v0.2/tutorials/semantic-cache/redis-cache/).

TLS encrypts traffic in transit. Configure Redis authentication and ACLs
separately, and store credentials and certificate private keys in Kubernetes
Secrets rather than inline configuration.

## Create the Cluster

```bash
kind create cluster \
  --config examples/llm-semantic-routing/k8s/semantic-cache/kind-config.yaml
```

The kind context is `kind-semantic-cache`:

```bash
kubectl config use-context kind-semantic-cache
```

## Install agentgateway

Install the Gateway API and agentgateway 1.4.1:

```bash
export AGENTGATEWAY_VERSION=v1.4.1

kubectl apply --server-side --force-conflicts \
  -f https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.6.0/standard-install.yaml

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

Create the proxy used by this example:

```bash
kubectl apply \
  -f examples/llm-semantic-routing/k8s/semantic-cache/gateway.yaml

kubectl wait --for=condition=Programmed gateway/agentgateway-proxy \
  -n agentgateway-system \
  --timeout=300s
kubectl rollout status deployment/agentgateway-proxy \
  -n agentgateway-system \
  --timeout=300s
```

## Deploy Redis Open Source

The manifest deploys the official Redis 8 image as a single-replica
StatefulSet. A 256 MiB persistent volume stores its append-only file and
periodic snapshots. This is ample for the example's small cache dataset and
AOF rewrite overhead. Redis 8 includes Redis Search and vector-search support.

For a production deployment, size Redis storage for the expected number of
cache entries, embedding and response sizes, cache TTL, dataset growth, and
temporary disk space required during AOF rewrites.

```bash
kubectl apply \
  -f examples/llm-semantic-routing/k8s/semantic-cache/redis.yaml

kubectl rollout status statefulset/redis-semantic-cache \
  -n agentgateway-system \
  --timeout=180s
```

Verify Redis and Redis Search:

```bash
kubectl exec -n agentgateway-system statefulset/redis-semantic-cache \
  -- redis-cli PING

kubectl exec -n agentgateway-system statefulset/redis-semantic-cache \
  -- redis-cli COMMAND INFO FT.SEARCH
```

The commands should report `PONG` and information about `FT.SEARCH`.

## Deploy the Support Backend

The backend implements the OpenAI Chat Completions response API with the
Python standard library. It returns fixed answers and records every invocation.
The manifest creates a dedicated `homehub` namespace so the mock application
is isolated from the agentgateway control plane and supporting services.

```bash
kubectl apply \
  -f examples/llm-semantic-routing/k8s/semantic-cache/support-backend.yaml

kubectl wait --for=condition=Available deployment/support-backend \
  -n homehub \
  --timeout=120s
```

## Install vSR

Install vSR with the Redis semantic-cache backend:

```bash
export VSR_CHART_VERSION=0.0.0-latest
export VSR_IMAGE_TAG=latest
export SEMANTIC_CACHE_DIR=examples/llm-semantic-routing/k8s/semantic-cache

helm upgrade -i semantic-router \
  oci://ghcr.io/vllm-project/charts/semantic-router \
  --version "${VSR_CHART_VERSION}" \
  --namespace agentgateway-system \
  -f "${SEMANTIC_CACHE_DIR}/semantic-router-values.yaml" \
  --set-string "image.tag=${VSR_IMAGE_TAG}" \
  --set "image.pullPolicy=Always"

kubectl wait --for=condition=Available deployment/semantic-router \
  -n agentgateway-system \
  --timeout=600s
```

The default tracks the latest vSR chart and image because the other vSR
examples in this repository rely on fixes newer than the last release. For a
repeatable deployment, replace both values with a tested chart and image pair.

Confirm that vSR connected to Redis and initialized its index:

```bash
kubectl logs -n agentgateway-system deployment/semantic-router \
  | grep -i -E 'redis|semantic.cache'

kubectl exec -n agentgateway-system statefulset/redis-semantic-cache \
  -- redis-cli FT.INFO semantic_cache_idx
```

## Configure Routing

Apply the mock provider, route, and vSR ExtProc policy:

```bash
kubectl apply \
  -f examples/llm-semantic-routing/k8s/semantic-cache/agentgateway-routing.yaml

kubectl wait --for=condition=Accepted agentgatewaybackend/homehub-support \
  -n agentgateway-system \
  --timeout=300s
kubectl describe httproute homehub-support -n agentgateway-system
kubectl describe agentgatewaypolicy semantic-cache-extproc \
  -n agentgateway-system
```

## Run the Verification

Run the automated verification from the repository root:

```bash
examples/llm-semantic-routing/k8s/semantic-cache/verify.sh
```

The script resets only the example's mock counter and Redis key prefix, then
asserts:

| Request | Expected cache result | Expected backend count |
| --- | --- | ---: |
| X2 factory-reset request | Miss | 1 |
| Exact repeat | Hit | 1 |
| X2 factory-reset paraphrase | Hit | 1 |
| HomeHub outdoor-use request | Miss | 2 |

It checks `x-vsr-cache-hit` rather than treating lower latency as proof of a
cache hit. It also verifies that `semantic_cache_idx` exists in Redis.

### Inspect a Request Manually

Port-forward the gateway:

```bash
kubectl port-forward \
  -n agentgateway-system \
  service/agentgateway-proxy 8080:80
```

In another terminal:

```bash
curl -sS -i http://127.0.0.1:8080/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -H 'X-VSR-Debug: true' \
  -H 'X-Request-ID: semantic-cache-manual-1' \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "How do I factory-reset my HomeHub X2?"}
    ],
    "max_tokens": 96
  }'
```

On a hit, the response includes `x-vsr-cache-hit: true`. With the debug header,
vSR also exposes the cache similarity when available.

The request ID lets vSR correlate the request-side cache entry with the
completed response that it stores. Use a unique `X-Request-ID` for each request.

## Optional: Share the Cache Across vSR Replicas

This test warms Redis with the original vSR process, scales vSR to two replicas,
deletes the original pod, and verifies that a paraphrase is still a cache hit:

```bash
examples/llm-semantic-routing/k8s/semantic-cache/verify.sh --shared-vsr
```

The script restores the Deployment to one replica when it exits. This test
demonstrates that the cache is shared through Redis rather than stored only in a
vSR instance.

## Optional: Restart Redis

The Redis manifest enables AOF persistence on a PVC. Verify recovery after a
Redis pod restart:

```bash
examples/llm-semantic-routing/k8s/semantic-cache/verify.sh --redis-restart
```

Both optional checks can run together:

```bash
examples/llm-semantic-routing/k8s/semantic-cache/verify.sh \
  --shared-vsr \
  --redis-restart
```

This restart test demonstrates local volume persistence. It does not make the
single Redis pod highly available.

## Entity-Sensitive Cache Matches

Do not assume that changing only `HomeHub X2` to `HomeHub X3` guarantees a
miss. General-purpose embedding models can consider those requests highly
similar even though returning the X2 procedure for an X3 would be incorrect.

The required negative control therefore changes the intent from factory reset
to safe outdoor use. vSR selects the uncached support decision, so the request
always reaches the backend. Treat X2-versus-X3 as an additional false-hit
regression test when tuning the embedding model and similarity threshold for
real product traffic.

## Troubleshooting

### vSR Does Not Become Available

The initial model download can take several minutes:

```bash
kubectl describe pod -n agentgateway-system \
  -l app.kubernetes.io/instance=semantic-router
kubectl logs -n agentgateway-system deployment/semantic-router --tail=200
kubectl get pvc -n agentgateway-system
```

Increase Docker's memory allocation if the vSR pod is `OOMKilled` or remains
unschedulable.

### Redis Index Is Missing

Check Redis Search and vSR's connection settings:

```bash
kubectl exec -n agentgateway-system statefulset/redis-semantic-cache \
  -- redis-cli COMMAND INFO FT.CREATE
kubectl logs -n agentgateway-system deployment/semantic-router \
  | grep -i -E 'redis|cache|index'
```

The configured vector dimension, `768`, must match the embedding model.

### A Paraphrase Misses

Inspect the debug response headers and vSR logs. Similarity thresholds are
model- and workload-specific. Lowering the threshold can improve recall but
also increases unsafe false hits, especially when product identifiers are the
only difference.

### The Backend Count Is Unexpected

Inspect the backend history:

```bash
kubectl port-forward -n homehub service/support-backend 18081:8080
curl -sS http://127.0.0.1:18081/stats | jq
```

## Security and Production Considerations

This local example intentionally omits TLS and Redis authentication. A
production design should include:

- Redis authentication and TLS
- Kubernetes NetworkPolicies
- Redis replication and automated failover
- Backups and restore testing
- Resource and eviction policies
- A cache-key policy that accounts for tenant, authorization, model, tools,
  system prompts, and entity-sensitive content
- Conservative similarity thresholds and false-hit evaluation

## Cleanup

The simplest cleanup removes the entire kind cluster:

```bash
kind delete cluster --name semantic-cache
```

To retain the cluster, delete resources in dependency order:

```bash
kubectl delete \
  -f examples/llm-semantic-routing/k8s/semantic-cache/agentgateway-routing.yaml
helm uninstall semantic-router -n agentgateway-system
kubectl delete \
  -f examples/llm-semantic-routing/k8s/semantic-cache/support-backend.yaml
kubectl delete \
  -f examples/llm-semantic-routing/k8s/semantic-cache/redis.yaml
kubectl delete \
  -f examples/llm-semantic-routing/k8s/semantic-cache/gateway.yaml
helm uninstall agentgateway -n agentgateway-system
helm uninstall agentgateway-crds -n agentgateway-system
```

Deleting `redis.yaml` also deletes its StatefulSet but might retain its PVC,
depending on the Kubernetes StatefulSet retention policy. Inspect and delete
the dedicated `data-redis-semantic-cache-0` PVC explicitly if you no longer
need the cached data.
