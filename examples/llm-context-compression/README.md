## Context Compression Example

This example shows how to shrink LLM request context through an external compression
engine before it reaches the provider, reducing token spend on long-context requests.

Compression is not a dedicated policy — it reuses the same guardrail webhook used for
prompt/response guards, pointed at a compression endpoint with `messageFormat: raw`. Any
service implementing [Headroom's](https://github.com/headroomlabs-ai/headroom)
`POST /v1/compress` API can be plugged in.

### Wire contract

In `guardrail` mode (the default), the webhook exchanges a simplified role/content message
envelope. `raw` mode drops the envelope and sends/receives provider-native messages
verbatim, so provider-specific blocks (`cache_control`, images, tool calls) survive the
round-trip:

```
POST /v1/compress
Content-Type: application/json

{ "messages": [ ...provider-native message objects... ], "model": "gpt-4o" }
```

The system prompt is *not* included: it is the stable prefix that prompt-cache reuse
depends on. `model` is a tokenizer/context-window hint.

`forwardHeaderMatches` controls which incoming request headers are forwarded to the engine
(none by default). If your engine decides compressibility from headers, forward them
explicitly, e.g. `anthropic-version`, `anthropic-beta`, `openai-beta`, `cache-control`.
Credentials are never forwarded regardless of this setting.

The service responds `200` with the compressed array (an absent/null `messages` passes the
request through unchanged):

```
{ "messages": [ ...compressed message objects... ] }
```

Any non-200 status, a response without a `messages` array, or malformed message objects is
treated as a failure and resolved per `failureMode` (default `failOpen`: the original request
is forwarded unchanged). Content-level correctness is left to the compression service and
the upstream provider.

The gateway runs this as a request guard, so it executes after other prompt guards (which
see the original content) and before token counting (rate limits and cost reflect what is
actually sent).

### Running the example

Start the engine. With Headroom, use `--mode cache` so it freezes prior turns and keeps
provider prefix-cache reuse intact:

```bash
headroom proxy --port 8787 --mode cache
```
Or, you can use docker compose `docker compose -f examples/llm-context-compression/docker-compose.yaml up`

Then run the gateway:

```bash
export OPENAI_API_KEY=sk-...
cargo run -- -f examples/llm-context-compression/config.yaml
```

### Sending a request

Compression only helps when there is enough context to compress; requests below
`minSizeBytes` (16KiB in this example) skip the engine entirely. `gen-context.sh` emits a
large synthetic reference document to stdout, so you can feed it straight into the request
with process substitution — no file needed:

```bash
jq -n --rawfile ctx <(examples/llm-context-compression/gen-context.sh) '{
    model: "gpt-4o",
    messages: [{
      role: "user",
      content: ("Here is some reference material:\n\n" + $ctx + "\n\nWhat are the key takeaways?")
    }]
  }' | curl http://localhost:4000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "@-"
```

### Prompt caching: compression can cost more than it saves

On providers with prompt caching — OpenAI (automatic, prefix-based) and Anthropic (explicit
`cache_control` markers) — cache reads are far cheaper than fresh input. A compressor whose
output for a given message changes as the conversation grows (position-dependent compression)
rewrites the cached prefix on every turn — busting the cache usually costs more than
compression saves.

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

### Raw mode only applies to supported request formats

`messageFormat: raw` currently round-trips OpenAI Chat Completions and Anthropic Messages
requests (the formats with provider-native tool-call pairing to preserve). Requests in
other formats (embeddings, rerank, ...) skip the webhook entirely rather than failing.
