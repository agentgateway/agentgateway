# agentgateway LLM Support

This module handles LLM requests, including Anthropic Messages, OpenAI completions and embeddings,
and applies policy, parses replies and converts between API formats when needed.

## Responses to Anthropic Messages

The shared Responses-to-Messages converter accepts OpenAI Responses requests for providers that
support Anthropic Messages, including Anthropic, Azure Foundry Claude, Copilot Claude, custom
Messages providers and Vertex Claude. Providers with native Responses or Converse routes use
those routes.

Requests are parsed into the typed Responses model and converted to Messages, with `ProviderState`
holding the conversion state for each request.

Function tools keep their names, descriptions and parameter schemas. Custom tools keep their
identity and carry free-form input in a `content` string schema. For function namespaces,
`NamespaceToolMap` rewrites declarations, tool choices and history, then restores names and
namespaces in buffered replies and stream events. Namespaced custom tools and the built-ins
`apply_patch`, `local_shell` and `shell` are unsupported.

Responses cache-breakpoint markers become ephemeral Messages cache controls on supported
documents, images, text and tool results, including system and developer text.
Calls and results keep their IDs across follow-up requests.

Nonempty Responses compaction controls and unsupported opaque history return explicit errors,
as does `prompt_cache_options`, including requests to prewarm the cache without generating output.
In-progress function results and unfinished function calls are rejected before replay.
When a response contains citations that Responses cannot represent, conversion returns a
response error or a safe error event for a stream.

Buffered replies and stream events use the standard Responses types and report the model
returned by Messages. Usage includes cache, cache-write and reasoning tokens when the provider
supplies them. Each supplied terminal usage counter replaces its initial value, while any
missing counter retains the value from the start of the stream. Cached input is counted once.

Thinking blocks, including unsigned blocks, are discarded from Responses output, and unsigned
thinking does not get a signature for replay to Anthropic.

Text streams as it arrives. When the upstream stop reason is `refusal`, buffered and streamed
replies keep `output_text` but return `status: "failed"` with error code `content_filter`, and
the stream ends with `response.failed`. This follows the Bedrock and Chat Completions adapters.

Copilot Claude requests go to `/v1/messages`, where the provider policy sets the Anthropic version,
filters beta features known to be unsupported and passes through native Messages `context_management`.
To use native context editing, send the `context-management-2025-06-27` beta header along with
the Messages `context_management` field. Custom hosts and `pathPrefix` settings follow the
provider's normal routing rules.

## Parsing and Conversion

Passthrough types store unknown fields in `rest`. This lets the gateway accept provider
extensions and fields added in later API versions without defining each one.

```rust
#[serde(flatten, default)]
pub rest: serde_json::Value
```

Fields the gateway reads or changes, such as `model`, have explicit definitions. Conversions
use additional `typed` variants when they need the full schema.
