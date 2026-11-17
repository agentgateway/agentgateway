# Fuzzing

Requires nightly Rust and `cargo-fuzz`.

```sh
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly fuzz run proxy_protocol --features agw
cargo +nightly fuzz run llm_request_conversions
cargo +nightly fuzz run cel_expression
```

## LLM IR

The `llm_ir_` targets depend only on `agent-llm-ir`. Their checked-in corpora contain small request and reply
examples; no seed-generation step is needed. CI neither compiles nor runs these targets. From the repository
root:

```sh
make fuzz-check                    # the targets still build
make fuzz                          # both targets, 60 seconds each
make fuzz FUZZ_SECONDS=600 FUZZ_TARGETS=llm_ir_reply
```

| Target | Checks |
|---|---|
| `llm_ir_request` | Converting any body from any client never panics, a converted body is JSON, the one-pass and two-pass parsers agree, and the model pre-read finds the model a full JSON read finds. |
| `llm_ir_reply` | Buffered and streamed provider input across all 12 converted pairs never panics. Client output is JSON, or complete SSE events ending in success or an error; an early HTTP failure sends no stream bytes. Each input also runs with output checks, releasing or stopping the reply. |

Fuzzing finds crashes and broken wire output. It does not show that converted content or token counts are
correct; the normal tests do that. Keep each bug it finds as a reduced normal test.

Reply corpus files start with a control byte that sets the event-size limit and transport chunk size (`D`: no
limit, 18-byte chunks). The target consumes it; it is not sent to a provider. Generated inputs use
cargo-fuzz's hashed names and are ignored; the named starting inputs are tracked.

`llm_request_conversions` still covers the legacy and Bedrock conversion paths.
