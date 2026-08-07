### LLM cost-aware routing

This example shows how to route one public virtual model to different upstream models using request data.

The important distinction is:

- `smart-model` is the public virtual model name the caller sends
- `economy`, `balanced`, and `premium` are gateway routing tiers
- `economy-model`, `balanced-model`, and `premium-model` are internal concrete model targets
- `gpt-4o-mini` and `gpt-4o` are the provider model names those targets use

The `smart-model` virtual model uses conditional routing. Its CEL conditions read the already-parsed `llmRequest`, checking `metadata.cost_tier` first and otherwise using `max_tokens` as the requested output-cost signal:

```yaml
virtualModels:
- name: smart-model
  routing:
    conditional:
      targets:
      - when: 'default(llmRequest.metadata.cost_tier, "") == "economy" || (default(llmRequest.metadata.cost_tier, "") == "" && default(llmRequest.max_tokens, 1024) <= 1024)'
        model: economy-model
      - when: 'default(llmRequest.metadata.cost_tier, "") == "balanced" || (default(llmRequest.metadata.cost_tier, "") == "" && default(llmRequest.max_tokens, 1024) <= 4096)'
        model: balanced-model
      - model: premium-model
```

Conditional targets are checked in order, and the last target is the fallback. In this example:

- `economy` routes to `gpt-4o-mini`
- `balanced` routes to `gpt-4o`
- `premium` also routes to `gpt-4o`
- callers can explicitly request a tier with `metadata.cost_tier`

This keeps the public API stable without adding a temporary routing header to the request.

Run the gateway:

```shell
cargo run -- -f examples/llm-cost-routing/config.yaml
```

Replace the placeholder `apiKey` values in `config.yaml` before sending requests to a real provider.

Example economy request:

```shell
curl -s http://localhost:4000/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"smart-model","messages":[{"role":"user","content":"summarize this"}],"max_tokens":256}'
```

Example premium override:

```shell
curl -s http://localhost:4000/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"smart-model","messages":[{"role":"user","content":"reason carefully"}],"max_tokens":256,"metadata":{"cost_tier":"premium"}}'
```
