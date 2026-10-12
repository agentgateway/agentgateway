# Kubernetes Response Cache

Configure response caching on an existing agentgateway installation using vSR,
Redis, and HomeHub. See the [overview](../README.md) or the
[standalone example](../standalone/README.md).

## Prerequisites

- An existing Kubernetes cluster. For local testing, see the
  [Kind quick start](https://kind.sigs.k8s.io/docs/user/quick-start/).
- [agentgateway installed](https://agentgateway.dev/docs/kubernetes/latest/documentation/quickstart/install/),
  including its controller, CRDs, and GatewayClass `agentgateway`.
  This example was tested with agentgateway v1.6.0.
- kubectl, Helm, curl, jq, and uuidgen, with your kubectl context selected.
- A default StorageClass for Redis and enough capacity for vSR's embedding
  model (the tested local environment had 6 CPUs, 10 GiB RAM, and 15 GiB disk).

The manifests use `agentgateway-system` and deploy HomeHub in a dedicated
`homehub` namespace. Adjust names and references for your existing installation.
Run commands from the repository root in the same shell:

```bash
export RESPONSE_CACHE_DIR=examples/llm-semantic-routing/response-cache/kubernetes
source "${RESPONSE_CACHE_DIR}/versions.env"
```

## Configure the Gateway

Apply the example Gateway configuration. To use an existing Gateway instead,
update the route's parent reference and omit `gateway.yaml`:

```bash
kubectl apply -f "${RESPONSE_CACHE_DIR}/gateway.yaml"
kubectl wait --for=condition=Programmed gateway/agentgateway-proxy \
  -n agentgateway-system --timeout=300s
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

Deploy the [HomeHub demo image](https://github.com/solo-io/homehub-support-demo)
in the `homehub` namespace. It returns fixed OpenAI Chat Completions responses
and counts requests so you can verify cache hits.

```bash
kubectl apply -f "${RESPONSE_CACHE_DIR}/support-backend.yaml"
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
  --set-string "image.tag=${VSR_IMAGE_TAG}"

kubectl wait --for=condition=Available deployment/semantic-router \
  -n agentgateway-system \
  --timeout=600s
```

The version settings pin the vSR v0.4.0 image and chart 0.4.0.

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

## Verify the Cache

In one terminal, port-forward the gateway:

```bash
kubectl port-forward -n agentgateway-system service/agentgateway-proxy 8080:80
```

In another terminal, set the gateway URL and follow the
[shared four-request walkthrough](../README.md#verify-the-cache):

```bash
export GATEWAY_URL=http://127.0.0.1:8080
```

Check the backend count before and after requests:

```bash
kubectl exec -n homehub deployment/support-backend -- python -c \
  'import urllib.request; print(urllib.request.urlopen("http://localhost:8080/stats").read().decode())'
```

Stop the port-forward with Control-C when finished.

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

## Cleanup

Remove the integration resources in dependency order. Keep the existing
Gateway and agentgateway installation.

```bash
kubectl delete \
  -f "${RESPONSE_CACHE_DIR}/agentgateway-routing.yaml"
helm uninstall semantic-router -n agentgateway-system
kubectl delete \
  -f "${RESPONSE_CACHE_DIR}/support-backend.yaml"
kubectl delete \
  -f "${RESPONSE_CACHE_DIR}/redis.yaml"
```

If you created `gateway/agentgateway-proxy` for this example, delete it
separately after removing the routes.

Deleting `redis.yaml` also deletes its StatefulSet but might retain its PVC,
depending on the Kubernetes StatefulSet retention policy. Inspect and delete
the dedicated `data-redis-semantic-cache-0` PVC explicitly if you no longer
need the cached data.
