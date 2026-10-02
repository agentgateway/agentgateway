# Substrate actor egress

The egress policies use the protocol-based API from
[Substrate PR #1751](https://github.com/agent-substrate/substrate/pull/1751).

Configure `substrateEgressActorResolution` on the CONNECT frontend. Its control
API backend handles both `GetActor` (before returning 200 to CONNECT) and
`GetActorEgressPolicy` (when the inner TLS ClientHello arrives). Actor traffic is
classified from its first bytes, independently of the destination port.

Set `protocol: AUTO` on the internal destination bind and supply these protocol
handlers:

- An HTTP listener and HTTP route for cleartext requests.
- An HTTPS listener with `dynamicCa` TLS configuration and an HTTP route for
  intercepted HTTPS. Actors must trust that CA.
- A TLS listener without termination and a TCP route for passthrough.

These listeners can cover the same names. The actor's policy chooses HTTPS
interception or TLS passthrough before listener selection. An unmatched name or
port selects interception so the gateway can return 403 before routing or auth
policies run. This completes TLS with the actor without contacting an upstream;
an allowed HTTP authority cannot override that connection's denial. Missing or
invalid SNI, unsupported protocols, and failed policy lookups close the connection.
No matching handler also closes
the connection. Server-first connections time out during protocol detection.
Cleartext detection uses the gateway's existing standard HTTP method list and
HTTP/2 prior-knowledge preface; custom method tokens are not recognized.

Every HTTP route serving actor traffic must select a `substrateEgress` request
policy. It fetches the current policy on each request, checks the authority
against the HTTP or HTTPS rules for the inner transport, and applies only the
winning rule's effects. For HTTPS, the ClientHello SNI selects the certificate;
the request authority is authorized independently. Upgrades in Substrate are
denied (including CONNECT).

Both HTTP and TLS passthrough routes require dynamic backends. The gateway
resolves the authorized HTTP authority or TLS SNI through DNS and uses the
original CONNECT destination port. The CONNECT IP, an authority port, and a
configured dynamic target expression cannot override that destination.
Intercepted HTTPS defaults to TLS on dynamic upstream connections; `backendTLS`
can configure private roots or client certificates. Passthrough TCP routes use
the SNI and original destination port, with no backend TLS termination or
credential effects. Their policy decision lasts for the connection.

Exact names rank ahead of single-label wildcards, then `*`; explicit ports rank
ahead of all ports. Omitted HTTP/HTTPS ports default to 80/443. Passthrough requires
ports. `replace_headers` resolves credentials only for headers already present
in the actor's request, replacing any placeholder value.
