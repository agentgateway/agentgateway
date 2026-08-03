# Agentgateway LLM Functionality

This module builds functionality for handling LLM requests.
This includes support for multiple different types of requests (OpenAI completions, Embeddings, Anthropic messages, etc),
policy and manipulation of these, parsing, and in some cases conversion.

## GitHub Copilot Claude Routes

Copilot models whose IDs start with `claude-` use Anthropic Messages as their native upstream format.

| Client route | Copilot route | Handling |
| --- | --- | --- |
| `/v1/messages` | `/v1/messages` | Anthropic Messages passthrough, with Copilot-specific compatibility handling (see below) |
| `/v1/responses` | `/v1/messages` | Direct Responses-to-Messages conversion |
| `/v1/chat/completions` | `/v1/messages` | Chat-to-Messages conversion |

Copilot exposes a narrower Anthropic Messages dialect than native Anthropic: it rejects the
top-level `context_management` field and some `anthropic-beta` header entries (confirmed:
`advisor-tool-2026-03-01`) that a real Claude Code client sends. For Copilot only, the gateway
removes `context_management` before rendering the upstream body and filters confirmed-unsupported
`anthropic-beta` entries while preserving every other entry. Every other provider (Anthropic,
Vertex, Bedrock, Azure, custom) forwards these fields unchanged. Verified Claude Code compatibility
covers streaming text, the built-in Read tool, a two-turn session, prompt caching, MCP, a custom
subagent, and the parent continuation after that subagent returned. All five configured Claude
aliases also passed short Responses, Chat Completions, and Messages probes. Background and parallel
subagents, context compaction near the limit, and long-running sessions have not been tested.

The Responses converter is stateless and requires `store: false`. Streaming requests must also set
`stream_options.include_obfuscation: false`. Caller metadata and cache hints (`client_metadata`,
`metadata`, `prompt_cache_key`, and `prompt_cache_retention`) are discarded because Messages has no
equivalent and they do not change the converted request. It adds no provider configuration and covers
the supported overlap between Responses and Messages, including streaming, tools, media, refusals, and usage.
Text is forwarded incrementally. Anthropic reports refusal status only at the end of a stream, so
a refusal reported after text has already been emitted terminates with `refusal_after_streaming`
instead of retyping the earlier `output_text` events or emitting `response.completed`.

Copilot may emit adaptive-thinking blocks by default for some Claude models. Because reasoning content and
history are not representable through this bridge, the converter validates those blocks and omits them from
buffered and streaming Responses output. Malformed thinking blocks still return a conversion error.

Reasoning requests, reasoning history, encrypted content, and hosted execution are rejected. Shell and patch tools
run through fixed local schemas. Other unsupported Responses features return a conversion error instead of losing
data during translation.

In order to facilitate maximum compatibility (across providers or across versions, as new fields are added),
we use a "passthrough" approach to parsing. Each message includes a final `rest` field that stores all unknown fields:
```rust
#[serde(flatten, default)]
pub rest: serde_json::Value
```
Only fields we specifically operate on (like `model`) need to be included in the type definitions.

However, in some cases having the full typed definitions is useful, such as for conversion from one type to another.
In these, we have additional `typed` variation that we upgrade the passhthrough type to internally.
