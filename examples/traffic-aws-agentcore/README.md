## AWS AgentCore Example

This example proxies HTTP traffic to an AWS Bedrock AgentCore runtime.

### Running the example

Update `agentRuntimeArn` in `config.yaml` for your runtime, then start agentgateway:

```bash
cargo run -- -f examples/traffic-aws-agentcore/config.yaml
```

Send traffic to the configured route:

```bash
curl http://localhost:3000/supply-chain-agent
```

The route forwards to AgentCore and sets the AgentCore user-id headers before the request is sent upstream.
