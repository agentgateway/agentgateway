## Context Compression Example

This example shows how to shrink LLM request context through an external compression
engine before it reaches the provider, reducing token spend on long-context requests.

The gateway is engine-agnostic: it defines a small wire contract (below), and any
service implementing it can be plugged in. [Headroom](https://github.com/headroomlabs-ai/headroom)
is used as the reference engine here.

### Wire contract (version 1)

For each compressible request, the gateway sends:

```
POST /v1/compress
Content-Type: application/json
x-agw-compression-version: 1

{ "messages": [ ...provider-native message objects... ], "model": "claude-sonnet-4-5" }
```

- `messages` is the request's native message array, forwarded verbatim — provider-specific
  blocks (`cache_control`, images, tool calls) survive the round-trip. The system prompt is
  *not* included: it is the stable prefix that prompt-cache reuse depends on.
- `model` is a tokenizer/context-window hint, not a routing target.

The engine responds `200` with the compressed array:

```
{ "messages": [ ...compressed message objects... ] }
```

Any non-200 status, malformed body, or output that breaks the request's tool-call pairing
is treated as an engine failure and resolved per `failureMode` (default `failOpen`: the
original request is forwarded unchanged).

The gateway compresses after prompt guards run (guards see the original content) and
before token counting (rate limits and cost reflect what is actually sent).

### Running the example

Start the engine. With Headroom, use `--mode cache` so it freezes prior turns and keeps
provider prefix-cache reuse intact:

```bash
headroom proxy --port 8787 --mode cache
```
Or, you can use docker compose `docker compose -f examples/llm-context-compression/docker-compose.yaml up`

Then run the gateway:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cargo run -- -f examples/llm-context-compression/config.yaml
```

### Sending a request

Compression only helps when there is enough context to compress; requests below
`minSizeBytes` (default 16KiB) skip the engine entirely. Embed a large context block:

```bash
curl http://localhost:4000/v1/messages \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d "$(jq -n --rawfile ctx some-large-file.txt '{
    model: "claude-sonnet-4-5",
    max_tokens: 200,
    messages: [{
      role: "user",
      content: ("Here is some reference material:\n\n" + $ctx + "\n\nWhat are the key takeaways?")
    }]
  }')"
```

Send `x-agw-compression-bypass: true` on any request to skip compression for that call.
The gateway consumes this header; it is not forwarded to the provider.

### Prompt caching: compression can cost more than it saves

On providers with prompt caching (Anthropic), cache reads are ~10x cheaper than fresh
input. A compressor whose output for a given message changes as the conversation grows
(position-dependent compression) rewrites the cached prefix on every turn — busting the
cache usually costs more than compression saves.

Only run engines in a deterministic, prefix-stable mode against cached providers. For
Headroom that is cache-stable configuration:

```bash
HEADROOM_MODE=cache \
HEADROOM_PROTECT_RECENT=0 \
HEADROOM_PROTECT_ANALYSIS_CONTEXT=0 \
HEADROOM_MIN_RATIO=0.75 \
HEADROOM_COMPRESS_MARKED_BLOCKS=1 \
headroom proxy --no-read-lifecycle
```

Keep `HEADROOM_NET_COST_POLICY` and `HEADROOM_SAVINGS_PROFILE` unset — both reintroduce
position-dependent behavior. Watch the provider's reported cache-read tokens across a long
session: they should stay high; a collapse means the engine is rewriting the cached prefix.

### Large contexts

Request and engine-response bodies are subject to the frontend's `maxBufferSize` (default
2MB). For contexts larger than that, raise `frontendPolicies.http.maxBufferSize` on the
bind; the gateway applies the same limit when reading the engine's compressed response.
