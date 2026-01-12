# Agentgateway LLM Functionality

This module builds functionality for handling LLM requests.
This includes support for multiple different types of requests (OpenAI completions, Embeddings, Anthropic messages, etc),
policy and manipulation of these, parsing, and in some cases conversion.

In order to facilitate maximum compatibility (across providers or across versions, as new fields are added),
we use a "passthrough" approach to parsing. Each message includes a final `rest` field that stores all unknown fields:
```rust
#[serde(flatten, default)]
pub rest: serde_json::Value
```
Only fields we specifically operate on (like `model`) need to be included in the type definitions.

However, in some cases having the full typed definitions is useful, such as for conversion from one type to another.
In these, we have additional `typed` variation that we upgrade the passhthrough type to internally.