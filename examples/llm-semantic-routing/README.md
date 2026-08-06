# LLM Semantic Routing

These Kubernetes examples combine agentgateway with
[vLLM Semantic Router (vSR)](https://vllm-semantic-router.com/) to select an
LLM from request content and context.

## Supported Examples

- [Cost-based routing](k8s/cost-based/README.md) classifies coding requests and
  selects either a lower-cost OpenAI model or a higher-capability OpenAI model.
  It also includes an optional policy that forces automatic model selection.
- [Tier-aware routing](k8s/tier-aware/README.md) selects a separate vSR
  configuration for Basic, Standard, or Pro callers, then routes the selected
  logical model to OpenAI or Anthropic with `AgentgatewayModel`.

Each subdirectory contains its own setup instructions, Helm values, and
Kubernetes manifests.
