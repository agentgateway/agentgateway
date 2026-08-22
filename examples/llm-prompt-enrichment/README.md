## Prompt Enrichment Example

This example shows how to use agentgateway to enrich prompts before requests are sent to AI providers.

### Running the example

This example uses OpenAI as a provider. Export an API key before running the gateway:

```bash
export OPENAI_API_KEY=...
```

```bash
cargo run -- -f examples/llm-prompt-enrichment/config.yaml
```

In this example, the configuration appends a system prompt instructing the LLM to respond with emojis only.

```bash
$ curl -s http://localhost:3000/v1/chat/completions -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -d '{"model":"gpt-5-nano","messages":[{"role":"user","content":"write a haiku about ai"}]}'

..."content":"🤖 🧠 💡 🌐 ✨\n📚 🧠 🔎 🛰️ 💫 🧭 🧩\n🌅 🤝 🌍 🔬 🔮"...
```

Prompt enrichment allows agentgateway to `append` or `prepend` multiple prompts with various role types, assuming the AI provider supports them.
