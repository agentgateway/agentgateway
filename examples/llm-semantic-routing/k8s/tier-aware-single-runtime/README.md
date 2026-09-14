# Tier-Aware Routing with One vLLM Semantic Router Runtime

This example combines agentgateway and [vLLM Semantic Router (vSR)](https://vllm-sr.ai/)
to select a different model for the same [semantic signal](https://vllm-sr.ai/docs/tutorials/signal/overview)
based on the request's access tier.

It's an alternative to the [CRD-based tier-aware example](../tier-aware/) by using
one vSR Deployment that reads a [canonical YAML configuration](https://vllm-sr.ai/docs/installation/configuration/)
from a ConfigMap instead of running one router per access tier defined by
`IntelligentPool` and `IntelligentRoute` custom resources.

The example defines these model entitlements:

| Tier | Allowed models | Model selected for a STEM prompt |
| --- | --- | --- |
| Basic | GPT-4.1, GPT-5.4 | GPT-5.4 |
| Standard | GPT-4.1, GPT-5.4, Claude Haiku 4.5 | Claude Haiku 4.5 |
| Pro | GPT-4.1, GPT-5.4, Claude Haiku 4.5, Claude Sonnet 4.6 | Claude Sonnet 4.6 |

Agentgateway and vSR work together to select a model based on the caller's tier
and the prompt's content:

- A Gateway-level `PreRouting` policy sends the request to one vSR ExtProc service.
- The [authz signal](https://vllm-sr.ai/docs/tutorials/signal/heuristic/authz)
  reads `x-authz-user-id` and `x-entitlement-tier` to identify the caller's tier.
- vSR combines the tier with STEM
  [keyword signals](https://vllm-sr.ai/docs/tutorials/signal/heuristic/keyword)
  to select a model, falling back to GPT-4.1 when no decision matches.
- vSR writes the selected model into the request body's `model` field.
- `AgentgatewayModel` routing sends the request to the OpenAI or Anthropic provider.

The following diagram summarizes the request flow from tier validation to the selected model provider:

```text
request identity + tier
  -> agentgateway PreRouting authorization
  -> one vSR ExtProc service
       authz(tier) AND keyword(stem) -> tier-specific model
       no matching decision          -> gpt-4.1
  -> AgentgatewayModel authorization + provider translation
  -> OpenAI or Anthropic
```

Keyword matching is used in the example to keep STEM detection predictable and
requires no classifier model. Other vSR [signals](https://vllm-sr.ai/docs/tutorials/signal/overview)
can replace or supplement the example's keyword condition.

After vSR writes the selected model into the request body, agentgateway uses
[AgentgatewayModel](https://agentgateway.dev/docs/kubernetes/main/reference/api/#agentgatewaymodel)
resources to route the request to the appropriate provider.
These resources also enforce tier access through authorization policies,
rejecting requests for models outside the caller's tier even though they appear
in the shared provider catalog.

**Note:** This example uses `AgentgatewayModel` because model routing runs after
vSR selects a model and rewrites the request body. `HTTPRoute` matching occurs
earlier, so it cannot select a provider using the model or headers produced by
vSR during PreRouting ExtProc processing.

## Before You Begin

This example requires:

- agentgateway v1.4.1 and its matching CRDs. Enable the experimental model API
  with the Helm value `agentgatewayModels.enabled=true`.
- A running `Gateway` named `agentgateway-proxy` in the
  `agentgateway-system` namespace.
- OpenAI and Anthropic API credentials with access to the configured models.
- Helm, `kubectl`, and curl.

Follow the agentgateway guides to
[install agentgateway](https://agentgateway.dev/docs/kubernetes/main/documentation/install/helm/)
and
[set up a Gateway](https://agentgateway.dev/docs/kubernetes/main/documentation/setup/gateway/).
Run commands from the agentgateway repository root. Run the two tier-aware
examples separately because they reuse Gateway policy and provider resource
names.

**Note:** Requests make billable calls to OpenAI/Anthropic providers.

For example, enable model routing on an existing agentgateway Helm installation while
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

Set `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` in your shell, then create provider
credentials in the Gateway namespace:

```bash
kubectl create secret generic openai-secret \
  -n agentgateway-system \
  --from-literal=Authorization="${OPENAI_API_KEY:?Set OPENAI_API_KEY}" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create secret generic anthropic-secret \
  -n agentgateway-system \
  --from-literal=Authorization="${ANTHROPIC_API_KEY:?Set ANTHROPIC_API_KEY}" \
  --dry-run=client -o yaml | kubectl apply -f -
```

The Gateway listener must allow `AgentgatewayModel` resources. Add the
`AgentgatewayModel` entry to the `http` listener's `allowedRoutes.kinds`,
retaining any existing kinds that the listener also serves:

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

The `sectionName` in `agentgateway-routing.yaml` must match that listener name. Once
`allowedRoutes.kinds` is present, the listener accepts only the listed kinds.

## Install the vLLM Semantic Router (vSR)

Create the ConfigMap and install the vSR Deployment and Service:

```bash
export EXAMPLE=examples/llm-semantic-routing/k8s/tier-aware-single-runtime

kubectl -n agentgateway-system create configmap tier-aware-config \
  --from-file="$EXAMPLE/config.yaml" --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f "$EXAMPLE/semantic-router.yaml"

kubectl -n agentgateway-system rollout status deployment/semantic-router \
  --timeout=600s
```

The Deployment uses `ghcr.io/vllm-project/semantic-router/vllm-sr:latest` with
`imagePullPolicy: Always` and passes `/app/config/config.yaml` to the image's
startup script. No vSR Helm release, operator, or CRDs are required.

## Configure Agentgateway

Apply the provider models and PreRouting ExtProc policy:

```bash
kubectl apply -f "$EXAMPLE/agentgateway-routing.yaml"

kubectl get agentgatewaymodel -n agentgateway-system
kubectl describe agentgatewaypolicy tiered-semantic-routing \
  -n agentgateway-system
```

Confirm the policy is accepted and attached.

## Run Requests

In an environment where a load balancer assigns the Gateway an address, set
the endpoint from Gateway status:

```bash
export INGRESS_GW_ADDRESS="http://$(kubectl get gateway agentgateway-proxy \
  -n agentgateway-system \
  -o jsonpath='{.status.addresses[0].value}')"
```

If no load-balancer address is available, port-forward the generated Service:

```bash
kubectl port-forward -n agentgateway-system service/agentgateway-proxy 8080:80
```

In another terminal, set the local endpoint:

```bash
export INGRESS_GW_ADDRESS=http://127.0.0.1:8080
```

Use `model: auto` to trigger semantic routing. For each request, verify the
selected-model header to confirm routing and check for HTTP 200 with generated
text to confirm the provider successfully processed the request.

### Basic STEM Request

```bash
curl --fail-with-body -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -H 'X-Authz-User-Id: demo-user' \
  -H 'X-Entitlement-Tier: basic' \
  -H 'X-VSR-Debug: true' \
  -d '{"model":"auto","messages":[{"role":"user","content":"Define quantum physics in one sentence."}],"max_tokens":64}'
```

Expected: `x-vsr-selected-model: gpt-5.4` and
`x-vsr-selected-decision: basic_stem`.

### Standard STEM Request

```bash
curl --fail-with-body -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -H 'X-Authz-User-Id: demo-user' \
  -H 'X-Entitlement-Tier: standard' \
  -H 'X-VSR-Debug: true' \
  -d '{"model":"auto","messages":[{"role":"user","content":"Define quantum physics in one sentence."}],"max_tokens":64}'
```

Expected: `x-vsr-selected-model: claude-haiku-4-5-20251001` and
`x-vsr-selected-decision: standard_stem`.

### Pro STEM Request

```bash
curl --fail-with-body -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -H 'X-Authz-User-Id: demo-user' \
  -H 'X-Entitlement-Tier: pro' \
  -H 'X-VSR-Debug: true' \
  -d '{"model":"auto","messages":[{"role":"user","content":"Define quantum physics in one sentence."}],"max_tokens":64}'
```

Expected: `x-vsr-selected-model: claude-sonnet-4-6` and
`x-vsr-selected-decision: pro_stem`.

### Fallback in Every Tier

```bash
for tier in basic standard pro; do
  printf '\nTier: %s\n' "$tier"
  curl --fail-with-body -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
    -H 'Content-Type: application/json' \
    -H 'X-Authz-User-Id: demo-user' \
    -H "X-Entitlement-Tier: $tier" \
    -H 'X-VSR-Debug: true' \
    -d '{"model":"auto","messages":[{"role":"user","content":"Say hello."}],"max_tokens":64}'
done
```

Expected: HTTP 200 and `x-vsr-selected-model: gpt-4.1` in all three cases.

### Reject Models Outside the Tier

A `basic` caller explicitly requesting Sonnet must receive HTTP 403 (Forbidden):

```bash
curl -sS -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -H 'X-Authz-User-Id: demo-user' \
  -H 'X-Entitlement-Tier: basic' \
  -d '{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"Say hello."}],"max_tokens":64}'
```

Additional checks (keep other headers valid):

| Request | Expected response |
| --- | --- |
| Standard requests `claude-sonnet-4-6` | 403 |
| Basic requests `claude-haiku-4-5-20251001` | 403 |
| Standard requests `claude-haiku-4-5-20251001` | 200 with generated text |
| `auto` with a missing tier, unknown tier, or missing user ID | 403 |
| `auto` with `X-VSR-Skip-Processing: true` | 403 |
| A valid tier requests an unconfigured model | 400 |

## Secure the Tier Context

The headers in these curl requests demonstrate routing and not caller
authentication. To add authentication and secure the tier context, follow the
agentgateway guides:

- [JWT authentication](https://agentgateway.dev/docs/kubernetes/latest/documentation/security/jwt/setup/)
  validates tokens from an identity provider. Combine it with
  [authorization rules](https://agentgateway.dev/docs/kubernetes/latest/documentation/security/authorization/)
  to require that the user ID and tier headers match validated JWT claims.
  Extend the existing Gateway-level PreRouting policy with authentication
  and claim checks so that vSR receives only requests with verified tier context.
- [API key authentication](https://agentgateway.dev/docs/kubernetes/latest/documentation/security/apikey/)
  validates the caller's credentials. Pair it with authorization that verifies
  the requested tier is assigned to that caller.
  Otherwise, a Basic customer could supply `X-Entitlement-Tier: pro`.
- [External authorization](https://agentgateway.dev/docs/kubernetes/latest/documentation/security/extauth/byo-ext-auth-service/)
  delegates access decisions to your own service, where you can look up the
  caller's entitlement and reject a mismatched tier.

Validate the user ID and access tier headers against trusted identity information
before vSR processes the request. For example, use JWT authentication with
authorization rules that require these headers to match the token’s claims.

Keep `x-vsr-skip-processing` reserved for agentgateway's internal processing.
The example rejects requests when clients supply [this header](https://vllm-sr.ai/docs/troubleshooting/vsr-headers/).

All tiers share one vSR runtime. Use the [CRD-based example](../tier-aware/)
if you need to manage each tier's runtime separately.

## Troubleshooting

```bash
kubectl -n agentgateway-system describe agentgatewaypolicy tiered-semantic-routing
kubectl -n agentgateway-system logs deployment/semantic-router --since=5m
kubectl -n agentgateway-system logs deployment/agentgateway-proxy --since=5m
```

Inspect the response body for provider credential, quota, or model-access errors.
A provider error is distinct from an incorrect routing decision. Confirm the
model IDs are available to your accounts, and inspect policy status
if requests do not reach a provider.

**Note:** The example AgentgatewayPolicy fails closed when vSR is unavailable.

## Cleanup

Stop any port-forward with Ctrl-C, then remove the example resources:

```bash
kubectl delete -f "$EXAMPLE/agentgateway-routing.yaml"
kubectl delete -f "$EXAMPLE/semantic-router.yaml"
kubectl delete configmap tier-aware-config -n agentgateway-system
```

The existing Gateway, agentgateway installation, and provider Secrets are retained.
Remove these additional resources if needed.
