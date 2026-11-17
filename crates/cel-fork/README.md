# cel-fork

This directory contains a vendored fork of [cel-rust](https://github.com/cel-rust/cel-rust)
(the `cel` and `cel-derive` crates), used by agentgateway to evaluate
[Common Expression Language (CEL)](https://github.com/google/cel-spec) expressions.

Upstream layout is preserved: as in cel-rust, `cel/README.md` is a symbolic link
to this file. When updating the fork from upstream, keep this file in place so
the link resolves.
