## Compression Example

This example shows how to compress responses sent to clients and decompress request bodies sent to the backend.

### Running the example

Start an upstream HTTP server:

```bash
python3 -m http.server 8080
```

Start agentgateway:

```bash
cargo run -- -f examples/traffic-compression/config.yaml
```

Request a page while advertising compression support:

```bash
curl -s -o /dev/null -D - -H 'Accept-Encoding: br, gzip' http://localhost:3000/
```

The response has `content-encoding: br` and `vary: accept-encoding`. Without an `Accept-Encoding` header, the response is sent uncompressed.

The `compression` policy configures both directions:

```yaml
policies:
  compression:
    response:
      compress:
        algorithms: [gzip, brotli, zstd]
    request:
      decompress:
        algorithms: [gzip]
```

`response.compress` compresses textual responses (HTML, JSON, CSS, JavaScript, XML, plain text, SVG) using an algorithm the client accepts.
`request.decompress` decodes request bodies sent with `Content-Encoding: gzip`, after authentication and authorization, so later policies and the backend see the plain body.
