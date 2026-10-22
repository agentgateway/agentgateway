## Context Compression Example

This example shrinks LLM request context with [Headroom](https://github.com/headroomlabs-ai/headroom)
before it reaches the provider, reducing token spend on long-context requests.

> [!NOTE]
> Context compression can degrade response quality or cost more when it invalidates a provider's
> prompt cache. Measure its effect with your own workloads before adopting it.

The external authorization policy sends the request's `messages` and `model` fields directly to
Headroom. It stores the compressed messages in external authorization metadata, then a request
transformation replaces the original messages. Requests under 16 KiB skip compression.

The policy allows requests through if Headroom cannot be reached and retains their original
messages.

### Run the example

Start Headroom:

```bash
docker compose -f examples/llm-context-compression/docker-compose.yaml up
```

Then run the gateway:

```bash
export OPENAI_API_KEY=sk-...
cargo run -- -f examples/llm-context-compression/config.yaml
```

Send a request with a large tool result. `gen-context.sh` generates a repetitive JSON log;
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

Compression is lossy: Headroom keeps a sample of the log rows, so the model may not see every
failing request.

### Prompt caching

With prompt caching, cached input is cheaper than fresh input. If compression output changes as
the conversation grows, it rewrites the cached prefix on every turn. Against cached providers,
run Headroom in a prefix-stable mode:

```bash
HEADROOM_MODE=cache \
HEADROOM_PROTECT_RECENT=0 \
HEADROOM_PROTECT_ANALYSIS_CONTEXT=0 \
HEADROOM_MIN_RATIO=0.75 \
HEADROOM_COMPRESS_MARKED_BLOCKS=1 \
headroom proxy --no-read-lifecycle
```
