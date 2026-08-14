## LLM Example

This example configures agentgateway as a simple LLM proxy.

Requests for the `smart` model are sent to OpenAI as `gpt-5.5`. Requests with models prefixed by `anthropic/` are sent to Anthropic with the prefix removed before forwarding.

### Running the example

Set provider API keys:

```bash
export OPENAI_API_KEY=...
export ANTHROPIC_API_KEY=...
```

Start agentgateway:

```bash
cargo run -- -f examples/llm-basic/config.yaml
```

Send a request to the OpenAI-backed `smart` model:

```bash
curl http://localhost:4000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "smart",
    "messages": [
      {
        "role": "user",
        "content": "Say hello from the smart model"
      }
    ]
  }'
```

Send an Anthropic request:

```bash
curl http://localhost:4000/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "model": "anthropic/claude-3-5-haiku-latest",
    "max_tokens": 64,
    "messages": [
      {
        "role": "user",
        "content": "Say hello from Anthropic"
      }
    ]
  }'
```
