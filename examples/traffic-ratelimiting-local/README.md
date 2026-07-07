## Local Rate Limiting Example

This example shows how to apply local rate limiting to ordinary HTTP traffic.

### Running the example

Start an upstream HTTP server:

```bash
python3 -m http.server 8080
```

Start agentgateway:

```bash
cargo run -- -f examples/traffic-ratelimiting-local/config.yaml
```

Send requests through the gateway:

```bash
curl http://localhost:3000/
```

The `localRateLimit` policy allows 10 requests per minute:

```yaml
policies:
  localRateLimit:
  - maxTokens: 10
    tokensPerFill: 1
    fillInterval: 60s
```

Increase `maxTokens`, `tokensPerFill`, or shorten `fillInterval` in `config.yaml` to change the allowed request rate.
