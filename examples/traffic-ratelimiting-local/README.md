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

The `localRateLimit` policy allows 50 requests per second:

```yaml
policies:
  localRateLimit:
  - type: requests
    limit: 50
    unit: seconds
```

Change `limit` or `unit` in `config.yaml` to adjust the allowed request rate.
