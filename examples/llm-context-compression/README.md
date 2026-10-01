## Context Compression Example

This example shrinks LLM request context with [Headroom](https://github.com/headroomlabs-ai/headroom)
before it reaches the provider, reducing token spend on long-context requests.

> [!NOTE]
> Agentgateway provides an interface to plug in your own context compressors. 
> Context compression often looks good on paper/benchmarks, but performs poorly in real world usage
> due to cache misses, degraded quality, etc.
> Measure with your own workflows before adopting.

A request guard webhook with `protocol: raw` receives the complete JSON request body, and can
replace it. `webhook.ts` forwards the request's messages to Headroom and swaps in the
compressed ones, leaving the rest of the request untouched. Requests under 16KiB are skipped.

The webhook responds with:

- `200`: the response body replaces the entire request body.
- `204`: the request is unchanged.
- `200` with the `x-agentgateway-direct-response` header: the request is not forwarded; the
  body, `{"status": <code>, "headers": {...}, "body": <string or JSON>}`, is returned to the client.

Errors follow `failureMode`; this example uses `failOpen`, so a compression failure sends the
original request. A replacement that is not a valid request always fails the request. The
webhook should not change the `model`.

### Running the example

Start Headroom and the webhook:

```bash
docker compose -f examples/llm-context-compression/docker-compose.yaml up
```

Or run them directly with `headroom proxy --port 8787 --mode cache` and
`bun examples/llm-context-compression/webhook.ts`.

Then run the gateway:

```bash
export OPENAI_API_KEY=sk-...
cargo run -- -f examples/llm-context-compression/config.yaml
```

Send a request with a large tool result; `gen-context.sh` generates a repetitive JSON log.
Headroom compresses tool output like this, but does not compress user messages.

```bash
jq -n --slurpfile logs <(examples/llm-context-compression/gen-context.sh) '{
    model: "gpt-6-luna",
    messages: [
      {role: "user", content: "Find the failing requests."},
      {role: "assistant", tool_calls: [{id: "c1", type: "function", function: {name: "get_logs", arguments: "{}"}}]},
      {role: "tool", tool_call_id: "c1", content: $logs[0]},
      {role: "user", content: "Which ids failed?"}
    ]
  }' | curl http://localhost:4000/v1/chat/completions \
  -H "Content-Type: application/json" -d @-
```

The webhook logs the token counts before and after compression; the response's
`usage.prompt_tokens` should be around 2k, a small fraction of the ~85KB log.

Compression is lossy: Headroom keeps a sample of the log rows, so the model may not see every
failing request.

### Prompt caching

With prompt caching (automatic on OpenAI, `cache_control` on Anthropic), cached input is far
cheaper than fresh input. If compression output changes as the conversation grows, it rewrites
the cached prefix on every turn, which usually costs more than it saves. Against cached
providers, run Headroom in a prefix-stable mode:

```bash
HEADROOM_MODE=cache \
HEADROOM_PROTECT_RECENT=0 \
HEADROOM_PROTECT_ANALYSIS_CONTEXT=0 \
HEADROOM_MIN_RATIO=0.75 \
HEADROOM_COMPRESS_MARKED_BLOCKS=1 \
headroom proxy --no-read-lifecycle
```

### Large contexts

Requests, and the webhook's replacement body, are limited by `maxBufferSize` (default 2MB).
Raise `frontendPolicies.http.maxBufferSize` for larger contexts.
