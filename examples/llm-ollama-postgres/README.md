## LLM Ollama + Postgres Example

This example configures agentgateway as an OpenAI-compatible LLM proxy for local Ollama models and stores request logs in Postgres.

### Running the example

Start Postgres:

```bash
docker compose -f examples/llm-ollama-postgres/docker-compose.yaml up -d
```

Start Ollama and make sure the model you want to use is available:

```bash
ollama pull llama3.2
```

Start agentgateway:

```bash
cargo run -- -f examples/llm-ollama-postgres/config.yaml
```

Send a request through agentgateway:

```bash
curl http://localhost:4000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama3.2",
    "messages": [
      {
        "role": "user",
        "content": "Say hello from Ollama"
      }
    ]
  }'
```

The Postgres database is exposed at:

```text
postgresql://agentgateway:agentgateway@localhost:5432/agentgateway
```
