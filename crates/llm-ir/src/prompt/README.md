# Request pipeline

A client parser builds a `Conversation`, the caller may edit it, and the target's renderer writes the provider
request. This module is the entry point for that path, and for reading a request that is not converted at all.

## Converting a request

| Entry point | Use |
|---|---|
| `prepare` | Parse into a `PreparedRequest` to inspect, edit and render. |
| `prepare_parsed` | As `prepare`, from a `serde_json::Value` the caller already has. |

```rust
# #![allow(dead_code)]
use agent_llm_ir::{
    ClientProtocol, ConvertedRequest, IdentifierPseudonymizer, RenderSettings,
    RequestRefusal, StateScope, Upstream, UpstreamProtocol, prepare,
};

fn messages_to_responses(
    body: &[u8],
    identifier_pseudonymizer: &dyn IdentifierPseudonymizer,
) -> Result<ConvertedRequest, RequestRefusal> {
    prepare(
        body,
        ClientProtocol::Messages,
        Upstream {
            identifier_pseudonymizer,
            scope: StateScope {
                backend: "backend-1",
                provider: "openai",
                // Use the model name that will be sent to the provider.
                model: "gpt-5",
            },
            protocol: UpstreamProtocol::Responses {
                safety_identifier: true,
                reasoning_context: false,
            },
            default_max_tokens: None,
        },
    )?
    .render(RenderSettings::ModelDefaults)
}
```

`Upstream` holds what the caller already knows about the destination:

| Field | Meaning |
|---|---|
| `protocol` | The provider protocol for this attempt, with its endpoint capabilities. |
| `scope` | The backend, provider and mapped model that replay state is scoped to. |
| `default_max_tokens` | Default output limit for Responses, Chat Completions and Gemini clients that omit one. |
| `identifier_pseudonymizer` | Shortens long client identifiers to fit provider limits. |

A `PreparedRequest` borrows the request input and the upstream, and may be held across an `await` while both
stay alive. The upstream cannot be replaced after parsing.

Raw bytes and a parsed `Value` go through the same validation and renderers. They differ in two ways: raw
input keeps the client's JSON encoding where parsed input serializes the supplied values, and a `Value` keeps
one value per key, so duplicate keys must be rejected by whoever parsed it.

## Edits and outside checks

`PreparedRequest::conversation_mut` is the one way to edit a parsed request. `Conversation::texts` reads every
text with its place, and `Conversation::edit_texts` replaces them; other fields are edited directly. `render`
validates the edited request.

A decision that needs outside work, such as a remote guard, is the caller's. It reads
`PreparedRequest::conversation`, waits with its own timeout and cancellation rules, then makes any resulting
edit before rendering.

## Model facts at render time

Renderers need to know what the selected model accepts. `render` uses [model-name defaults](mod.rs); a caller
that knows more passes it in.

`RenderSettings` carries reasoning and caching choices:

| Variant | Effect |
|---|---|
| `ModelDefaults` | Model-name defaults throughout. |
| `ChatCompletions`, `Responses` | `ReasoningControls`: whether the model reasons and which effort levels it accepts. |
| `Gemini` | Thinking levels or budgets the model accepts. |
| `Messages` | Prompt cache placement in place of the renderer's own. |

With `ReasoningControls`, the request's effort, after edits, maps to the nearest accepted level, the higher on
a tie. `supported: Some(false)` omits effort and records the omission. Knowing that a model reasons does not
establish which levels it accepts, so absent levels fall back to defaults.

Model-name defaults also decide these facts:

| Protocol | Fact | Controls |
|---|---|---|
| Chat Completions | `native_stop_sequences` | Whether `stop` is sent or emulated on the reply. |
| Chat Completions | `reasoning_with_tools` | Whether effort is sent beside tools, and the record when it is not. |
| Responses | `cache_breakpoints` | Whether client cache breakpoints are sent or dropped. |

If reasoning beside tools requires `none` and the accepted levels exclude it, effort is omitted and recorded.

## Passthrough

The pipeline refuses a request whose client and provider speak the same protocol. The caller sends such a
request on unconverted, so only the provider decides what it accepts, and reads it with these:

| API | Reads |
|---|---|
| `leading_model` | Only the model, when it precedes the conversation, so a destination can be chosen without scanning the body. |
| `RequestFields` | The top level once, skipping the conversation: the model, then settings such as the output limit and whether the reply streams. |
| `ParsedRequest` | The same from a `Value` the caller already parsed. |
| `ObservedReply` | Usage and telemetry from the reply, while the caller forwards its bytes. See [passthrough replies](../reply/README.md#passthrough-replies). |

A repeated `model` is refused, so the model a caller routes on is the one the provider reads.

## What conversion returns

| `ConvertedRequest` field | Caller's use |
|---|---|
| `body` | The provider request body. |
| `delivery` | Buffered or streaming transport. |
| `anthropic_beta` | A Messages beta header to add, when present. |
| `dropped` | Fields omitted on purpose, to record. |
| `reply` | The `ReplyPlan` for [driving the reply](../reply/README.md#reply-context). |

`ReplyPlan` carries what the reply side must remember: `routes` to restore flattened tool names and call
kinds, `show_reasoning`, `include_usage`, and `emulated_stop_sequences` for sequences the provider cannot
enforce.

Unsupported input is a `RequestRefusal` listing every problem found while parsing and rendering.
