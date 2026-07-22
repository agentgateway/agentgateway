## SPIFFE Workload API Example

This example shows how to source agentgateway's mTLS identity from the local
[SPIFFE Workload API](https://github.com/spiffe/spiffe/blob/main/standards/SPIFFE_Workload_API.md),
on both the serving (listener) side and the upstream (backend) side.

Unlike the static [TLS](../tls) example, no certificate or key files are configured: the
gateway fetches its X.509-SVID and the trust bundle from the Workload API endpoint and rotates
them automatically.

### Prerequisites

- A running SPIFFE Workload API provider (for example, a [SPIRE](https://spiffe.io/docs/latest/spire-about/)
  agent) with an entry registered for the gateway workload
- A SPIFFE-aware client and backend

### Running the example

```bash
cargo run -- -f examples/traffic-spiffe/config.yaml
```

### What it configures

An HTTPS listener that gets its cert and key from the Workload API endpoint, and accepts requests from clients that present
a valid certificate. The verified SPIFFE ID is forwarded to the backend in the `x-client-spiffe-id` header via the
CEL `source.spiffeId` value.


**Upstream identity (port 3001).** The `backendTLS.spiffe: {}` policy makes the gateway
present its own SVID to the upstream and verify the upstream's certificate against the SPIFFE
trust bundle. SPIFFE SVIDs carry a `spiffe://` URI SAN and no DNS SAN, so hostname checks do
not apply; by default any upstream SVID chaining to the trust bundle is accepted. Pin
specific upstream identities with `subjectAltNames`.

```yaml
backendTLS:
  spiffe: {}
  subjectAltNames:
  - spiffe://example.org/ns/default/sa/upstream
```

> **Trust-domain scope:** all authorities in the SPIFFE bundle set (including any federated
> trust domains) are accepted. Restrict further by pinning upstream SPIFFE IDs with
> `subjectAltNames`, or, on the serving side, with a CEL authorization policy on
> `source.spiffeId`.
