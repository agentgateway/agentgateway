# Response Cache with Redis Open Source

These examples combine agentgateway v1.6.0, [vLLM Semantic Router
(vSR)](https://vllm-sr.ai/), and Redis Open Source to reuse responses to
semantically equivalent product-support questions. Both run a deterministic
HomeHub Python backend that returns fixed answers without calling an LLM
provider, so no LLM credentials are needed.

## Choose a deployment

- [Kubernetes](kubernetes/README.md): local kind cluster with the agentgateway
  controller, vSR, Redis, and the HomeHub Python backend.
- [Standalone](standalone/README.md): Docker Compose with standalone
  agentgateway, vSR, Redis, and the HomeHub Python backend.

Each deployment pins agentgateway v1.6.0 and vSR v0.4.0. Kubernetes uses
vSR chart 0.4.0. Both deployments pin the public HomeHub image to commit `fd19c81`.

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

The [HomeHub demo](https://github.com/solo-io/homehub-support-demo) is maintained
separately and published as `ghcr.io/solo-io/homehub-support-demo`. Neither
deployment requires a local backend source file or image build.

## Verify the cache

After following either deployment guide, use its gateway URL and backend-count
command for these four requests. The table assumes a fresh cache and backend:

| Request | Prompt | Expected cache result | Backend count |
| --- | --- | --- | ---: |
| Initial request | How do I factory-reset my HomeHub X2? | Miss | 1 |
| Exact repeat | How do I factory-reset my HomeHub X2? | Hit | 1 |
| Paraphrase | What is the procedure for restoring a HomeHub X2 to factory settings? | Hit | 1 |
| Uncached question | Can a HomeHub be used outdoors in freezing rain? | Miss | 2 |

Send the first request, then repeat the command with `PROMPT` set to each of
the remaining prompts:

```bash
PROMPT='How do I factory-reset my HomeHub X2?'
jq -n --arg prompt "${PROMPT}" '{
  model: "auto",
  stream: false,
  messages: [{role: "user", content: $prompt}],
  max_tokens: 96
}' | curl -sS -i "${GATEWAY_URL}/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -H 'X-VSR-Debug: true' \
  -H "X-Request-ID: $(uuidgen)" \
  --data-binary @-
```

On a hit, the response includes `x-vsr-cache-hit: true`. With the debug header,
vSR also exposes the cache similarity when available. Check the backend count
after each request: it increases on a miss and stays unchanged on a hit.

The request ID lets vSR correlate the request-side cache entry with the
completed response that it stores. Use a unique `X-Request-ID` for each request.

If reusing an existing cache, the first request may already hit. Compare counts
before and after requests instead of expecting the absolute values above.
Optionally restart vSR or Redis, wait for readiness, and repeat the cached
request to check reuse across restarts.

## Using your own data

These examples use fixed, public answers and a single Redis instance without
authentication or TLS. Before adapting them for real traffic, see
[response caching in production](https://agentgateway.dev/docs/kubernetes/main/integrations/llm/routing/vllm-semantic-router/#response-caching-in-production)
for guidance on cache correctness, access protection, and Redis operations.
