## TLS Example

This example shows how expose agentgateway on an HTTPS listener.

### Running the example

```bash
cargo run -- -f examples/tls/config.yaml
```

Beyond the [basic](../basic) configuration, we have changed our listener to be of protocol `HTTPS` and add `tls` information, using an example key and certificate.

```yaml
listeners:
- name: default
  protocol: HTTPS
  tls:
    cert: examples/tls/certs/cert.pem
    key: examples/tls/certs/key.pem
```

This example currently won't work with the `mcpinspector` as it doesn't support unverified TLS certificates.
However, we can use `curl` to send a request to verify the TLS is working properly:

```bash
$ curl https://localhost:3000 -k
Not Acceptable: Client must accept text/event-stream
```

Note the `-k` to disable TLS verification, as the example certificate is self-signed.
The request fails as we did not pass a valid MCP request, but this shows the TLS was handled properly.

### Workload Identity (Istio mTLS)

When running in an Istio service mesh, you can use workload identity instead of static certificates. This enables automatic certificate rotation and mesh-native mTLS:

```yaml
listeners:
- name: default
  protocol: HTTPS
  tls:
    workloadIdentity: true
```

This requires the following environment variables:
- `CA_ADDRESS`: Istio CA address (e.g., `istiod.istio-system.svc:15012`)
- `TRUST_DOMAIN`: Mesh trust domain (e.g., `cluster.local`)
- `NAMESPACE`: Pod namespace
- `SERVICE_ACCOUNT`: Pod service account

With workload identity, agentgateway obtains certificates from Istio CA and validates client certificates against the mesh trust domain.
