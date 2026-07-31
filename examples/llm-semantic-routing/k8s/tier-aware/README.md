# Tier-Aware Semantic Routing with vLLM Semantic Router

This example combines agentgateway and
[vLLM Semantic Router (vSR)](https://vllm-semantic-router.com/) to select a
different model for the same semantic signal based on an authenticated access
tier.

The example defines these model entitlements:

| Tier | Allowed models | Model selected for a STEM prompt |
| --- | --- | --- |
| Basic | GPT-4.1, GPT-5.4 | GPT-5.4 |
| Standard | GPT-4.1, GPT-5.4, Claude Haiku 4.5 | Claude Haiku 4.5 |
| Pro | GPT-4.1, GPT-5.4, Claude Haiku 4.5, Claude Sonnet 4.6 | Claude Sonnet 4.6 |

Each tier has its own `IntelligentPool` and `IntelligentRoute`. A Gateway-level
`PreRouting` policy selects the corresponding vSR ExtProc service from the
trusted `x-entitlement-tier` request header. vSR classifies the prompt, writes
the selected model into the request body's `model` field, and
`AgentgatewayModel` routing selects the correct OpenAI or Anthropic provider.

```text
authenticated tier
        |
        v
conditional PreRouting ExtProc
        |
        +-- basic ----> basic vSR configuration
        +-- standard -> standard vSR configuration
        `-- pro ------> pro vSR configuration
                              |
                              v
                   selected logical model
                  (request body + debug header)
                              |
                              v
                 OpenAI or Anthropic model
```

This indirection is necessary because an `AgentgatewayPolicy` references an
ExtProc service, not an `IntelligentPool` or `IntelligentRoute`. A vSR process
using Kubernetes configuration expects one pool and one route in its watched
namespace, so this example runs one vSR release per tier.

The `semantic-router-basic`, `semantic-router-standard`, and
`semantic-router-pro` namespaces isolate the watched configuration objects;
they do not isolate the vSR runtimes. All three vSR Deployments and Services
run in `agentgateway-system` and each process watches one of those configuration
namespaces. Namespace-per-runtime isolation is not required for this pattern.
Deploy each vSR runtime into its own namespace only when you also need separate
RBAC, resource quotas, network policies, or operational ownership. In that
case, update the ExtProc service references and any required cross-namespace
permissions accordingly.

The provider selection deliberately uses `AgentgatewayModel` instead of an
`HTTPRoute` header match. Gateway API route matching happens before a
`PreRouting` ExtProc can inspect and rewrite the request body, so a header
created by vSR cannot select an `HTTPRoute` backend. Model routing occurs after
the request-body rewrite and can therefore route the selected model across
providers.

## Before You Begin

This example requires:

- agentgateway v1.4.0 or later, including the matching CRDs, for conditional
  ExtProc and `AgentgatewayModel` support. Enable the experimental model API
  with the agentgateway Helm value `agentgatewayModels.enabled=true`.
- A running `Gateway` named `agentgateway-proxy` in the
  `agentgateway-system` namespace.
- OpenAI and Anthropic API credentials.
- Helm and `kubectl`.

Follow the agentgateway guides to
[install agentgateway](https://agentgateway.dev/docs/kubernetes/main/install/helm/)
and
[set up a Gateway](https://agentgateway.dev/docs/kubernetes/main/setup/gateway/).

For example, enable model routing on an existing Helm installation while
retaining its other values:

```bash
export AGENTGATEWAY_VERSION=v1.4.1

helm upgrade agentgateway \
  oci://ghcr.io/agentgateway/charts/agentgateway \
  --version "${AGENTGATEWAY_VERSION}" \
  --namespace agentgateway-system \
  --reuse-values \
  --set agentgatewayModels.enabled=true
```

Create provider credentials in the Gateway namespace:

```bash
kubectl create secret generic openai-secret \
  -n agentgateway-system \
  --from-literal=Authorization="${OPENAI_API_KEY}"

kubectl create secret generic anthropic-secret \
  -n agentgateway-system \
  --from-literal=Authorization="${ANTHROPIC_API_KEY}"
```

The manifests use current provider model IDs as examples. Change the model
transformations in `agentgateway-routing.yaml` if your provider account uses
different models or pinned model versions.

The Gateway listener must allow `AgentgatewayModel` resources. Add the
`AgentgatewayModel` entry to the `http` listener's `allowedRoutes.kinds` before
continuing, retaining any existing kinds that the listener also serves:

```yaml
spec:
  listeners:
  - name: http
    # protocol and port omitted
    allowedRoutes:
      namespaces:
        from: Same
      kinds:
      - group: agentgateway.dev
        kind: AgentgatewayModel
```

The `sectionName` in `agentgateway-routing.yaml` must match that listener
name. Once `allowedRoutes.kinds` is present, the listener accepts only the
listed kinds.

## Install the Tier Routers

Create one configuration namespace for each tier:

```bash
kubectl apply \
  -f examples/llm-semantic-routing/k8s/tier-aware/namespaces.yaml
```

Install one vSR release per tier. All three services run in
`agentgateway-system`, while each process watches only its tier-specific
configuration namespace:

```bash
export VSR_CHART_VERSION=0.0.0-latest
export VSR_IMAGE_TAG=latest

for tier in basic standard pro; do
  helm upgrade -i "semantic-router-${tier}" \
    oci://ghcr.io/vllm-project/charts/semantic-router \
    --version "${VSR_CHART_VERSION}" \
    --namespace agentgateway-system \
    -f examples/llm-semantic-routing/k8s/tier-aware/semantic-router-values.yaml \
    --set-string "image.tag=${VSR_IMAGE_TAG}" \
    --set "image.pullPolicy=Always" \
    --set-json \
      "args=[\"--secure=false\",\"--namespace=semantic-router-${tier}\"]"
done
```

Wait for the ExtProc deployments:

```bash
for tier in basic standard pro; do
  kubectl wait --for=condition=Available \
    "deployment/semantic-router-${tier}" \
    -n agentgateway-system \
    --timeout=600s
done
```

Apply the three pool and route pairs:

```bash
kubectl apply \
  -f examples/llm-semantic-routing/k8s/tier-aware/intelligent-routing.yaml

for tier in basic standard pro; do
  kubectl wait --for=condition=Ready \
    "intelligentpool/${tier}-models" \
    -n "semantic-router-${tier}" \
    --timeout=300s
  kubectl wait --for=condition=Ready \
    "intelligentroute/${tier}-routing" \
    -n "semantic-router-${tier}" \
    --timeout=300s
done
```

## Configure Agentgateway

Apply the provider models and conditional PreRouting ExtProc policy:

```bash
kubectl apply \
  -f examples/llm-semantic-routing/k8s/tier-aware/agentgateway-routing.yaml

kubectl get agentgatewaymodel -n agentgateway-system
kubectl describe agentgatewaypolicy tiered-semantic-routing \
  -n agentgateway-system
```

The policy rejects a request unless `x-entitlement-tier` is `basic`,
`standard`, or `pro`. Its conditional ExtProc entries are evaluated in order,
and the first match selects the tier-specific vSR service. vSR replaces
`model: auto` with the tier's selected logical model. The four
`AgentgatewayModel` resources match those logical names, authenticate with the
corresponding provider, and translate the two friendly Claude names to
Anthropic's provider model IDs. The Anthropic model policies repeat the tier
checks as defense in depth.

## Run Requests

In an environment where a load balancer assigns the Gateway an address, set
the endpoint from Gateway status:

```bash
export INGRESS_GW_ADDRESS="http://$(kubectl get gateway agentgateway-proxy \
  -n agentgateway-system \
  -o jsonpath='{.status.addresses[0].value}')"
```

A local kind cluster does not assign a load-balancer address by default, so
`.status.addresses` is empty. Port-forward the generated Service instead:

```bash
kubectl port-forward \
  -n agentgateway-system \
  service/agentgateway-proxy 8080:80
```

In another terminal, set the local endpoint:

```bash
export INGRESS_GW_ADDRESS=http://127.0.0.1:8080
```

Use `model: auto` so vSR performs semantic model selection. The debug header
exposes the decision in the response headers.

A STEM prompt from the Basic tier selects GPT-5.4:

```bash
curl -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -H "X-Entitlement-Tier: basic" \
  -H "X-VSR-Debug: true" \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "Define quantum physics."}
    ],
    "max_tokens": 64
  }'
```

Expected response header:

```text
x-vsr-selected-model: gpt-5.4
```

The same prompt from Standard selects Claude Haiku:

```bash
curl -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -H "X-Entitlement-Tier: standard" \
  -H "X-VSR-Debug: true" \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "Define quantum physics."}
    ],
    "max_tokens": 64
  }'
```

Expected response header:

```text
x-vsr-selected-model: claude-haiku-4.5
```

The Pro tier selects Claude Sonnet:

```bash
curl -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -H "X-Entitlement-Tier: pro" \
  -H "X-VSR-Debug: true" \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "Define quantum physics."}
    ],
    "max_tokens": 64
  }'
```

Expected response header:

```text
x-vsr-selected-model: claude-sonnet-4.6
```

A prompt without a configured STEM keyword falls back to GPT-4.1 in every
tier. A caller also cannot force a model outside its pool; for example, a Basic
request with `model: claude-sonnet-4.6` returns `400 model is not available`.

## Secure the Tier Context

The request header is directly supplied by `curl` only to keep this example
focused on routing. Do not trust a client-provided entitlement header in
production.

Use [agentgateway policies](https://agentgateway.dev/docs/kubernetes/latest/about/policies/)
for JWT, API key, or external authorization to authenticate the caller. Derive
or overwrite `x-entitlement-tier` from trusted identity context before the
conditional ExtProc policy runs. Retain model authorization as defense in
depth.

## Troubleshooting

Verify that the installed CRD supports conditional ExtProc:

```bash
kubectl explain \
  agentgatewaypolicy.spec.traffic.extProc.conditional
kubectl explain agentgatewaymodel.spec.match.model
```

Check which configuration each vSR deployment watches:

```bash
for tier in basic standard pro; do
  kubectl logs "deployment/semantic-router-${tier}" \
    -n agentgateway-system \
    --since=5m |
    grep -E \
      'kubernetes_controller_starting|kubernetes_config_applied|routing_decision'
done
```

Inspect the CRD status if a router does not become ready:

```bash
kubectl get intelligentpool,intelligentroute -A
```

## Cleanup

```bash
kubectl delete \
  -f examples/llm-semantic-routing/k8s/tier-aware/agentgateway-routing.yaml
kubectl delete \
  -f examples/llm-semantic-routing/k8s/tier-aware/intelligent-routing.yaml

for tier in basic standard pro; do
  helm uninstall "semantic-router-${tier}" -n agentgateway-system
done

kubectl delete \
  -f examples/llm-semantic-routing/k8s/tier-aware/namespaces.yaml
```
