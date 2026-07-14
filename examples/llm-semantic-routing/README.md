# Cost-Based Semantic Routing with vLLM Semantic Router

This example configures agentgateway and [vLLM Semantic Router (vSR)](https://vllm-semantic-router.com/)
to route OpenAI-compatible chat traffic to a lower-cost or higher-capability
model. vLLM Semantic Router classifies the request, selects a model, and
agentgateway forwards the request to OpenAI.

The included policy is tuned for coding prompts: routine implementation,
refactoring, unit tests, documentation, and simple debugging go to
`gpt-5.4-nano`. It escalates advanced distributed-systems design, formal
verification, difficult debugging, and research synthesis to `gpt-5.5`.
Its advanced keyword signal uses literal high-specificity phrases, while the
remaining semantic, complexity, context, and structure signals handle less
obvious requests.
Customize the signals, candidates, weights, and thresholds for your traffic by
following the [vLLM Semantic Router configuration guide](https://vllm-semantic-router.com/docs/installation/configuration/).

## Before You Begin

This example assumes a working agentgateway LLM path with cost and
observability data available:

- [Install agentgateway with Helm](https://agentgateway.dev/docs/kubernetes/main/install/helm/).
- [Set up an agentgateway proxy](https://agentgateway.dev/docs/kubernetes/main/setup/gateway/).
- [Configure OpenAI as an LLM provider](https://agentgateway.dev/docs/kubernetes/main/llm/providers/openai/).
- [Price LLM requests with a model cost catalog](https://agentgateway.dev/docs/kubernetes/main/llm/costs/).
- [Install an OpenTelemetry stack](https://agentgateway.dev/docs/kubernetes/main/observability/otel-stack/).

The `AgentgatewayBackend` in `k8s/agentgateway-routing.yaml` expects an
`openai-secret` in `agentgateway-system`, matching the provider setup guide.

## Configure Routing

Replace any existing `HTTPRoute` attached to this Gateway that matches
`/v1/chat/completions` or `/v1/responses` before applying this example.

Install vLLM Semantic Router:

```bash
export VSR_CHART_VERSION=0.0.0-latest
export VSR_IMAGE_TAG=latest

helm upgrade -i semantic-router oci://ghcr.io/vllm-project/charts/semantic-router \
  --version "${VSR_CHART_VERSION}" \
  --namespace agentgateway-system \
  -f examples/llm-semantic-routing/k8s/semantic-router-values.yaml \
  --set-string "image.tag=${VSR_IMAGE_TAG}" \
  --set "image.pullPolicy=Always"

kubectl wait --for=condition=Available deployment/semantic-router \
  -n agentgateway-system \
  --timeout=600s
```

Apply the routed backend, route, and streamed ExtProc policy:

```bash
kubectl apply -f examples/llm-semantic-routing/k8s/agentgateway-routing.yaml

kubectl wait --for=condition=Accepted agentgatewaybackend/openai-router-selected \
  -n agentgateway-system \
  --timeout=300s
kubectl describe httproute openai-semantic-routing -n agentgateway-system
kubectl describe agentgatewaypolicy semantic-router-extproc -n agentgateway-system
```

This example defaults to the latest vSR chart and image, which include the
[Responses API streaming fix](https://github.com/vllm-project/semantic-router/issues/2446)
and the [FullDuplexStreamed request-body fix](https://github.com/vllm-project/semantic-router/issues/2486).
For a repeatable historical deployment, override both values with released
chart and image versions that contain both fixes.

## Verify Streamed ExtProc

Set your gateway address:

```bash
export INGRESS_GW_ADDRESS="http://$(kubectl get gateway agentgateway-proxy \
  -n agentgateway-system \
  -o jsonpath='{.status.addresses[0].value}')"
```

The values include a narrow, deterministic immediate-response probe. It proves
that `FullDuplexStreamed` request processing reaches vSR without sending tokens
to OpenAI:

```bash
curl -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -H "X-VSR-Debug: true" \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "VSR_IMMEDIATE_RESPONSE_PROBE"}
    ],
    "max_tokens": 16
  }'
```

Expect a `200` response with `x-vsr-fast-response`; the request should not
reach OpenAI. Remove the probe signal and decision before using this policy in
a production route.

## Run a Request

Routine coding prompts should use the lower-cost model:

```bash
curl -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -H "X-VSR-Debug: true" \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "Implement a small Go helper and one table-driven test."}
    ],
    "max_tokens": 64
  }'
```

Advanced distributed-systems prompts should use the higher-capability model:

```bash
curl -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -H "X-VSR-Debug: true" \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "Design a distributed rate limiter that remains correct during Redis failover and regional network partitions. Compare token bucket, sliding window, local fallback, and global reconciliation."}
    ],
    "max_tokens": 64
  }'
```

The response headers should include `x-vsr-selected-model: gpt-5.4-nano` for
the routine request and `x-vsr-selected-model: gpt-5.5` for the advanced
request. The debug header is for verification only and should not be required
by application traffic.

Agentgateway’s model catalog, metrics, logs, and traces remain the cost and
observability source of record. Use isolated evaluation traffic with forced
lower-cost and always-expensive baselines before adopting the policy broadly.

## Optional: Use Codex Through the Gateway

The gateway works with any OpenAI API-compatible client or agent. This optional
section shows a Codex-specific configuration. Codex uses the OpenAI Responses
API, and the nightly vSR image configured above translates streamed Responses
events so it can send the same `auto` model name through the gateway. Configure
a user-level Codex profile:

```toml
# ~/.codex/agentgateway.config.toml
model = "auto"
model_provider = "agentgateway"

[model_providers.agentgateway]
name = "Corporate agentgateway"
base_url = "https://my.corp.agentgateway.com/v1"
wire_api = "responses"
env_key = "OPENAI_API_KEY"
```

Set your OpenAI API key, then start Codex with the profile:

```bash
export OPENAI_API_KEY='sk-...'
codex --profile agentgateway
```

The gateway authenticates to OpenAI with its configured provider credential and
records the selected model and cost as it does for other OpenAI-compatible
clients. Agentgateway can [rewrite client-facing model names with model
aliases](https://agentgateway.dev/docs/kubernetes/latest/llm/alias/). An
organization can also [validate and reject unsupported request-body model
values](https://agentgateway.dev/docs/kubernetes/latest/traffic-management/transformations/validate/),
such as any value other than `auto`. Treat `auto` as the supported client path
when testing this policy.

## Cleanup

```bash
kubectl delete -f examples/llm-semantic-routing/k8s/agentgateway-routing.yaml
helm uninstall semantic-router -n agentgateway-system
```
