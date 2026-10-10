## iFlytek Astron MaaS Example

This example shows how to route requests through agentgateway to iFlytek
Astron MaaS (讯飞星辰 MaaS) using the built-in `openAI` provider with a custom
`params.baseUrl`. Both configs use Spark X2.5 (`spark-x2.5`, 256K context).

- **Pay-as-you-go** (`config.yaml`) — `https://maas-api.cn-huabei-1.xf-yun.com/v2`.
  See the [API quick start](https://maas.xfyun.cn/doc-viewer?url=https://maas.xfyun.cn/doc/guide/1%E3%80%81%E5%BF%AB%E9%80%9F%E5%BC%80%E5%A7%8B/1.1%20API%E6%8E%A5%E5%85%A5.html).
- **Token Plan** (`token-plan-config.yaml`) — `https://maas-token-api.cn-huabei-1.xf-yun.com/v2`.
  The plan's model list is in the [Token Plan documentation](https://www.xfyun.cn/doc/spark/TokenPlan.html);
  `xsparkx2flash` is the lighter option there.

The two hosts speak the OpenAI chat-completions format, so the built-in `openAI`
provider only needs a base URL and an API key. A Token Plan key only works on
the Token Plan host.

### Running the example

Create an API key in the [MaaS console](https://maas.xfyun.cn/), export it and
start agentgateway:

```bash
export SPARK_API_PASSWORD=your-maas-api-key
cargo run -- -f examples/llm-iflytek/config.yaml
```

Then send an OpenAI-style request:

```bash
curl -s http://localhost:3000/v1/chat/completions -H 'Content-Type: application/json' \
  -d '{"model":"spark-x2.5","max_tokens":2048,"messages":[{"role":"user","content":"用一句话介绍合肥"}]}'
```

Spark X2.5 is a reasoning model with thinking on by default. The reasoning comes
back in `reasoning_content`, and if `max_tokens` is too small it can be used up
before any `content` is written (`finish_reason` is then `length`).

For the Token Plan, export the plan's key and use the other config:

```bash
export ASTRON_API_KEY=your-token-plan-api-key
cargo run -- -f examples/llm-iflytek/token-plan-config.yaml
```

See https://agentgateway.dev/docs/llm/providers/ for more on provider parameters
and authentication approaches.
