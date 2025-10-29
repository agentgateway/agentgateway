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

### Custom Response Headers

You can customize the HTTP headers in rejection responses. This is particularly useful for:
- Returning JSON errors with proper `Content-Type` for OpenAI-compatible clients
- Adding custom metadata to rejection responses
- Removing unwanted default headers

#### JSON Error Response (OpenAI-Compatible)

```yaml
promptGuard:
  request:
    regex:
      action:
        reject:
          response:
            status: 400
            headers:
              set:
                content-type: "application/json"
            body: |
              {
                "error": {
                  "message": "Request blocked by content moderation",
                  "type": "invalid_request_error",
                  "code": "content_policy_violation"
                }
              }
      rules:
      - builtin: ssn
```

#### Multiple Header Values

Headers can be set (single value), added (multiple values), or removed:

```yaml
rejection:
  status: 403
  headers:
    set:
      content-type: "application/json"
      x-moderation-version: "v1"
    add:
      x-blocked-category: "violence"
      x-blocked-category: "hate"
    remove:
      - server
  body: '{"error": "Forbidden"}'
```

**Header Operations:**
- `set`: Replace or create a header (overwrites existing value)
- `add`: Append a header value (allows multiple values for the same header)
- `remove`: Remove a header from the response

### Guardrails Webhook

A webhook can be used to reject or mask content sent to or received from the LLM.

Example policy to forward the request and response to a webhook for moderation:
```yaml
policies:
  ai:
    promptGuard:
      request:
        webhook:
          target: 127.0.0.1:8000
          # By default, request headers are not forwarded.
          # forwardHeaderMatches specifies a list of header matchers to use
          # to determine the request headers to forward to the webhook
          forwardHeaderMatches:
          - name: h1
            value:
              regex: v1
          - name: h2
            value:
              regex: v2.*
      response:
        webhook:
          target: 127.0.0.1:8000
          # set forwardHeaderMatches for to forward response headers