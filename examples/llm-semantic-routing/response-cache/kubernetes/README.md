# Kubernetes Response Cache

Run agentgateway v1.5.0, vSR, Redis Open Source, and the HomeHub Python backend
in a local kind cluster. No LLM credentials are required because HomeHub
returns fixed answers without calling an LLM provider. See the
[overview](../README.md) for the cache policy and request flow, or use the
[standalone example](../standalone/README.md) to run with Docker Compose.

## Before You Begin

Install these tools:

- Docker
- kind 0.29.0 or later
- kubectl
- Helm
- curl
- jq

The example uses Kubernetes 1.36, agentgateway 1.5.0, the current vSR chart and
image, and Redis Open Source 8.10.0. vSR downloads an embedding model on first
startup, so allocate at least 6 CPUs, 10 GiB of memory, and 15 GiB of free disk
space to Docker for the model and the other services.

## Create the Cluster

Run the commands below from the repository root.

```bash
export RESPONSE_CACHE_DIR=examples/llm-semantic-routing/response-cache/kubernetes
source "${RESPONSE_CACHE_DIR}/versions.env"

kind create cluster \
  --config "${RESPONSE_CACHE_DIR}/kind-config.yaml"
```

The kind context is `kind-semantic-cache`:

```bash
kubectl config use-context kind-semantic-cache
```

## Install agentgateway

Install the Gateway API and agentgateway 1.5.0:

```bash
kubectl apply --server-side --force-conflicts \
  -f https://github.com/kubernetes-sigs/gateway-api/releases/download/${GATEWAY_API_VERSION}/standard-install.yaml

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
  -f "${RESPONSE_CACHE_DIR}/gateway.yaml"

kubectl wait --for=condition=Programmed gateway/agentgateway-proxy \
  -n agentgateway-system \
  --timeout=300s
kubectl rollout status deployment/agentgateway-proxy \
  -n agentgateway-system \
  --timeout=300s
```

## Deploy Redis Open Source

Redis runs as a single pod with a 256 MiB persistent volume for its append-only
file and snapshots.

> [!NOTE]
> Redis 8 includes the vector-search support vSR needs.

```bash
kubectl apply \
  -f "${RESPONSE_CACHE_DIR}/redis.yaml"

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

Deploy the backend in the `homehub` namespace and create a ConfigMap from the
[shared Python backend](../shared/support-backend.py). It returns fixed OpenAI
Chat Completions responses and counts requests so you can verify cache hits.

```bash
kubectl apply -f "${RESPONSE_CACHE_DIR}/support-backend.yaml"
kubectl create configmap support-backend -n homehub \
  --from-file="server.py=${RESPONSE_CACHE_DIR}/../shared/support-backend.py" \
  --dry-run=client -o yaml | kubectl apply -f -
kubectl rollout restart deployment/support-backend -n homehub
kubectl rollout status deployment/support-backend -n homehub --timeout=120s
```

## Install vSR

Install vSR with the Redis response-cache backend:

```bash
helm upgrade -i semantic-router \
  oci://ghcr.io/vllm-project/charts/semantic-router \
  --version "${VSR_CHART_VERSION}" \
  --namespace agentgateway-system \
  -f "${RESPONSE_CACHE_DIR}/semantic-router-values.yaml" \
  --set-string "image.tag=${VSR_IMAGE_TAG}" \
  --set "image.pullPolicy=Always"

kubectl wait --for=condition=Available deployment/semantic-router \
  -n agentgateway-system \
  --timeout=600s
```

The version settings keep vSR on `latest` and its chart on `0.0.0-latest`,
as explained in the [overview](../README.md).

Confirm that vSR connected to Redis and initialized its index:

```bash
kubectl logs -n agentgateway-system deployment/semantic-router \
  | grep -i -E 'redis|semantic.cache'

kubectl exec -n agentgateway-system statefulset/redis-semantic-cache \
  -- redis-cli --raw FT._LIST
```

## Configure Routing

Apply the HomeHub provider, route, and vSR ExtProc policy:

> [!NOTE]
> HomeHub does not stream responses, so ExtProc streaming is disabled and
> requests use `stream: false`. Agentgateway sends complete request and response
> bodies to vSR, which can cache the response in Redis.

```bash
kubectl apply \
  -f "${RESPONSE_CACHE_DIR}/agentgateway-routing.yaml"

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
"${RESPONSE_CACHE_DIR}/verify.sh"
```

The script resets the backend counter and the example's Redis cache, then
checks:

| Request | Expected cache result | Expected backend count |
| --- | --- | ---: |
| X2 factory-reset request | Miss | 1 |
| Exact repeat | Hit | 1 |
| X2 factory-reset paraphrase | Hit | 1 |
| HomeHub outdoor-use request | Miss | 2 |

The script checks `x-vsr-cache-hit`, backend counts, and Redis Search indexes.

> [!NOTE]
> vSR derives index names from the embedding model, so the names can differ
> from the configured `semantic_cache_idx`.

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
    "stream": false,
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

This test caches a response, starts a second vSR replica, and removes the
original pod. It then checks that a new pod can return the cached response:

```bash
"${RESPONSE_CACHE_DIR}/verify.sh" --shared-vsr
```

The script restores the Deployment to one replica when it exits.

## Optional: Restart Redis

The Redis manifest enables AOF persistence on a PVC. Verify recovery after a
Redis pod restart:

```bash
"${RESPONSE_CACHE_DIR}/verify.sh" --redis-restart
```

Both optional checks can run together:

```bash
"${RESPONSE_CACHE_DIR}/verify.sh" \
  --shared-vsr \
  --redis-restart
```

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
model and workload-specific. See the [cache correctness guidance](https://agentgateway.dev/docs/kubernetes/main/integrations/llm/routing/vllm-semantic-router/#response-caching-in-production)
before adjusting the threshold.

### The Backend Count Is Unexpected

Inspect the backend history:

```bash
kubectl port-forward -n homehub service/support-backend 18081:8080
curl -sS http://127.0.0.1:18081/stats | jq
```

## Cleanup

Remove the example cluster:

```bash
kind delete cluster --name semantic-cache
```

To retain the cluster, delete resources in dependency order:

```bash
kubectl delete \
  -f "${RESPONSE_CACHE_DIR}/agentgateway-routing.yaml"
helm uninstall semantic-router -n agentgateway-system
kubectl delete \
  -f "${RESPONSE_CACHE_DIR}/support-backend.yaml"
kubectl delete \
  -f "${RESPONSE_CACHE_DIR}/redis.yaml"
kubectl delete \
  -f "${RESPONSE_CACHE_DIR}/gateway.yaml"
helm uninstall agentgateway -n agentgateway-system
helm uninstall agentgateway-crds -n agentgateway-system
```

Deleting `redis.yaml` also deletes its StatefulSet but might retain its PVC,
depending on the Kubernetes StatefulSet retention policy. Inspect and delete
the dedicated `data-redis-semantic-cache-0` PVC explicitly if you no longer
need the cached data.
