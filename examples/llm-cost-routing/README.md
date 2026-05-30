### LLM cost-aware routing

This example shows how to route one public model name to different upstream models using request-time policy.

The important distinction is:

- `smart-model` is the public name the caller sends
- `economy`, `balanced`, and `premium` are gateway routing tiers
- `gpt-4o-mini` and `gpt-4o` are the concrete upstream models the gateway can pick

The gateway transformation computes `x-gateway-cost-class` from the request body. That header is only a routing signal; it is removed before the request reaches the provider. The example uses a plain CEL expression, not a custom helper:

```yaml
x-gateway-cost-class: 'default(json(request.body).metadata.cost_tier, "") != "" ? default(json(request.body).metadata.cost_tier, "") : (default(json(request.body).max_tokens, 1024) > 4096 ? "premium" : default(json(request.body).max_tokens, 1024) > 1024 ? "balanced" : "economy")'
```

The model entries then use normal header matches. In this example:

- `economy` routes to `gpt-4o-mini`
- `balanced` routes to `gpt-4o`
- `premium` also routes to `gpt-4o`
- callers can explicitly request a tier with `metadata.cost_tier`

This keeps the public API stable while letting the gateway make a policy decision before model selection.

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
