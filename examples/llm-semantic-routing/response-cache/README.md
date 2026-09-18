# Response Cache with Redis Open Source

These examples combine agentgateway v1.5.0, [vLLM Semantic Router
(vSR)](https://vllm-sr.ai/), and Redis Open Source to reuse responses to
semantically equivalent product-support questions. Both run a deterministic
HomeHub Python backend that returns fixed answers without calling an LLM
provider, so no LLM credentials are needed.

## Choose a deployment

- [Kubernetes](kubernetes/README.md): local kind cluster with the agentgateway
  controller, vSR, Redis, and the HomeHub Python backend.
- [Standalone](standalone/README.md): Docker Compose with standalone
  agentgateway, vSR, Redis, and the HomeHub Python backend.

Each deployment has its own `versions.env`. vSR intentionally uses `latest`
(and chart `0.0.0-latest` on Kubernetes) because the v0.3.0 release lacks the
required ExtProc streaming changes.

## How it works

```text
Client --> agentgateway --> HomeHub backend (cache miss)
                 |                 |
                 v                 |
             vSR ExtProc <---------+ response stored on a miss
                 |
                 v
               Redis
```

On a hit, vSR returns the cached completion through ExtProc and agentgateway
responds without calling HomeHub. On a miss, agentgateway calls HomeHub and
passes the complete response through vSR to store it in Redis.

> [!NOTE]
> HomeHub returns complete JSON responses and does not support streaming
> completions. Requests use `stream: false`. To keep this example simple,
> agentgateway also buffers ExtProc request and response bodies, and vSR's
> streamed request-body handling is disabled. ExtProc body processing and LLM
> response streaming are separate settings.

The example enables vSR's [response-cache plugin](https://vllm-sr.ai/docs/tutorials/plugin/response-cache/)
for factory-reset questions. With `mode: semantic`, vSR uses semantic matching
for both identical questions and paraphrases, with a similarity threshold of
0.70. `scope: global` allows callers to share these fixed, non-personalized
answers. Redis stores the cached responses so they remain available across
vSR instances and restarts.

> [!NOTE]
> vSR also supports an in-memory cache. Its entries belong to one vSR process
> and are lost when that process restarts.

Both verification scripts check an initial miss, an identical request hit,
a paraphrase hit, and an uncached question. Backend invocation counts confirm
that cache hits bypass HomeHub. Optional checks verify reuse from a fresh vSR
instance and persistence across a Redis restart.

The [shared backend](shared/support-backend.py) uses only Python's standard
library. Kubernetes loads it from a ConfigMap and Docker Compose mounts it into
the Python image.

## Using your own data

These examples use fixed, public answers and a single Redis instance without
authentication or TLS. Before adapting them for real traffic, see
[response caching in production](https://agentgateway.dev/docs/kubernetes/main/integrations/llm/routing/vllm-semantic-router/#response-caching-in-production)
for guidance on cache correctness, access protection, and Redis operations.
