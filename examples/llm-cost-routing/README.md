### LLM cost-aware routing

This example shows how to route one public model name to different upstream models using request-time policy.

The gateway transformation computes `x-gateway-cost-class` from the request body:

```yaml
x-gateway-cost-class: 'llm.costClass(default(json(request.body).max_tokens, 1024), 1024, 4096, default(json(request.body).metadata.cost_tier, ""))'
```

The model entries then use normal header matches:

- `economy` routes to a lower-cost upstream model
- `balanced` routes to the default higher-capability upstream model
- `premium` routes to a higher-capability upstream model
- callers can explicitly request a tier with `metadata.cost_tier`

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
