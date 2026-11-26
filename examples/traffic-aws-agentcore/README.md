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

### MCP targets in Kubernetes

For a runtime that serves MCP, add a `static.agentCore` target to an `AgentgatewayBackend` and reference that backend from an `HTTPRoute`. The target uses Streamable HTTP. The gateway builds the invocation URL from the ARN and optional qualifier and signs requests with SigV4 using the ARN's region.

```yaml
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: agentcore-mcp
spec:
  mcp:
    targets:
    - name: runtime
      static:
        agentCore:
          agentRuntimeArn: arn:aws:bedrock-agentcore:eu-west-1:123456789012:runtime/example
          qualifier: DEFAULT
```
