## LLM Telemetry Example

This example shows how to export traces for LLM backend calls.

The `tracing/` directory contains provider-specific examples for OpenTelemetry-compatible backends such as Jaeger, Langfuse, OpenLLMetry, and Phoenix.

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
