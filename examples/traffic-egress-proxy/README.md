# Egress proxy

Use agentgateway as an HTTP CONNECT proxy to control which HTTPS destinations
clients can reach. Configure clients with an HTTP proxy URL such as:

```bash
export HTTPS_PROXY=http://127.0.0.1:3000
```

This directory contains three configurations:

- [config.yaml](config.yaml) allowlists HTTPS destinations without decrypting
  their traffic.
- [config-conditional.yaml](config-conditional.yaml) passes most HTTPS traffic
  through unchanged, but terminates TLS for selected hostnames so agentgateway
  can serve LLM and HTTP routes.
- [config-dynamic-tls.yaml](config-dynamic-tls.yaml) terminates all HTTPS
  traffic, allowing HTTP policies to inspect or modify every request and
  response.

Run only one configuration at a time. Each one listens for proxy requests on
port 3000 and handles HTTPS destinations on port 443.

## Allowlist HTTPS destinations

Use `config.yaml` when you only need to restrict outbound HTTPS by hostname.
Agentgateway uses the TLS SNI hostname for routing and forwards the encrypted
connection without decrypting it.

Edit `tcpRoutes[].hostnames` to define the allowed destinations:

```yaml
hostnames:
- pypi.org
- files.pythonhosted.org
- api.github.com
```

Start the proxy:

```bash
agentgateway -f examples/traffic-egress-proxy/config.yaml
```

In another terminal, verify an allowed destination:

```bash
curl --proxy http://127.0.0.1:3000 --head https://pypi.org/
```

Verify that a destination not present in the allowlist is blocked:

```bash
curl --proxy http://127.0.0.1:3000 https://example.com/
```

The second command should fail because `example.com` has no matching route.
Package managers often download from several hostnames, so include artifact and
redirect destinations such as `files.pythonhosted.org`, not only the primary
site.

To send a permitted hostname to a different destination, set a TCP dynamic
target expression. It must return a `host:port` string:

```yaml
tcpRoutes:
- hostnames:
  - pypi.org
  backends:
  - dynamic:
      target: 'destination.hostname == "pypi.org" ? "mirror.example.com:8443" : destination.hostname + ":" + string(destination.port)'
```

TCP target expressions can use `destination.hostname`,
`destination.address`, `destination.port`, and `source.*`. This form is scoped
to `tcpRoutes`; dynamic backends are not accepted as the destination for policy
calls.

## Terminate TLS for selected hostnames

Use `config-conditional.yaml` when most destinations should remain encrypted
end to end, but selected hostnames should be handled by agentgateway.

The example configures three behaviors:

- `llm.example.com` terminates TLS and serves the top-level `llm` API.
- `static.example.com` terminates TLS and returns a configured response.
- The public hostname allowlist passes TLS through without decrypting it.

Replace the example certificates with certificates valid for the hostnames you
configure. The checked-in development certificate is not valid for
`llm.example.com` or `static.example.com`, so the validation commands below use
`--insecure`.

Start the proxy with a placeholder provider key:

```bash
OPENAI_API_KEY=dummy agentgateway \
  -f examples/traffic-egress-proxy/config-conditional.yaml
```

Verify that an allowlisted public destination still works normally:

```bash
curl --proxy http://127.0.0.1:3000 --head https://pypi.org/
```

Verify the direct-response route:

```bash
curl --proxy http://127.0.0.1:3000 --insecure \
  https://static.example.com/
```

The response body should be:

```text
hello from agentgateway
```

Verify that the LLM hostname serves the configured model catalog:

```bash
curl --proxy http://127.0.0.1:3000 --insecure \
  https://llm.example.com/v1/models
```

The response should contain a model with the ID `smart`. Listing models is
handled locally, so this command does not call OpenAI or use the placeholder
key.

## Terminate all HTTPS traffic

Use `config-dynamic-tls.yaml` when agentgateway must apply HTTP policies to
every outbound request. The proxy generates a certificate for each requested
hostname using the configured CA, then creates a separate TLS connection to the
destination.

Clients must trust that CA. Do not distribute the example CA outside local
testing; generate and protect a CA appropriate for your environment.

Start the proxy:

```bash
agentgateway -f examples/traffic-egress-proxy/config-dynamic-tls.yaml
```

The example adds one request header and one response header. Verify both against
an endpoint that echoes request headers:

```bash
curl --proxy http://127.0.0.1:3000 \
  --cacert examples/mcp-tls/certs/ca-cert.pem \
  --include https://httpbingo.org/headers
```

The HTTP response headers should include:

```text
x-agentgateway-resp-message: Hello from AgentGateway!
```

The JSON response body should show the header added to the upstream request:

```text
X-Agentgateway-Req-Message: Hello from AgentGateway!
```

Because this configuration has no hostname allowlist, it permits every HTTPS
destination. Add hostname-specific listeners or routes if it must also enforce
an egress allowlist.
