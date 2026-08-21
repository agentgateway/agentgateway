## Bedrock attribution: the bill and the logs

This example carries a caller's identity from the gateway into both places AWS records Amazon Bedrock usage, using one set of CEL expressions:

- **The bill.** `assumeRole.sessionName` and `assumeRole.tags` ride the per-request STS `AssumeRole` call. The session name lands in CloudTrail and in the Cost and Usage Report's IAM principal column; the tags surface as cost allocation tags in Cost Explorer and the CUR once you activate the keys.
- **The logs.** `requestMetadata` is attached to every Bedrock call and recorded in the model invocation logs, which is where per-prompt attribution lives (CloudWatch Logs Insights, Athena). Bedrock does not enforce metadata and records whatever a caller sends; setting it at the gateway is what makes it mandatory and trustworthy.

Each entry is either a static `value` or a CEL `expression` evaluated against the request. Values derived from `jwt.*` come from a token the gateway validated; operator-set values come from this file. A caller cannot override an operator entry: if a client sends its own `x-bedrock-metadata` header, keys the operator claimed are replaced and the rest are kept, up to Bedrock's limit of 16 entries. An expression that errors, or produces an empty or invalid value, rejects the request before it reaches AWS.

Limits are checked at config load for static values and per request for dynamic ones: at most 16 entries, keys and values up to 256 characters, and the character set shared with STS tags.

### Running the example

The AWS credentials in the environment must be allowed to assume the configured role with `sts:AssumeRole` and `sts:TagSession`. Replace the role ARN and, if you use another model, the model ID.

```bash
cargo run -- -f examples/llm-bedrock-attribution/config.yaml
```

Send a request with a token (see `examples/mcp-authentication` for a signed test token):

```bash
curl http://localhost:4000/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H "x-team: platform" \
  -H "Content-Type: application/json" \
  -d '{"model": "amazon.nova-micro-v1:0", "messages": [{"role": "user", "content": "hello"}]}'
```

Then:

- **CloudTrail**, filter by event name `Converse`: `userIdentity.arn` ends with the caller's `jwt.sub`, not one shared session name.
- **Model invocation logs** (once enabled in the region): each record carries `requestMetadata.user`, `requestMetadata.team`, and `requestMetadata.environment`.
- **Cost Explorer**, after activating `user`, `team`, and `environment` as cost allocation tags and waiting for the next billing data: Bedrock spend grouped by any of them.

### Where the metadata goes on the wire

| Upstream API | Placement |
|---|---|
| `Converse`, `ConverseStream` (chat routes) | `requestMetadata` field in the request body |
| `InvokeModel`, `InvokeModelWithResponseStream` (embeddings, passthrough) | `x-amzn-bedrock-request-metadata` header, SigV4-signed |
| `CountTokens`, `Rerank` | not supported by Bedrock; left untouched |
