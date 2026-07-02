# Fuzzing Report

This repo has many parsers, but the fuzzing budget should go where arbitrary input reaches local code. The best first pass is to avoid trusted xDS shape fuzzing and focus on downstream wire bytes, downstream LLM JSON bodies, and user-supplied expression languages.

## Top Candidates

| Rank | Candidate | Trust level | ROI | Notes |
| --- | --- | --- | --- | --- |
| 1 | PROXY protocol detection in `crates/agentgateway/src/proxy/proxy_protocol.rs` | Untrusted downstream wire bytes | High | Small harness, custom read/limit state machine, identity TLV parsing, and socket metadata feeding policy context. Good exploit class: panic, overread, unbounded buffering, or identity confusion before HTTP handling. |
| 2 | CEL expression compile/eval in `crates/agentgateway/src/cel/` and `crates/cel-fork/cel/` | Medium-trust config data | Medium-high | Expressions are user configuration, not arbitrary internet traffic, but they are a custom parser, optimizer, function dispatch layer, and dynamic executor over request/response/auth/LLM/MCP data. Good bug class: parser panics, stack blowups, optimizer crashes, function-call dispatch bugs, and JSON materialization failures. |
| 3 | AWS EventStream decode in `crates/agentgateway/src/parse/aws_sse.rs` | External upstream response bytes | Medium | Binary framing plus CRC/size validation is fuzz-friendly. Lower priority than request paths because the bytes come from configured model providers, but malicious or compromised upstreams make it worth a later harness. |
| 4 | LLM request conversion in `crates/agentgateway/src/llm/conversion/` | Untrusted HTTP request JSON | Medium-low | Users can send deeply nested or unusual OpenAI, Anthropic, and Responses bodies, but much of the first-hop surface is serde. The realistic bug class is panic, CPU/memory amplification, or semantic field loss during provider-specific normalization, not memory corruption. |
| 5 | MCP streamable HTTP JSON-RPC deserialization | Untrusted HTTP request JSON | Medium-low | The endpoint is important, but first-hop JSON-RPC parsing is mostly in `rmcp`. Higher-value MCP fuzzing would need a stateful relay harness with fake backends, which is more expensive than the likely first bugs justify. |
| 6 | xDS/protobuf translation, route regex compilation, and local YAML config | Trusted to medium-trust control/config data | Low first-pass ROI | Useful for robustness, but less exploit-oriented and more likely to spend cycles rediscovering invalid config handling. Prefer targeted unit tests for known translation bugs unless a specific crash appears. |

## Implemented Targets

Run the smoke checks with:

```sh
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly fuzz check proxy_protocol
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly fuzz check llm_request_conversions
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly fuzz check cel_expression
```

Run short campaigns with:

```sh
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly fuzz run proxy_protocol
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly fuzz run llm_request_conversions
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly fuzz run cel_expression
```

The LLM target enables the `agentgateway/fuzzing` feature so it can reach conversion internals without exposing them in normal builds.

For local smoke runs in sandboxed environments, LeakSanitizer can fail after the corpus completes. If that happens, rerun with `ASAN_OPTIONS=detect_odr_violation=0:detect_leaks=0`.
