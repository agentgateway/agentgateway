## HTTP Example

This example shows using agentgateway as a standard HTTP proxy.

### Running the example

```bash
cargo run -- -f examples/traffic-http/config.yaml
```

The example contains a few HTTP routes that demonstrate matching, policy application, direct responses, and authorization rules.

```yaml
binds:
- port: 3000
  listeners:
  - protocol: HTTP
    routes:
    - name: match-example
      matches:
      - path:
          pathPrefix: /match
        method: GET
      backends:
      - host: 127.0.0.1:8080
```

We have a few concepts to understand here:
* `binds` represent each port our server listens on. In this case, we will listen on port 3000.
* `listeners` contain groups of HTTP routes.
* `routes` match requests and attach traffic policies.
* `backends` define where matching requests are forwarded.

Start a simple upstream server for the proxied routes:

```bash
python3 -m http.server 8080
```

Try the matched route:

```bash
curl '127.0.0.1:3000/match?param=hello' -H 'x-header: test-0'
```

The `/direct` and `/ips` routes return direct responses without contacting an upstream.
