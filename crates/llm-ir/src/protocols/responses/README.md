# Responses

The adapter for OpenAI's [Responses API](https://platform.openai.com/docs/api-reference/responses). It reads
requests from Responses clients such as Codex and writes their replies, and it renders requests for a
Responses provider and decodes its replies. A Responses client on a Responses provider is a
[passthrough](../../prompt/README.md) and uses none of this.

## What shapes this adapter

Each entry gives the API's behaviour, then the choice made here.

**Server-side conversations.** Responses can store a conversation and continue it by `previous_response_id`.
The adapter is stateless: the client sends its whole history, and `store: true` and unknown fields,
`previous_response_id` among them, are refused.

**Opaque reasoning items.** A client returns each reasoning item's `encrypted_content` on its next turn. For a
provider of another protocol, that field carries the provider's reasoning state as a
[carrier](../../carrier/mod.rs) naming its backend, provider and model. Reasoning without a valid carrier for
the request's destination is refused.

**A final event that repeats the response.** A stream ends with an event that contains every output item
again. The decoder checks the stream against it, keeping a length and hash of each part's deltas instead of
the text. Because that event can exceed the event limit, it is the one event read in pieces: usage and outcome
are kept, the repeated text is not. Oversized events from other providers fail the reply.

**Tool kinds other protocols lack.** Responses has namespaced tools, custom tools that take free text, and a
tool search the client runs. The parser flattens them to function tools and records how each name read to the
client; the writer uses that record to restore the call the client expects.

## As a client

The writer sends nothing until the first output part exists, so an earlier failure is a plain HTTP error.
Calls cut short by a token limit are omitted from both buffered output and stream events. Codex can dispatch
`response.output_item.done` even when the item's status is `incomplete`, so that status alone cannot prevent
execution. The reply still ends as `response.incomplete` with the provider's usage; completed calls are sent
normally.

Against a Messages provider:

| Feature | Responses client → Messages upstream |
| --- | --- |
| Text, system prompts and assistant history | Converted; leading developer messages join the system prompt, and later ones become `<system-reminder>` user text in their place |
| Function tools and `strict` | Converted; namespaced names are flattened and restored on replies, and a namespace's description goes before each of its tools' own |
| Tool calls and results | Converted; custom and client-executed tool-search calls are restored on replies |
| Deferred tools | Client-executed tool search is converted |
| `temperature`, `top_p` | Forwarded; the upstream validates model support |
| `top_k` | No Responses request field; neutral edits can set it for Messages |
| Stop sequences | No Responses request field |
| Reasoning effort | `low` through `xhigh` become adaptive thinking; `none` and `minimal` are refused |
| Thinking budgets and effort `max` | Refused |
| Reasoning summaries | Summaries are shown when requested and thinking is enabled |
| Reasoning replay | Only carriers naming this backend, provider, mapped model, and client field |
| Structured output | `text.format` JSON schema is converted; name and strictness metadata are dropped and recorded; `json_object` is refused |
| Images, including tool results | Supported base64 data URLs are converted; URLs and files are refused |
| Prompt cache controls | Input cache breakpoints are refused; the renderer adds Messages cache marks, or a prompt caching policy chooses them |
| Provider metadata | `prompt_cache_key`, `client_metadata` and `access_programs` are dropped and recorded |
| Server tools, unknown blocks, redacted or foreign thinking | Unsupported items or reasoning without a valid carrier are refused |
| Replies | Buffered and streaming, with usage accounting and explicit failures |

Schemas are forwarded as supplied; `strict: true` asks the provider to enforce one, and nothing here rewrites
a schema to fit. Images must be PNG, JPEG, GIF or WebP.

Other providers are documented where the differences arise: [Gemini](../gemini/README.md#as-a-provider) and
[Chat Completions](../chat_completions/README.md#as-a-provider).

## As a provider

Endpoints that speak Responses do not all accept the same fields, so the caller states two capabilities on
`UpstreamProtocol::Responses`:

- `safety_identifier`: the endpoint accepts a client's user identifier.
- `reasoning_context`: the model accepts `reasoning.context`.

The caller can supply reasoning effort levels. Prompt cache breakpoint support comes from [model-name
defaults](../../prompt/mod.rs). See [model facts](../../prompt/README.md#model-facts-at-render-time).

The decoder is strict: an unknown, repeated or out-of-order event is an error.

What other clients can send is documented from the client's side: [Messages](../messages/README.md), [Chat
Completions](../chat_completions/README.md#as-a-client) and [Gemini](../gemini/README.md#as-a-client).
