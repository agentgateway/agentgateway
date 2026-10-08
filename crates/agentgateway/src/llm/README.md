# Agentgateway LLM Functionality

This module builds functionality for handling LLM requests.
This includes support for multiple different types of requests (OpenAI completions, Embeddings, Anthropic messages, etc),
policy and manipulation of these, parsing, and in some cases conversion.

## Responses to Anthropic Messages

Providers that advertise Anthropic Messages can accept OpenAI Responses requests through the
shared Responses-to-Messages converter. This includes Anthropic, Copilot Claude, Vertex Claude,
Azure Foundry Claude, and custom Messages providers. Providers with a native Responses or Converse
route keep their existing path.

The converter deserializes the public request into the typed Responses model, then maps the common
request surface into Anthropic Messages. Function tools keep their names, descriptions, and
parameter schemas. Responses custom tools keep their identity and expose free-form input through a
`content` string schema. Function namespace tools use the shared `NamespaceToolMap`, which
rewrites declarations, choices and history, then restores tool names and namespaces in buffered
and streamed replies. Namespaced custom tools and built-in tools such as `shell`, `local_shell`,
and `apply_patch` remain unsupported.

Responses cache-breakpoint markers become ephemeral Messages cache controls on supported text,
image, document and tool-result content, including system and developer text. Calls and results
keep their IDs across follow-up requests. Unsupported opaque history and nonempty Responses
compaction controls produce explicit errors. Responses `prompt_cache_options`, including cache
prewarming without generation, also remain unsupported.
In-progress function results and unfinished function calls are rejected before history is replayed.
Citation output that cannot be represented in Responses produces an explicit response error,
including a safe error event for streams.

Buffered and streaming translations return the standard Responses types. They report the upstream
Messages model and include cache, cache-write, and reasoning token usage when the provider sends
those fields. Conversion state is carried per request through `ProviderState`.

Copilot Claude requests use `/v1/messages`. Copilot's provider policy sets the Anthropic version,
filters beta features known to be unsupported, and preserves native Messages `context_management`.
Context editing requires the provider's `context-management-2025-06-27` beta header. Custom host
and `pathPrefix` behavior stays unchanged.

In order to facilitate maximum compatibility (across providers or across versions, as new fields are added),
we use a "passthrough" approach to parsing. Each message includes a final `rest` field that stores all unknown fields:
```rust
#[serde(flatten, default)]
pub rest: serde_json::Value
```
Only fields we specifically operate on (like `model`) need to be included in the type definitions.

However, in some cases having the full typed definitions is useful, such as for conversion from one type to another.
For these cases, we define additional `typed` variants and convert the passthrough types to them internally.
