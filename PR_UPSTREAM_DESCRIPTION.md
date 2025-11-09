Title: Backend HTTP policy with ALPN enforcement and observability

Summary

- Introduces a general backend HTTP policy (`policies.http.version: "1.1" | "2"`) applicable to all backends (AI and Service).
- Enforces HTTP/1.1 deterministically over TLS by restricting ALPN to `http/1.1` when policy is set to `"1.1"`.
- Adds lightweight observability: structured fields `upstream.http_version` and `upstream.tls.alpn` in request logs (parity across backend types) plus a debug trace of the selection.
- Implements precedence and heuristics in AI path: `policy > appProtocol > heuristics` (TLS→h1, gRPC→h2, plaintext mirrors downstream version).

Motivation

Local AI backends like vLLM/SGLang often support only HTTP/1.1. Without control, ALPN may negotiate h2 over TLS or clients may send h2c prior-knowledge in plaintext, leading to failures (e.g., `PRI * HTTP/2.0`). This PR provides per-backend control with sensible defaults and clear observability, without adding extra proxy layers.

Behavior

- Policy (preferred):
  - `policies.http.version: "1.1"` → sets upstream HTTP/1.1; if TLS is configured, clamps ALPN to `http/1.1`.
  - `policies.http.version: "2"` → prefers HTTP/2 (best-effort; requires ALPN negotiation).
- Precedence and heuristics:
  - Precedence: `policy > appProtocol > heuristics`.
  - Heuristics: TLS downstream ⇒ HTTP/1.1 (except gRPC ⇒ HTTP/2); plaintext ⇒ mirror downstream version.
  - No implicit TLS: setting HTTP version never enables TLS on a plaintext backend.
  - Plaintext h2c caveat: if a downstream client uses HTTP/2 prior-knowledge (h2c), the mirrored version may be HTTP/2. For HTTP/1.1-only backends (e.g., local AI servers), set `policies.http.version: "1.1"` to avoid h2c propagation.
 - HTTP/2 hygiene: when sending HTTP/2 upstream, drop hop-by-hop headers (`Connection`, `Transfer-Encoding`).

Config Example (YAML)

```yaml
backends:
  - ai:
      name: qwen3-32b
      hostOverride: 192.168.1.10:8001
      provider:
        openAI:
          model: qwen3-32b
    policies:
      http:
        version: "1.1"
```

Key Changes

- New policy type: `http::backend::HTTP` with `HttpVersion` ("1.1" | "2").
  - file: crates/agentgateway/src/http/backend.rs
- Policy plumbing:
  - `BackendPolicy::HTTP` added; merge logic and YAML parsing wired in.
  - files: crates/agentgateway/src/types/agent.rs, crates/agentgateway/src/store/binds.rs, crates/agentgateway/src/types/local.rs
- Proxy (AI path): precedence + heuristics, and ALPN clamp for policy="1.1".
  - file: crates/agentgateway/src/proxy/httpproxy.rs
- TLS/ALPN: add `BackendTLS::with_alpn_http11()` helper.
  - file: crates/agentgateway/src/http/backendtls.rs
- Observability: add `upstream.http_version`, `upstream.tls.alpn` to logs.
  - file: crates/agentgateway/src/telemetry/log.rs
  - Note: `upstream.tls.alpn` reflects the configured/clamped ALPN when policy="1.1". Negotiated ALPN can be added in a follow-up if needed.

Compatibility

- Backward compatible. The policy is optional. If omitted, behavior follows existing `appProtocol` for Service backends and heuristics for AI.
- Does not implicitly enable TLS.

Testing

- Unit
  - `crates/agentgateway/tests/http_policy.rs`: verifies `HTTP` policy helpers (`version_override`, `is_http11`, `is_http2`).
- Integration (manual/E2E)
  - TLS upstream advertising h2 + `policies.http.version: "1.1"` ⇒ ALPN is `http/1.1`; upstream HTTP/1.1 is used.
  - Plaintext HTTP/1.1-only backends ⇒ no h2c prior knowledge.
  - Logs include `upstream.http_version` and (when TLS) `upstream.tls.alpn`.

Follow-ups

- If desired, extend observability to include negotiated ALPN at connection time for non-policy paths.
- Consider documenting explicit precedence in user docs along with examples for Service backends using `appProtocol`.

Related

- #598: Provider-specific `ai.httpVersion` proposal. This PR supersedes that approach with a general backend policy.
- #617: Backend HTTP policy + heuristics. This PR complements it by enforcing ALPN clamp for `"1.1"` and adding observability fields.
