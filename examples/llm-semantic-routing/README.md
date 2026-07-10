# Cost-Based Semantic Routing with vLLM Semantic Router

This example configures agentgateway and [vLLM Semantic Router (vSR)](https://vllm-semantic-router.com/)
to route OpenAI-compatible chat traffic to a lower-cost or higher-capability
model. vSR classifies the request, selects a model, and agentgateway forwards
the request to OpenAI.

The included policy sends routine implementation work to `gpt-5.4-nano` and
escalates advanced distributed-systems design, formal verification, difficult
debugging, and research synthesis to `gpt-5.5`.

This is the reusable integration configuration. For the three-lane benchmark,
catalog-backed cost report, observability checks, corpora, and result chart, use
the [cost-based semantic-routing demo](https://github.com/danehans/agentgateway-demos/tree/main/cost-based-semantic-routing).

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

Do not apply this route beside an existing `HTTPRoute` with the same Gateway and
`/v1/chat/completions` prefix. Gateway API resolves otherwise identical matches
by route precedence, so an older route can bypass the ExtProc policy. Replace
that route with this example, or adapt its backend and attach the ExtProc policy
to the route you retain.

Install vSR:

```bash
helm upgrade -i semantic-router oci://ghcr.io/vllm-project/charts/semantic-router \
  --version 0.3.0 \
  --namespace agentgateway-system \
  -f examples/llm-semantic-routing/k8s/semantic-router-values.yaml

kubectl wait --for=condition=Available deployment/semantic-router \
  -n agentgateway-system \
  --timeout=600s
```

Apply the routed backend, route, and Streamed ExtProc policy:

```bash
kubectl apply -f examples/llm-semantic-routing/k8s/agentgateway-routing.yaml

kubectl wait --for=condition=Accepted agentgatewaybackend/openai-router-selected \
  -n agentgateway-system \
  --timeout=300s
kubectl describe httproute openai-semantic-routing -n agentgateway-system
kubectl describe agentgatewaypolicy semantic-router-extproc -n agentgateway-system
```

The values pin the vSR chart and `extproc` image to v0.3.0. Update both pins
together when validating a newer vSR release.

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
curl -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
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
reach OpenAI. Remove the probe signal and decision from the values before using
this policy in a production route.

## Run a Request

```bash
curl "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "Implement a small Go helper and one table-driven test."}
    ]
  }'
```

Agentgateway’s model catalog, metrics, logs, and traces remain the cost and
observability source of record. Run the demo to compare the routed policy with
forced lower-cost and always-expensive baselines.

## Cleanup

```bash
kubectl delete -f examples/llm-semantic-routing/k8s/agentgateway-routing.yaml
helm uninstall semantic-router -n agentgateway-system
```
