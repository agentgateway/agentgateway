# Fuzzing

Requires nightly Rust and `cargo-fuzz`.

```sh
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly fuzz run proxy_protocol --features agw
cargo +nightly fuzz run llm_request_conversions
cargo +nightly fuzz run cel_expression
```
