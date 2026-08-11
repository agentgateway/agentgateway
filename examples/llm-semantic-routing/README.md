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

| Example | Demonstrates | Best for |
| --- | --- | --- |
| [Cost-based routing](k8s/cost-based/) | Route requests to lower-cost or higher-capability models based on semantic classification. | Cost optimization while maintaining response quality. |
| [Tier-aware routing](k8s/tier-aware/) | Select different model pools according to authenticated user entitlements. | SaaS plans, internal vs external users, premium AI features. |
| [Semantic caching](k8s/semantic-cache/) | Cache semantically equivalent requests in Redis Open Source and optionally share entries across vSR replicas. | Product support, documentation assistants, FAQ chatbots, and other workloads with many repeated questions. |

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

See: `k8s/tier-aware`

---

### Semantic caching

Use this example when many users ask **the same question in different ways**.

Instead of generating a new response for every request, vSR recognizes
semantically equivalent prompts and returns a previously generated response from
a Redis Open Source cache.

The example demonstrates:

- local kind deployment
- Redis Open Source 8 with vector search
- Redis-backed semantic cache
- semantic cache hits for paraphrased requests
- optional cache sharing across vSR replicas
- cache persistence across Redis pod restarts

vSR supports multiple cache backends, including a default in-memory store.
Redis is used here as a production-oriented backend because it allows vSR
replicas to share cache entries and persist them across process restarts. Redis
also backs other agentgateway-related services, such as [global rate
limiting](https://agentgateway.dev/docs/kubernetes/main/security/rate-limit-global/).
The example enables Redis persistence on a local persistent volume.

See: `k8s/semantic-cache`
