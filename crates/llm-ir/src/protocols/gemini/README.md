# Gemini

The adapter for Google's [Gemini `generateContent` API](https://ai.google.dev/api/generate-content). It reads
requests from Gemini clients such as Gemini CLI and writes their replies, and it renders requests for a Gemini
provider and decodes its replies. A Gemini client on a Gemini provider is a
[passthrough](../../prompt/README.md) and uses none of this.

## What shapes this adapter

Each entry gives the API's behaviour, then the choice made here.

**The URL names the model and the delivery.** `generateContent` is buffered and
`streamGenerateContent?alt=sse` is streaming. For a Gemini client, the caller passes
`ClientEndpoint::Gemini(delivery)` and maps the URL's model to `Upstream::scope`; a `model` in the body is
dropped and recorded. For a Gemini provider, the caller builds the URL from `Upstream.scope.model` and
`ConvertedRequest::delivery`.

**A subset of JSON Schema.** Gemini's tool schemas lack references and most keywords. As a provider, schemas
are rewritten to fit, and what can't be kept is dropped and recorded or refused. As a client, Gemini's schema
is converted to JSON Schema.

**Reasoning state on a content part.** Gemini clients keep a `thoughtSignature` as opaque base64 and return it
on the same part. A reply puts the carriers of the reasoning that precedes a part on that part's signature.
The part is the next call or text, never a thought, because clients drop thoughts from history; a reply that
ends with reasoning uses an empty text part. Each signature holds every carrier before its part, so reasoning
returns in order. Signatures are read in either base64 alphabet, with or without padding. A carrier opens only
for the backend, provider, model and field it names, so a conversation moved to another model is refused.

**Two spellings of every field.** Field names are read in camel or snake case, as Google's API does; the same
field given both ways is refused.

## As a client

| Feature | Gemini client → Messages or Responses upstream |
|---|---|
| Text, `systemInstruction` and model history | Converted; a content with no role is a user turn |
| Function declarations | Converted. Gemini's `parameters` schema becomes JSON Schema: types in lower case and `nullable` as a `null` type; `propertyOrdering` and `example` are dropped and recorded; other keywords are refused. `parametersJsonSchema` is forwarded as supplied; response schemas are dropped and recorded |
| Function calls and responses | Converted. A response belongs to the call with its `id`, or else the oldest open call of its name. `{"output": text}` and `{"content": text}` become the result's text, `{"error": text}` a failed result, and any other response its JSON text. Results go first in their turn, and images in a response go with it |
| `toolConfig.functionCallingConfig` | `AUTO`, `ANY`, `NONE` and `ANY` with one allowed name are converted; more than one allowed name and `VALIDATED` are refused |
| `temperature`, `topP`, `topK`, `stopSequences` | Converted as for the upstream's own clients |
| `maxOutputTokens` | Converted; without one, a Messages upstream uses the caller's default or refuses |
| `thinkingConfig` | A level becomes that effort and a budget of 0 turns thinking off. Other budgets become the effort Gemini 2 would get for them, and `-1` becomes `medium`; the budget is recorded as dropped. `includeThoughts` shows reasoning summaries |
| Structured output | `responseMimeType: application/json` with `responseSchema` or `responseJsonSchema` is converted; other types are refused |
| Images | Supported base64 `inlineData` in user turns is converted; other media, `fileData`, and images in model turns are refused |
| Reasoning replay | Only `thoughtSignature`s carrying state for this backend, provider, mapped model, and client field. Google's documented placeholder signatures are dropped and recorded; other signatures are refused. Thought text without a signature is dropped and recorded |
| `safetySettings`, `labels`, `mediaResolution` | Dropped and recorded |
| Google's tools (`googleSearch`, `codeExecution`, `urlContext` and others), `cachedContent`, code execution parts, `candidateCount` other than 1, non-text `responseModalities`, `seed` and penalties | Refused |
| Replies | `GeminiBuffered` writes one `GenerateContentResponse`, and `GeminiStream` one per event with `alt=sse`. Text streams as it arrives; a function call goes out whole, with the upstream's call ID. Thoughts are shown only when `includeThoughts` asked. Finish reasons are `STOP`, `MAX_TOKENS`, `SAFETY` and `OTHER`; a full context window is an error. A call cut off by the output limit is left out. Usage counts cached tokens in the prompt and thoughts apart from the answer; citations become `citationMetadata` sources |
| Errors | Google's error shape, `{"error": {"code", "message", "status"}}`, with fixed text. A failure before output is the HTTP error alone; after output, the stream ends with an error event. `refusal_reply` writes a refused request as a 400 |
| `countTokens`, model listing and other endpoints | Handled by the caller, which owns the tokenizer or count endpoint and the routing a model list describes |

**A known client limit.** Gemini CLI 0.67.0 records a streamed text part by joining later text into the first
chunk's part and keeping only that part's other fields. A signature on later text in the same turn, as when
the model reasons between two pieces of text, is lost, and the model then sees the turn without that
reasoning. Signatures on function calls are kept.

## As a provider

Thinking settings come from model-name defaults unless the caller supplies `RenderSettings::Gemini`.

| Feature | Messages client → Gemini upstream | Responses client → Gemini upstream |
|---|---|---|
| Text, system prompts and assistant history | Converted; leading system text becomes `systemInstruction`, and later system messages become `<system-reminder>` user text in their place | Converted; turns are joined, and later developer messages become `<system-reminder>` user text in their place |
| Function tools and `strict` | Converted; names must fit Gemini's 64-character set; `strict` is dropped and recorded | Converted; namespaced and custom tools are flattened and restored on replies; a custom grammar is described, not enforced, and recorded |
| Tool schemas | Rewritten to the subset Gemini takes: `$ref`s inlined, `allOf` merged; other keywords, such as `additionalProperties`, are dropped and recorded; `not` and `if` are refused | Same |
| Tool calls and results | Converted; results go first in their own turn, in call order; failed results get a text marker | Converted; failed results get a text marker |
| `tool_choice` | `auto`, `any`, `none` and a named tool are converted; `disable_parallel_tool_use` is refused | Converted; `parallel_tool_calls: false` is refused |
| `temperature`, `top_p`, `top_k` | Forwarded; the upstream validates model support | `temperature` and `top_p` forwarded |
| Stop sequences | Sent as `stopSequences` | No Responses request field |
| Reasoning effort | Gemini 2 gets a token budget and later models a level | Same |
| Turning thinking off | Gemini 2 Flash gets a budget of 0. Gemini 2 Pro and later models can't stop thinking: they get their smallest budget or level, and `thinking.type disabled` is recorded as dropped | The same, recorded as `reasoning.effort none` |
| Thinking budgets | `budget_tokens` is dropped and recorded; the model's default thinking applies | No Responses request field |
| Reasoning summaries | Thoughts are requested when thinking is enabled or adaptive and not omitted | Thoughts are requested when a summary is requested |
| Reasoning replay | Only carriers naming this backend, provider, mapped model, and client field. A signature Gemini sends after text that has already streamed is dropped and counted by `GeminiMessagesCompat`; call signatures are always kept | Only carriers naming this backend, provider, mapped model, and client field |
| Structured output | `output_config.format` becomes `responseSchema` with a JSON reply. With tools, Gemini can't enforce it: the schema is put in the system instruction, and enforcement is recorded as dropped | `text.format` JSON schema and `json_object` are converted the same way, with the same limit beside tools |
| Images, including tool results | Supported base64 images become `inlineData`; images in a tool result are nested in its `functionResponse`, which Gemini 2.5 refuses; URLs and file references are refused | Supported base64 data URLs are converted; URLs and files are refused |
| Prompt cache controls | Dropped and recorded | Input cache breakpoints are refused |
| Provider metadata | `metadata` and `context_management` are dropped and recorded | `prompt_cache_key`, `client_metadata` and `access_programs` are dropped and recorded |
| Server tools, unknown blocks, redacted or foreign thinking | Refused | Unsupported items or reasoning without a valid carrier are refused |
| Replies | Buffered and streaming, with usage accounting; thoughts count as output tokens. More than one candidate, code execution, server tool calls, inline media and unknown finish reasons fail the reply. A blocked prompt or output ends with Messages' `refusal` | Buffered and streaming, with usage accounting; the same parts fail; a blocked reply is incomplete with `content_filter` |

**Tool result JSON.** When a successful result has text blocks containing complete, strict JSON objects or
arrays, its `functionResponse.response.output` is an ordered array of those JSON values and the surrounding
text blocks. This keeps JSON data separate from client-added text such as Codex's timing header, so the model
does not have to copy escaped JSON out of prose. Other results retain their `content` text, including failed
results and text containing invalid JSON or JSON primitives. JSON is not extracted from within prose, and
returned tool arguments are never unescaped or repaired.
