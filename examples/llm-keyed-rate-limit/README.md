## Keyed Local Rate Limiting Example

This example gives every caller its own local rate limit, keyed by the claims in their JWT.

A `localRateLimit` rule without a `key` holds all callers to one bucket, so the busiest one
exhausts the quota for everyone else. A `key` is a CEL expression evaluated per request, and each
distinct value it produces gets its own bucket with the rule's limits. Requests without a value,
or whose expression cannot be evaluated, share one bucket.

The config applies three rules, and all of them have to admit the request:

```yaml
localRateLimit:
# Each user may send 60 requests a minute.
- type: requests
  maxTokens: 60
  tokensPerFill: 60
  fillInterval: 60s
  key: jwt.sub
# Each team shares 100k tokens an hour.
- type: tokens
  maxTokens: 100000
  tokensPerFill: 100000
  fillInterval: 1h
  key: jwt.team
# Each user also gets a separate token allowance per model.
- type: tokens
  maxTokens: 20000
  tokensPerFill: 20000
  fillInterval: 1h
  key: jwt.sub + "/" + llm.requestModel
```

A `requests` rule is checked before the request body is read, so its key sees the request alone.
A `tokens` rule is charged once the LLM request has been parsed, so its key can also read the
requested model, and the bucket is settled with the real input and output token counts once the
response is known.

### Running the example

Point `baseUrl` at an OpenAI-compatible endpoint, and start agentgateway:

```bash
cargo run -- -f examples/llm-keyed-rate-limit/config.yaml
```

Send a request with one of the bundled tokens:

```bash
curl -H "Authorization: Bearer $(cat manifests/jwt/example1.key)" \
  http://localhost:4000/v1/chat/completions \
  -d '{"model": "qwen3", "messages": [{"role": "user", "content": "hi"}]}'
```

The response carries the tightest of the limits that admitted it:

```
x-ratelimit-limit: 60
x-ratelimit-remaining: 59
x-ratelimit-reset: 41
```

Once a bucket is empty its caller gets a `429`, and a request with a different `sub` still gets
through. The bundled tokens carry no `team` claim, so the second rule cannot evaluate its key and
counts them all in one bucket; `manifests/jwt/README.md` shows how to sign a token with claims of
your own.

Buckets live in the proxy instance that created them, so with several replicas each holds the
configured limit. Use `remoteRateLimit` for a quota shared across a fleet.
