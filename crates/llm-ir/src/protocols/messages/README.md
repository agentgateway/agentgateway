# Messages

The adapter for Anthropic's [Messages API](https://docs.anthropic.com/en/api/messages). It reads requests from
Messages clients such as Claude Code and writes their replies, and it renders requests for a Messages provider
and decodes its replies. A Messages client on a Messages provider is a [passthrough](../../prompt/README.md)
and uses none of this.

## What shapes this adapter

Each entry gives the API's behaviour, then the choice made here.

**One open block at a time.** A Messages stream opens, fills and closes one content block before the next,
while other providers' parts can overlap. The writer's sequencer decides what is sent now, what is held and
what fails. Streamed and buffered replies are written from the same operations, and a tool call is sent only
when complete.

**No final values.** A Messages stream never repeats a block once streamed, so the decoder builds each final
value from the deltas. Any break in the expected event order is an error.

**Alternating roles.** Messages requires user and assistant turns to alternate, and takes system text only in
the top-level `system` field. The renderer joins adjacent messages of one role, and puts a system message that
comes after the conversation starts in the user turn at its place, as `<system-reminder>` text after that
turn's tool results.

**Signed thinking blocks.** A client returns each thinking block with its `signature`. For a provider of
another protocol, the signature carries the provider's reasoning state as a [carrier](../../carrier/mod.rs)
naming its backend, provider and model. Thinking without a valid carrier for the request's destination is
refused.

**A required output limit.** Messages requires `max_tokens`. A client of another protocol may omit it, so the
caller supplies a default through `Upstream::default_max_tokens`; without one the request is refused.

**Explicit cache breakpoints.** A Messages client marks where the prompt cache ends with `cache_control`. As a
provider, the renderer places marks on the last system block and the last block of the last user message, or
where the caller's `PromptCaching` setting puts them, after every stage has run.

## As a client

Against a Responses provider:

| Feature | Messages client → Responses upstream |
| --- | --- |
| Text, system prompts and assistant history | Converted |
| Function tools and `strict` | Converted; `strict` defaults to `false` |
| Tool calls and results | Converted; failed results get a text marker |
| Deferred tools | Loaded tools are included after a tool-reference search result |
| `temperature`, `top_p` | Forwarded; the upstream validates model support |
| `top_k` | Dropped and recorded |
| Stop sequences | Emulated on reply text while still reading upstream usage |
| Reasoning effort | Adaptive thinking with `low` through `xhigh`, or disabled thinking; neutral effort edits override client effort |
| Thinking budgets and effort `max` | Refused |
| Reasoning summaries | Summarized thinking display is refused; omitted display is supported |
| Reasoning replay | Only carriers naming this backend, provider, mapped model, and client field |
| Structured output | `output_config.format` JSON schema is converted |
| Images, including tool results | Supported base64 images are converted; URLs and file references are refused |
| Prompt cache controls | Supported Responses models receive cache marks; otherwise marks are dropped and recorded |
| Provider metadata | `metadata.user_id` becomes `safety_identifier` when supported, otherwise is dropped and recorded; other metadata is refused |
| Server tools, unknown blocks, redacted or foreign thinking | Refused |
| Replies | Buffered and streaming, with usage accounting and explicit failures |

Schemas are forwarded as supplied; `strict: true` asks the provider to enforce one, and nothing here rewrites
a schema to fit. Images must be PNG, JPEG, GIF or WebP.

Other providers are documented where the differences arise: [Gemini](../gemini/README.md#as-a-provider) and
[Chat Completions](../chat_completions/README.md#as-a-provider).

## As a provider

`ConvertedRequest::anthropic_beta` names an `anthropic-beta` header the caller must add when the rendered
request needs one.

What other clients can send is documented from the client's side:
[Responses](../responses/README.md#as-a-client), [Chat Completions](../chat_completions/README.md#as-a-client)
and [Gemini](../gemini/README.md#as-a-client).
