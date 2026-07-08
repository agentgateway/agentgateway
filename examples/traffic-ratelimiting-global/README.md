## Global Rate Limiting Example

This example shows how to apply global rate limiting to ordinary HTTP traffic with Envoy's ratelimit service and Redis.

### Running the example

Start Redis and the ratelimit server:

```bash
docker run -d --name redis --network host redis:7.4.3
docker run -d --name ratelimit \
  --network host \
  -e REDIS_URL=127.0.0.1:6379 \
  -e USE_STATSD=false \
  -e LOG_LEVEL=debug \
  -e REDIS_SOCKET_TYPE=tcp \
  -e RUNTIME_ROOT=/data \
  -e RUNTIME_SUBDIRECTORY=ratelimit \
  -v $(pwd)/examples/traffic-ratelimiting-global/ratelimit-config.yaml:/data/ratelimit/config/config.yaml:ro \
  envoyproxy/ratelimit:3e085e5b \
  /bin/ratelimit -config /data/ratelimit/config/config.yaml
```

Start an upstream HTTP server:

```bash
python3 -m http.server 8080
```

Start agentgateway:

```bash
cargo run -- -f examples/traffic-ratelimiting-global/config.yaml
```

Send requests through the gateway:

```bash
curl http://localhost:3000/
```

The `remoteRateLimit` policy sends request descriptors to the ratelimit service:

```yaml
policies:
  remoteRateLimit:
    domain: agentgateway
    host: 127.0.0.1:8081
    descriptors:
    - entries:
      - key: method
        value: request.method
      - key: path
        value: request.path
      type: requests
```

Monitor rate limit decisions:

```bash
docker logs -f ratelimit | grep -E '(OVER_LIMIT|OK)'
```
