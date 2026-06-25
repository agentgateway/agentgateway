## iFlytek Spark / Astron MaaS Example

This example shows how to route requests through agentgateway to iFlytek's
OpenAI-compatible LLM endpoints using the built-in `openAI` provider with a
host override.

- **iFlytek Spark** (`config.yaml`) — the Spark HTTP API at
  `https://spark-api-open.xf-yun.com/v1`. Models include `generalv3.5`,
  `4.0Ultra`, `max-32k`, `pro-128k`, and `lite`.
- **iFlytek Astron MaaS** (`astron-maas-config.yaml`) — the Astron MaaS Token
  Plan at `https://maas-token-api.cn-huabei-1.xf-yun.com/v2`. Switch the host to
  `maas-coding-api.cn-huabei-1.xf-yun.com` and the model to `astron-code-latest`
  for the Coding Plan.

Both endpoints speak the OpenAI chat-completions format, so only the upstream
host (and, for Astron MaaS, the `/v2` base path) needs to be overridden.

### Running the example

Spark authenticates with an HTTP API password (`APIPassword`) obtained from
[xinghuo.xfyun.cn/sparkapi](https://xinghuo.xfyun.cn/sparkapi). Export it and
start agentgateway:

```bash
export SPARK_API_PASSWORD=your-spark-api-password
cargo run -- -f examples/iflytek/config.yaml
```

Then send an OpenAI-style request:

```bash
curl -s http://localhost:3000/v1/chat/completions -H 'Content-Type: application/json' \
  -d '{"model":"generalv3.5","messages":[{"role":"user","content":"用一句话介绍合肥"}]}'
```

For Astron MaaS, export the plan's API key and use the other config:

```bash
export ASTRON_API_KEY=your-astron-maas-api-key
cargo run -- -f examples/iflytek/astron-maas-config.yaml
```

See https://agentgateway.dev/docs/llm/providers/ for more on provider overrides
and `backendAuth` approaches.
