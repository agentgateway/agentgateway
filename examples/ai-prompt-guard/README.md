## AI Prompt Guard Example

This example shows how to configure prompt guards for LLM requests and responses with agentgateway.

### Running the example

```bash
cargo run -- -f examples/ai-prompt-guard/config.yaml
```

The `promptGuard.request` and `promptGuard.response` fields define the regex rules to match against the request and response content, respectively. If a match is found, the specified action will be taken.

```yaml
policies:
  ai:
    promptGuard:
      request:
        regex:
          action:
            reject:
              response:
                body: "Request rejected due to inappropriate content"
          rules:
          - pattern: SSN
            name: SSN
          - pattern: Social Security
            name: Social Security
      response:
        regex:
          action:
            reject:
              response:
                body: "Response rejected due to inappropriate content"
          rules:
          - builtin: email
```

Example request containing `SSN` pattern rejected by the prompt guard:
```bash
curl http://localhost:3000   -H "Content-Type: application/json"   -H "Authorization: Bearer $OPENAI_API_KEY"   -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {
        "role": "system",
        "content": "You are a helpful assistant."
      },
      {
        "role": "user",
        "content": "Is 123-45-6789 a valid SSN"
      }
    ]
  }'
Request rejected due to inappropriate content
```

Example response containing `email` pattern rejected by the prompt guard:
```bash
curl http://localhost:3000   -H "Content-Type: application/json"   -H "Authorization: Bearer $OPENAI_API_KEY"   -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {
        "role": "system",
        "content": "You are a helpful assistant."
      },
      {
        "role": "user",
        "content": "Return a fake email address"
      }
    ]
  }'
Response rejected due to inappropriate content
```