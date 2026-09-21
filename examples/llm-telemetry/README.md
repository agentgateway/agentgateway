## LLM Telemetry Example

This example shows how to export traces for LLM backend calls.

The `tracing/` directory contains provider-specific examples for OpenTelemetry-compatible backends such as Jaeger, Langfuse, OpenLLMetry, and Phoenix.

For Datadog metrics and OTLP tracing with a synthetic provider, see the
[Datadog observability example](../datadog/README.md).

### Running the example

Start agentgateway with the OpenTelemetry tracing config:

```bash
cargo run -- -f examples/llm-telemetry/tracing/otel.yaml
```

Send a request to the LLM provider:

```bash
curl "http://localhost:3000/" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $GEMINI_API_KEY" \
  -d '{
    "model": "gemini-2.0-flash",
    "messages": [
      {
        "role": "user",
        "content": "Explain how AI works"
      }
    ]
  }'
```

### Finish reasons

Default structured request logs and traces include `gen_ai.response.finish_reasons`
as a string array, even with completion and tool-call logging disabled. Reasons use
the client-facing protocol's values, such as `stop`, `length`, `tool_calls`,
`end_turn`, or `MAX_TOKENS`. Each observed generation retains its own position,
including duplicate reasons. If an expected reason never arrives, its position is
reported as `error`, as recommended by the
[pinned GenAI conventions](https://github.com/open-telemetry/semantic-conventions-genai/blob/c88d504ab3d9879f8e50d3cc87e69775e11db234/docs/gen-ai/gen-ai-spans.md).
Responses API generations report `completed` or `incomplete`; failed and cancelled
generations report `error`. Buffered background replies with `queued` or
`in_progress` status omit finish reasons because generation is still pending.

Finish reasons are available in CEL as `llm.finishReasons`. To add the first reason
as an optional metric label, merge this into your configuration:

```yaml
config:
  metrics:
    fields:
      add:
        finish_reason: >-
          llm != null && has(llm.finishReasons) && size(llm.finishReasons) > 0
          ? llm.finishReasons[0] : 'unknown'
```

This adds a label to existing metrics. Default metric labels remain unchanged.
The guard handles requests without generation metadata, including non-LLM and
non-generation requests. Streaming reasons are finalized when the response ends;
CEL expressions evaluated earlier may see incomplete metadata.
