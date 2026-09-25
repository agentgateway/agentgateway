# Semantic Routing Examples

These examples demonstrate different ways to integrate [vLLM Semantic Router (vSR)](https://vllm-sr.ai/)
with agentgateway.

All examples use the same core architecture:

```text
Client
   |
agentgateway
   |
vSR ExtProc
   |
LLM provider(s)
```

Each example focuses on a different production use case.

| agentgateway mode | Example | Demonstrates | Best for |
| --- | --- | --- | --- |
| Kubernetes | [Cost-based routing](k8s/cost-based/) | Route requests to lower-cost or higher-capability models based on semantic classification. | Cost optimization while maintaining response quality. |
| Kubernetes | [Tier-aware routing with CRDs](k8s/tier-aware/) | Select a tier-specific vSR runtime configured by `IntelligentPool` and `IntelligentRoute`. | Kubernetes-native pool/route management and separate runtimes per tier. |
| Kubernetes | [Tier-aware routing with one runtime](k8s/tier-aware-single-runtime/) | Combine tier and semantic signals in a single vSR runtime using the [v0.3 Unified Config Contract](https://vllm-sr.ai/docs/proposals/unified-config-contract-v0-3). | This example **does not** use the vSR `IntelligentPool` and `IntelligentRoute` CRDs. |
| Kubernetes | [Response caching](response-cache/kubernetes/) | Cache semantically equivalent requests in Redis Open Source and optionally share entries across vSR replicas. | Product support, documentation assistants, FAQ chatbots, and other workloads with many repeated questions. |
| Standalone | [Tier-aware routing](standalone/tier-aware-single-runtime/) | Use one YAML-configured vSR runtime with standalone agentgateway in Docker Compose. | Local development and deployments without Kubernetes. |
| Standalone | [Response caching](response-cache/standalone/) | Run agentgateway, vSR, Redis, and the HomeHub Python backend with Docker Compose. | Local response-cache evaluation without Kubernetes or provider credentials. |


## Choosing an example

### Cost-based routing

Use this example when you want vSR to decide **which model** should answer a
request.

Typical goals include:

- reducing LLM cost
- balancing quality and latency
- automatically selecting inexpensive models for routine requests

See: `k8s/cost-based`

---

### Tier-aware routing

Use this example when different users are allowed to access different model
capabilities.

Typical goals include:

- Basic / Pro subscriptions
- internal vs external users
- premium AI features
- provider-specific model pools

Choose a deployment pattern:

- [Kubernetes with CRDs](k8s/tier-aware/): one vSR runtime per tier, with Kubernetes `IntelligentPool`
  and `IntelligentRoute` custom resources.
- [Kubernetes with one runtime](k8s/tier-aware-single-runtime/): one shared vSR runtime,
  with tier-aware decisions in a canonical YAML ConfigMap.
- [Standalone with Docker Compose](standalone/tier-aware-single-runtime/): one shared
  vSR runtime configured with canonical YAML.

All three examples demonstrate Basic, Standard, and Pro user entitlements and use
agentgateway for provider-based routing.

---

### Response caching

Use this example when users ask the same question in different ways. vSR
reuses responses from Redis, reducing calls to the backend. A deterministic
HomeHub Python backend makes cache hits easy to verify without LLM credentials.

The example checks identical and paraphrased requests, cache sharing across
vSR instances, and persistence across Redis restarts.

Choose [Kubernetes](response-cache/kubernetes/) or
[standalone Docker Compose](response-cache/standalone/). See the
[response-cache overview](response-cache/README.md) for the request flow.
