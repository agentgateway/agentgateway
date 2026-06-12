## AI Backend with CORS Example

This example shows how to expose an AI backend through agentgateway with a permissive CORS policy, so browser-based clients can call the AI endpoint directly.

The backend points at a local [Ollama](https://ollama.com/) server using its OpenAI-compatible API. Any OpenAI-compatible provider can be substituted by changing the `provider` block.

### Running the example

Start Ollama locally and pull the model used in the config:

```bash
ollama pull smallthinker
ollama serve
```

Then run agentgateway:

```bash
cargo run -- -f examples/ai-cors/config.yaml
```

The gateway listens on port `8080` and forwards requests to Ollama on `localhost:11434`.

### Trying it out

```bash
curl -s http://localhost:3000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"smallthinker","messages":[{"role":"user","content":"hello"}]}'
```

### The CORS policy

The route applies a permissive CORS policy so that browser clients on any origin can issue requests:

```yaml
policies:
  cors:
    allowCredentials: false
    allowHeaders: ["*"]
    allowMethods: [GET, POST, OPTIONS]
    allowOrigins: ["*"]
```

Tighten `allowOrigins` and `allowHeaders` to specific values for production deployments.
