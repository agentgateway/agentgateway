# agent-llm-ir

> [!TIP]
> This crate is experimental and still a work in progress. Some areas are incomplete or still being refined;
> see [Limits and open questions](#limits-and-open-questions).

An IR for LLM requests and replies, with adapters that translate provider wire protocols to and from it. A
client that speaks one protocol can be served by a provider that speaks another.

The crate does no I/O and needs no async runtime: it is synchronous state machines over bytes. It suits
anything between an LLM client and a provider, such as a proxy or gateway, an agent harness targeting several
providers, or tooling that records, replays or inspects traffic.

## The problem

LLM APIs share the outline of a conversation and differ in the detail:

- **Structure.** Where system instructions live, how a tool call pairs with its result, whether tool names are
  flat or namespaced, which turn orders are legal.
- **Settings.** Reasoning is a token budget in one protocol and an effort level in another. A stop sequence or
  output schema may have no equivalent.
- **Reply streams.** Each protocol has its own event grammar, its own rule for when a tool call is complete,
  and its own way to report usage, truncation and failure mid-stream.
- **State across turns.** Some providers return opaque reasoning state that must come back on the next
  request, in a field the client's protocol may lack.

Converting each pair directly needs a converter between every two protocols, for requests and for replies,
each deciding for itself what to do with a field the target cannot express.

## The approach

Each protocol is one adapter with four parts. Adapters meet only at the IR, so a new adapter works with every
existing one.

| Part     | Direction             | Responsibility                                          |
| -------- | --------------------- | ------------------------------------------------------- |
| Parser   | client request → IR   | Validate the request and build a `Conversation`         |
| Renderer | IR → provider request | Write the provider request and a `ReplyPlan` for later  |
| Decoder  | provider reply → IR   | Turn a provider body or stream into `Event`s            |
| Writer   | IR → client reply     | Write the client's body or stream from `Event`s         |

Supported protocols are listed under [Protocols](#protocols).

## The IR

**Requests** are a [`Conversation`](src/ir/conversation.rs):

- `Settings`: output limit, sampling parameters, stop sequences, tool choice, reasoning effort and output
  format. A value a target may refuse stays as the client wrote it until the renderer decides.
- `Tool`: a name, an optional namespace, a JSON schema, and a kind (function, free-text, or client-side tool
  search).
- `Message`s of ordered `Block`s, whose `Content` is text, an image, a tool call, a tool result, or reasoning
  state to replay. Each block has a stable `BlockId`, so edits and replies can refer to it.

**Replies** are a sequence of [`Event`](src/ir/events.rs)s:

- `ResponseStart`, then per part `PartStart`, `Delta`s, and `PartEnd` with the final value. A part is text, a
  refusal, reasoning, or a tool call.
- `Usage` as the provider reported it; unknown is distinct from zero.
- `Finish` (completed, or incomplete with a reason) or `Error`.

A JSON body and a stream decode to the same events.

## Design decisions

**Synchronous core; the caller waits.** Every step is a plain function call, so the caller can pause between
any two. Asynchronous work, such as a remote moderation or guardrail call, is a value the caller awaits with
its own runtime, timeouts and concurrency:

- Request: the caller reads the `Conversation`, waits for outside decisions, and makes any resulting edits
  before rendering.
- Reply: with output checks set, the driver withholds output and hands the caller what to check. It keeps
  accounting, and resumes on the verdict.

See [request edits](src/prompt/README.md#edits-and-outside-checks) and [held
replies](src/reply/README.md#replies-held-for-a-verdict).

**Replies are events, not documents.** Output is produced as input arrives and taken between chunks, so
backpressure reaches the provider. Usage and telemetry observe the provider's events before stop handling or
rewrites change them. See [event order](src/reply/README.md#event-and-accounting-order).

**Loss is explicit.** Every client field is rendered, omitted with a record for the caller, or refused. A
refusal lists every problem and is final; nothing is retried in a degraded form. Each adapter's README gives
the outcome per field.

**A failed reply never looks complete.** Decoders reject unknown, repeated or out-of-order events. A failure
before output is an HTTP error; after output, an error in the client's protocol with no successful final
event.

**Middleware targets the IR.** An edit changes the `Conversation`, and a reply rewrite or output check works
on reply events, whatever the protocols at either end.

**Same-protocol requests pass through.** Converting them would only risk dropping fields the provider
understands, so the pipeline refuses to. The [passthrough](src/prompt/README.md) path leaves the bytes to the
caller and reads the request's model and settings, and the reply's usage and telemetry.

**Provider state rides with the client, scoped to its destination.** Reasoning state needed on a later turn
goes in a field the client already returns, encoded with its backend, provider, model and field; a mismatch is
refused. The carrier is client input: it is neither encrypted nor signed, so the check guards against
misrouting, not tampering. See [`carrier`](src/carrier/mod.rs).

**Few copies, bounded memory.** Conversations are long and resent every turn. Request text and JSON borrow
from the client's body until edited. Reply text is forwarded in the provider's JSON escaping, and a decoder
that checks streamed text against a final value keeps a length and hash, not the text. The body, an event, a
decoded part and held output each have a caller-set limit. See [size limits](src/reply/README.md#size-limits);
the [benchmarks](benches/README.md) profile allocations as well as time.

## Protocols

| Protocol         | Wire API                                                                       | Adapter documentation                                        |
| ---------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| Messages         | [Anthropic Messages](https://docs.anthropic.com/en/api/messages)               | [messages](src/protocols/messages/README.md)                 |
| Responses        | [OpenAI Responses](https://platform.openai.com/docs/api-reference/responses)   | [responses](src/protocols/responses/README.md)               |
| Chat Completions | [OpenAI Chat Completions](https://platform.openai.com/docs/api-reference/chat) | [chat_completions](src/protocols/chat_completions/README.md) |
| Gemini           | [Gemini `generateContent`](https://ai.google.dev/api/generate-content)         | [gemini](src/protocols/gemini/README.md)                     |

An adapter can serve as the client protocol, the provider protocol, or both. Its README records what it
accepts, omits and refuses against each peer.

## Further reading

- [Request pipeline](src/prompt/README.md): conversion, edits and checks, and passthrough.
- [Driving replies](src/reply/README.md): buffered and streaming drivers, held replies, failure rules and
  limits.
- [Validation](tests/README.md) and [benchmarks](benches/README.md): what is tested, fuzzed and measured, and
  what is not.

---

## Limits and open questions

Not supported today:

- **Content.** Images must be inline base64. Image URLs, file references, audio and provider-executed tools
  are refused.
- **One reply per request.** Multiple candidates are refused.
- **Generation only.** Token counting, model listing, embeddings and other endpoints are not converted.

Areas that need more thought/input/work:

- **Passthrough is read for its model and settings only.** A same-protocol request can't be read for checks or
  edited, so edits apply only to converted requests. Editing means writing the client's protocol back out
  without losing fields the IR does not model.
- **Conversations are stateless.** The client must send its full history; continuing from a response the
  provider stored is refused, over HTTP or a persistent connection. Should provider-held state be supported,
  and who would track it: the crate or its caller?
- **Some model capabilities are inferred from model names.** A caller can supply reasoning controls. Stop
  sequences, reasoning alongside tools, and cache breakpoints come from [model-name
  defaults](src/prompt/mod.rs), which go stale. Should every such fact come from a caller-held model catalog,
  and what applies without one?
- **Transport is one HTTP body or an SSE stream.** Another stream framing, such as AWS Bedrock's binary event
  stream, needs only a framing layer before a decoder. A persistent multi-turn connection, such as the
  Responses [WebSocket mode](https://developers.openai.com/api/docs/guides/websocket-mode), needs connection
  handling around otherwise unchanged requests.
