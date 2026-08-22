## iFlytek Spark / Astron MaaS Example

This example shows how to route requests through agentgateway to iFlytek's
OpenAI-compatible LLM endpoints using the built-in `openAI` provider with a
custom `params.baseUrl`.

- **iFlytek Spark** (`config.yaml`) — the Spark HTTP API at
  `https://spark-api-open.xf-yun.com/v1`, using the current `4.0Ultra` model.
  See the [official HTTP API documentation](https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html).
- **iFlytek Astron MaaS** (`astron-maas-config.yaml`) — the Astron MaaS Token
  Plan at `https://maas-token-api.cn-huabei-1.xf-yun.com/v2`, using `xsparkx2`.
  See the [Token Plan documentation](https://www.xfyun.cn/doc/spark/TokenPlan.html).
  For the Coding Plan, use `https://maas-coding-api.cn-huabei-1.xf-yun.com/v2`
  with `astron-code-latest`; see its [official documentation](https://www.xfyun.cn/doc/spark/CodingPlan.html).

Both endpoints speak the OpenAI chat-completions format, so the built-in
`openAI` provider only needs a custom base URL and API key.

### Running the example

Spark authenticates with an HTTP API password (`APIPassword`) obtained from
[xinghuo.xfyun.cn/sparkapi](https://xinghuo.xfyun.cn/sparkapi). Export it and
start agentgateway:

```bash
export SPARK_API_PASSWORD=your-spark-api-password
cargo run -- -f examples/llm-iflytek/config.yaml
```

Then send an OpenAI-style request:

```bash
curl -s http://localhost:3000/v1/chat/completions -H 'Content-Type: application/json' \
  -d '{"model":"4.0Ultra","messages":[{"role":"user","content":"用一句话介绍合肥"}]}'
```

For Astron MaaS, export the plan's API key and use the other config:

```bash
export ASTRON_API_KEY=your-astron-maas-api-key
cargo run -- -f examples/llm-iflytek/astron-maas-config.yaml
```

See https://agentgateway.dev/docs/llm/providers/ for more on provider parameters
and authentication approaches.
