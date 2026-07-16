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

## Run a Request

Set your gateway address:

```bash
export INGRESS_GW_ADDRESS="http://$(kubectl get gateway agentgateway-proxy \
  -n agentgateway-system \
  -o jsonpath='{.status.addresses[0].value}')"
```

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

### Force a Model Tier

The default configuration allows a client to request either configured model
directly. This bypasses vSR's automatic model selection, while retaining the
same agentgateway forwarding, cost tracking, and observability path:

```bash
curl -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5.5",
    "messages": [
      {"role": "user", "content": "Use the advanced model for this request."}
    ],
    "max_tokens": 64
  }'
```

The response body `model` field identifies the serving model. vSR response
headers record the explicit selection when available.

### Force Automatic Routing

To require automatic routing regardless of a client-provided model, apply the
optional override policy:

```bash
kubectl apply -f examples/llm-semantic-routing/k8s/force-auto.yaml
```

The policy uses an [agentgateway request-body
transformation](https://agentgateway.dev/docs/kubernetes/latest/traffic-management/transformations/validate/)
to rewrite the request's `model` field to `auto` before vSR selects a model.
Remove it to restore direct model selection:

```bash
kubectl delete -f examples/llm-semantic-routing/k8s/force-auto.yaml
```

Keep this optional policy disabled when running an evaluation that includes a
forced-model baseline, such as the cost-based semantic-routing demo's
`always_expensive` lane.

Agentgateway’s model catalog, metrics, logs, and traces remain the cost and
observability source of record. Use isolated evaluation traffic with forced
lower-cost and always-expensive baselines before adopting the policy broadly.

## Optional: Use Codex Through the Gateway

The gateway works with any OpenAI API-compatible client or agent. This optional
section configures Codex to use the gateway's `auto` model name. Codex uses the
OpenAI Responses API and vSR translates streamed Responses events before the
gateway forwards the request to the selected model.

### Codex CLI

This configuration was tested with `codex-cli 0.144.4`. The
[Codex CLI profile documentation](https://learn.chatgpt.com/docs/config-file/config-advanced#profiles)
describes how `--profile` overlays a named user-level configuration file.
Create the profile:

```bash
export CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
export AGENTGATEWAY_BASE_URL="http://$(kubectl get gateway agentgateway-proxy \
  -n agentgateway-system \
  -o jsonpath='{.status.addresses[0].value}')/v1"
# For a TLS-enabled corporate gateway, set AGENTGATEWAY_BASE_URL to its https URL.
mkdir -p "$CODEX_HOME"
cat > "$CODEX_HOME/agentgateway.config.toml" <<EOF
model = "auto"
model_provider = "agentgateway"

[model_providers.agentgateway]
name = "Corporate agentgateway"
base_url = "${AGENTGATEWAY_BASE_URL}"
wire_api = "responses"
EOF
```

Start Codex with that profile:

```bash
codex --profile agentgateway
```

### Codex in the ChatGPT Desktop App

Codex is available in the ChatGPT desktop app as a Codex environment. This
configuration was tested with ChatGPT desktop app version `26.707.72221`.
See the [Codex environment documentation](https://learn.chatgpt.com/docs/environments/modes)
and [Codex configuration basics](https://learn.chatgpt.com/docs/config-file/config-basic).

For the same gateway configuration, back up and replace the user-level config,
then restart the ChatGPT desktop app:

```bash
export AGENTGATEWAY_BASE_URL="http://$(kubectl get gateway agentgateway-proxy \
  -n agentgateway-system \
  -o jsonpath='{.status.addresses[0].value}')/v1"
# For a TLS-enabled corporate gateway, set AGENTGATEWAY_BASE_URL to its https URL.
cp ~/.codex/config.toml ~/.codex/config.toml.bak
cat > ~/.codex/config.toml <<EOF
model = "auto"
model_provider = "agentgateway"

[model_providers.agentgateway]
name = "Corporate agentgateway"
base_url = "${AGENTGATEWAY_BASE_URL}"
wire_api = "responses"
EOF
```

Replacing `~/.codex/config.toml` also replaces other user-level Codex settings.
To edit that file through the app instead, open **Settings > Configuration >
Open config.toml** and apply the same configuration.

### Verify Codex Routing

After sending a task from Codex CLI or the ChatGPT desktop app, inspect the vSR
decision and the completed agentgateway request:

```bash
kubectl logs -n agentgateway-system deploy/semantic-router --since=5m \
  | grep -E '"event":"routing_decision"|"event":"router_replay_complete"' \
  | tail -n 4

kubectl logs -n agentgateway-system deploy/agentgateway-proxy --since=5m \
  | grep 'http.path=/v1/responses' \
  | tail -n 4
```

The vSR output identifies `original_model: auto`, the `selected_model`, and a
successful `response_status`. The agentgateway output identifies the
`openai-semantic-routing` route, the selected request and response model,
catalog-priced token usage, and the realized request cost.

Codex also probes `/v1/models` to discover model metadata. Until [agentgateway
issue #1462](https://github.com/agentgateway/agentgateway/issues/1462) adds a
gateway-generated model list, Codex may warn that metadata for `auto` is not
found. That warning does not prevent `/v1/responses` traffic from routing.

The gateway authenticates to OpenAI with its configured provider credential and
records the selected model and cost as it does for other OpenAI-compatible
clients. Agentgateway can [rewrite client-facing model names with model
aliases](https://agentgateway.dev/docs/kubernetes/latest/llm/alias/). An
organization can also use a [request-body
transformation](https://agentgateway.dev/docs/kubernetes/latest/traffic-management/transformations/validate/)
to rewrite every request to `auto`. Treat `auto` as the supported client path
when testing this policy.

## Cleanup

```bash
kubectl delete -f examples/llm-semantic-routing/k8s/agentgateway-routing.yaml
helm uninstall semantic-router -n agentgateway-system
```
