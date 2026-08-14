## TLS Example

This example shows how expose agentgateway on an HTTPS listener.

### Running the example

```bash
cargo run -- -f examples/mcp-tls/config.yaml
```

Beyond the [basic](../mcp-basic) configuration, we have changed our listener to be of protocol `HTTPS` and add `tls` information, using an example key and certificate.

```yaml
listeners:
- name: default
  protocol: HTTPS
  tls:
    cert: examples/mcp-tls/certs/cert.pem
    key: examples/mcp-tls/certs/key.pem
```

This example currently won't work with the `mcpinspector` as it doesn't support unverified TLS certificates.
However, we can use `curl` to send a request to verify the TLS is working properly:

```bash
$ curl https://localhost:3000 -k
Not Acceptable: Client must accept text/event-stream
```

Note the `-k` to disable TLS verification, as the example certificate is self-signed.
The request fails as we did not pass a valid MCP request, but this shows the TLS was handled properly.
