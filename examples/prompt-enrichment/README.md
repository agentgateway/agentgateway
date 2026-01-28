## Prompt Enrichment Example

This example shows how to use agentgateway to enrich prompts before requests are sent to AI providers.

### Running the example

This example uses OpenAI as a provider which requires API credentials. For the simplest solution run
agentgateway with an exported API key and add the following to the end of the configuration. For additional 
providers and `backendAuth` approaches, see https://agentgateway.dev/docs/llm/providers/

```yaml
...
policies:
  ai:
    prompts:
      append:
      - role: system
        content: Respond with emojis only
  backendAuth:
    key: $OPENAI_API_KEY      
```

```bash
cargo run -- -f examples/prompt-enrichment/config.yaml
```

In this example, the configuration appends a system prompt instructing the LLM to respond with emojis only.

```bash
$ curl -s http://localhost:3000/v1/chat/requests -H 'Content-Type: application/json' \
  -d '{"model":"gpt-5-nano","messages":[{"role":"user","content":"write a haiku about ai"}]}'

..."content":"🤖 🧠 💡 🌐 ✨\n📚 🧠 🔎 🛰️ 💫 🧭 🧩\n🌅 🤝 🌍 🔬 🔮"...
```

Prompt enrichment allows agentgateway to `append` or `prepend` multiple prompts with various role types, assuming the AI provider supports them.
