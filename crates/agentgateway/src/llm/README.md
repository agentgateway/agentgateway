# Agentgateway LLM Functionality

This module builds functionality for handling LLM requests.
This includes support for multiple different types of requests (OpenAI completions, Embeddings, Anthropic messages, etc),
policy and manipulation of these, parsing, and in some cases conversion.

## GitHub Copilot Claude Routes

Copilot models whose IDs start with `claude-` use Anthropic Messages as their native upstream format.

| Client route | Copilot route | Handling |
| --- | --- | --- |
| `/v1/messages` | `/v1/messages` | Anthropic Messages passthrough, with Copilot-specific compatibility handling (see below) |
| `/v1/responses` | `/v1/messages` | Provider-neutral Responses-to-Messages conversion |
| `/v1/chat/completions` | `/v1/messages` | Chat-to-Messages conversion |

Responses-to-Messages routing follows the selected provider and model's advertised Messages
capability. Anthropic, Vertex Claude, Azure Foundry Claude, custom Messages providers, and Copilot
Claude all use the same conversion path. Providers that advertise native Responses or another wire
format keep that route, including OpenAI Responses, Bedrock Converse, and Vertex Gemini. The
converter carries per-request response translation state through `ProviderState`; it does not use
Copilot-specific context fields.

Copilot exposes a narrower Anthropic Messages dialect than native Anthropic: it rejects the
top-level `context_management` field and some `anthropic-beta` header entries (confirmed:
`advisor-tool-2026-03-01`) that a real Claude Code client sends. For Copilot only, the gateway
removes `context_management` before rendering the upstream body and filters confirmed-unsupported
`anthropic-beta` entries while preserving every other entry. These rules live in the Copilot
provider policy rather than the format converter. Every other provider (Anthropic,
Vertex, Bedrock, Azure, custom) forwards these fields unchanged. Verified Claude Code compatibility
covers streaming text, the built-in Read tool, a two-turn session, prompt caching, MCP, a custom
subagent, and the parent continuation after that subagent returned. All five configured Claude
aliases also passed short Responses, Chat Completions, and Messages probes. Background and parallel
subagents, context compaction near the limit, and long-running sessions have not been tested.

The Responses converter is stateless and requires `store: false`. Streaming requests may omit
`stream_options`. When present, `include_obfuscation` must be `false`. The converter does not add
OpenAI stream-obfuscation padding. Caller metadata and cache hints (`client_metadata`,
`metadata`, `prompt_cache_key`, and `prompt_cache_retention`) are discarded because Messages has no
equivalent and they do not change the converted request. It adds no provider configuration and covers
the supported overlap between Responses and Messages, including streaming, tools, media, refusals, and usage.
Text is forwarded incrementally. Anthropic reports refusal status only at the end of a stream, so
a refusal reported after text has already been emitted terminates with `refusal_after_streaming`
instead of retyping the earlier `output_text` events or emitting `response.completed`.

Copilot may emit adaptive-thinking blocks by default for some Claude models. Because reasoning content and
history are not representable through this bridge, the converter validates those blocks and omits them from
buffered and streaming Responses output. Malformed thinking blocks still return a conversion error.

Codex and Copilot's automatic reasoning-summary and encrypted-reasoning hints are discarded because this bridge
does not return reasoning content. Reasoning efforts from `low` through `max` request Anthropic adaptive thinking
with the matching effort, while `none` requests no thinking. Unsupported effort and summary values still fail
instead of losing requested behavior. The provider-neutral converter rejects hosted `web_search` because Messages
cannot preserve either live or cache-only search. For Copilot Claude Responses only, the Copilot request policy
removes Codex's default `{"type":"web_search","external_web_access":false}` declaration when tool choice is absent
or automatic. Live search, ambiguous declarations, malformed values, and explicitly selected hosted search still
fail instead of losing requested behavior. Shell and patch tools run through fixed local schemas. Other unsupported
Responses features return a conversion error instead of losing data during translation.

Every Responses field the converter still rejects was classified against one test: whether dropping it could
change execution, data handling, or security. None can be dropped safely. Each known field below returns its
own error rather than sharing one message; an unrecognised field reports only that an unsupported field was
present, because its name comes from the caller.

| Rejected field | Why it cannot be ignored |
| --- | --- |
| `store` other than `false` | The converter holds no server-side state, so accepting it would promise a retrievable response that does not exist. |
| `previous_response_id`, `conversation`, `prompt` | Name server-side state this bridge does not hold. |
| `background` | Requests deferred execution the bridge does not provide. |
| `logprobs`, `top_logprobs` | Request output Messages does not return. |
| `max_tool_calls` | Bounds tool calling that the model would otherwise exceed. |
| `service_tier` | Selects an upstream tier the converted request cannot carry. |
| `truncation` other than `disabled` | Server-side history trimming changes what reaches the model. |
| `text.verbosity` | Changes the generated output. |
| `include` beyond `reasoning.encrypted_content` | Requests output items the bridge cannot produce. |
| `stream_options` beyond `include_obfuscation: false` | The converter emits no obfuscation padding. |
| hosted `web_search` | Generic conversion rejects it because removing a requested tool could change execution. Copilot policy has one documented cache-only automatic exception. |
| `vendor_extensions` and unknown effectful fields | Unknown semantics could change execution. |

In order to facilitate maximum compatibility (across providers or across versions, as new fields are added),
we use a "passthrough" approach to parsing. Each message includes a final `rest` field that stores all unknown fields:
```rust
#[serde(flatten, default)]
pub rest: serde_json::Value
```
Only fields we specifically operate on (like `model`) need to be included in the type definitions.

However, in some cases having the full typed definitions is useful, such as for conversion from one type to another.
In these, we have additional `typed` variation that we upgrade the passthrough type to internally.
