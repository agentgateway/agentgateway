# LLM IR in the gateway

> [!TIP]
> This path is experimental and off by default. The existing converters remain the default and the fallback.

This module serves LLM requests with the [`agent-llm-ir`](../../../../llm-ir/README.md) crate. That crate
converts between protocols and does no I/O; this module is its caller. It decides which requests use the IR,
chooses the provider's protocol, runs policies around the conversion, and drives the reply.

## Enabling

| Setting | Default | Effect |
|---|---|---|
| `AGENTGATEWAY_LLM_IR_ENABLED` | `false` | `true` serves eligible requests with the IR. Any value other than `true` or `false` fails startup. |

With the flag on, the model router also changes for every LLM request: it reads only the body's `model`
instead of parsing the whole body, and refuses a body that names the model twice.

## Which requests use the IR

Chat-style routes (Messages, Responses, Chat Completions and Gemini `generateContent`) are candidates; other
routes keep their own handling. A candidate stays on the existing converters when it needs something the IR
path does not do yet, such as a provider or policy that isn't wired to it. Those cases are a fixed set,
documented on [`Ineligible`](selection.rs), and each is recorded as the request's legacy reason.

The decision is made once per attempt: first from the configuration, before the body is read, then from the
model. After that it is final. A request the IR refuses is returned to the client as a 400 listing every
problem, and is not retried on the existing converters.

The configuration check names every field of the provider and policy types, so a new field fails to compile
until someone decides how the IR treats it.

## Choosing the provider's protocol

Among the protocols the provider serves for the model, the gateway prefers native Gemini, then the client's
own protocol, then Messages, Responses and Chat Completions. The order and the reasons for it are in
[`selection.rs`](selection.rs).

## Policies

Existing LLM policies apply at the same points as on the existing converters: JSON policies before the IR
reads the request, model aliases and overrides before the destination is chosen, conversation policies such as
prompt enrichment and regex guards on the parsed conversation, and final transformations on the rendered body.
Token rate limits are settled from the reply's usage.

Model facts come from the model catalog where it has them, and otherwise from the IR crate's model-name
defaults.

## Passthrough

When the provider serves the client's own protocol, nothing is converted. The client's bytes go on with only
the edits the existing converters also make, such as the served model and settings needed to count usage.
Other fields stay as the client wrote them. The reply is forwarded unchanged and read only for usage.

## Replies

[`reply`](reply.rs) drives the IR's buffered and streaming reply drivers. Each provider chunk is decoded and
its output sent before the next chunk is read, so backpressure reaches the provider. Usage is recorded as it
arrives and settled when the stream ends or the client disconnects. A failure before any output becomes an
HTTP error in the client's protocol; after output, it ends the stream with that protocol's error.

Provider reasoning state that must return on a later turn rides in a field the client already sends back,
scoped to the backend, provider and model. See the crate's [design
decisions](../../../../llm-ir/README.md#design-decisions).

## Telemetry

| Attribute | Values |
|---|---|
| `agw.ai.converter` | `llm_ir`, `llm_ir_passthrough`, `native` or `legacy`. Set only while the IR is enabled. |
| `agw.ai.legacy_reason` | Why a request stayed on the existing converters, when the converter is `legacy`. |

## Outstanding work

- **Providers.** Bedrock and Vertex need their endpoints, signing and model families wired to the IR.
- **Policies that call out.** Prompt guards that call a service hold the request or reply for an outside
  verdict, which the gateway's IR stages don't do yet.
- **Request-side token counting** for rate-limit admission.
- **Content capture** of completions and tool calls for logging and CEL.
- **Editing a passthrough.** A same-protocol request isn't parsed, so prompt guards and Messages prompt
  caching can't apply to it.
- **Model facts.** Some capabilities still come from model names rather than the catalog.
