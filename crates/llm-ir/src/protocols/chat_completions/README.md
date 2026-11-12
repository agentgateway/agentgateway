# Chat Completions

The adapter for OpenAI's [Chat Completions API](https://platform.openai.com/docs/api-reference/chat) and the
many endpoints compatible with it. It reads requests from Chat Completions clients and writes their replies,
and it renders requests for a Chat Completions provider and decodes its replies. A Chat Completions client on
a Chat Completions provider is a [passthrough](../../prompt/README.md) and uses none of this.

## What shapes this adapter

Each entry gives the API's behaviour, then the choice made here.

**No field for reasoning state.** A client has nowhere to return opaque reasoning. The writer carries it, with
its position in the visible text, in each tool call's ID, and the parser restores it on the next turn.

**Many compatible endpoints.** Providers that take this protocol differ in what they accept, so several rows
below depend on the model. A caller that knows the model supplies `RenderSettings::ChatCompletions` for
reasoning effort; native stop sequences and reasoning alongside tools use model-name defaults. See [model
facts](../../prompt/README.md#model-facts-at-render-time).

**A stream ends with `[DONE]`.** Usage comes in a last chunk, only when the client asks with
`stream_options.include_usage`. A failure after output ends the stream with an `error` event and no `[DONE]`,
so a client never takes a broken reply for a finished one.

## As a client

| Feature | Messages upstream | Responses upstream | Gemini upstream |
|---|---|---|---|
| Text, system and developer messages, assistant history | Converted; system and developer messages before the first turn join the system prompt, and later ones are placed as for a Responses client | The same | The same |
| Function tools and `strict` | Converted; a tool without `parameters` takes an empty object schema; custom tools are refused | The same | The same |
| Tool calls and results | Converted; every result must match an open call, and every call must have a result; a result's `name` must match its call and is dropped and recorded | The same | The same |
| `tool_choice`, `parallel_tool_calls` | Converted; allowed-tools choices are refused | The same | The same |
| `temperature`, `top_p`, `stop` | Forwarded; stop sequences are sent | Forwarded; stop sequences are emulated on reply text | Forwarded; stop sequences are sent |
| Output limit | `max_completion_tokens`, else `max_tokens`, else the default a caller supplies; with none, the request is refused | The same, but no limit is needed | The same, but no limit is needed |
| `reasoning_effort` | `low` through `xhigh`; `none` and `minimal` are refused | Converted | Converted to a thinking level or budget for the model |
| Reasoning text | Shown as `reasoning_content` when the client sets `reasoning_effort` other than `none`; reasoning text a client sends back is dropped and recorded | The same | The same |
| Reasoning replay | Reasoning and its visible-text position are carried in each call's ID and restored on the next turn; only carriers matching this backend, provider, mapped model, and client field are accepted | The same | The same; Gemini's signatures on text are dropped and recorded |
| Structured output | `json_schema` is converted, its name, description and strictness dropped and recorded; `json_object` is refused | Both are converted | Both are converted |
| Images | Base64 data URLs are converted, `detail` dropped and recorded; URLs, `file` and audio parts are refused | The same | The same |
| Streaming usage | A last chunk with usage, only with `stream_options.include_usage` | The same | The same |
| `n` other than 1, `logprobs`, `logit_bias`, penalties other than 0, `seed`, audio, predicted output, web search, `store: true`, the `flex` and `priority` tiers, `prompt_cache_breakpoint`, `reasoning_signature`, message `name`s, and unknown fields | Refused | Refused | Refused |
| Identifiers and metadata (`user`, `safety_identifier`, `prompt_cache_key`, `prompt_cache_retention`, `metadata`), `store: false`, the `auto` and `default` tiers, `verbosity`, `stream_options.include_obfuscation` | Dropped and recorded | Dropped and recorded | Dropped and recorded |
| Replies | Buffered `chat.completion` and streamed `chat.completion.chunk`, ending with `[DONE]`; each call is sent whole, numbered in order; a call cut off by the output limit is left out; citations, which only come from server tools a Chat Completions client can't ask for, are left out | The same | The same |

A failure before any output is an HTTP error in OpenAI's error shape.

## As a provider

| Feature | Messages client → Chat Completions upstream | Responses client → Chat Completions upstream |
|---|---|---|
| Text, system prompts and assistant history | Converted; system blocks and adjacent assistant texts are joined with a blank line | Converted; developer messages are sent as system messages |
| Function tools and `strict` | Converted; `strict` is sent when the client sets it | Converted; namespaced names are flattened and restored on replies; custom tools become functions of one string with the grammar in the description |
| Tool calls and results | Converted; failed results get a text marker; a result's images follow the results in a user message | Converted; custom calls are restored on replies |
| Deferred tools | A tool-reference search result becomes text naming the loaded tools | Client-executed tool search is converted |
| `temperature`, `top_p` | Forwarded; the upstream validates model support | Forwarded; the upstream validates model support |
| `top_k` | Refused | No Responses request field |
| Stop sequences | Sent as `stop`; emulated on reply text for models that refuse `stop` | No Responses request field |
| Reasoning effort | `output_config.effort` is sent as `reasoning_effort`, `max` as `xhigh`; omitted for models that refuse it, and `none` beside tools where the model can't reason with tools, with a record | `reasoning.effort` is sent as `reasoning_effort`, with the same model rules |
| Thinking budgets | Dropped and recorded; adaptive thinking without an effort sends none | No Responses request field |
| Reasoning summaries | Reasoning text comes back as thinking | `reasoning.summary` is dropped and recorded; reasoning text is shown only when summaries were requested |
| Reasoning replay | `reasoning_content` or `reasoning` in the field it came in, from carriers naming this backend, provider, mapped model, and client field | The same |
| Structured output | `output_config.format` JSON schema becomes a `json_schema` named `structured_output`; `output_format` is refused | `text.format` JSON schema likewise; `json_object` is sent as it is |
| Images, including tool results | Supported base64 images become data URLs; URLs and file references are refused | Supported base64 data URLs are converted; URLs and files are refused |
| Prompt cache controls | Cache marks are dropped and recorded | Input cache breakpoints are refused |
| Provider metadata | `metadata.user_id` becomes `user`, pseudonymized when longer than 64 bytes; other metadata is refused | `prompt_cache_key`, `client_metadata` and `access_programs` are dropped and recorded |
| Server tools, unknown blocks, redacted or foreign thinking | Refused | Unsupported items or reasoning without a valid carrier are refused |
| Replies | Buffered and streaming, waiting for `[DONE]`; usage from the final chunk; Mistral's `content` part lists can't be read and fail the reply | The same; a call cut off by the output limit is left out |

Output tokens are counted as the larger of `completion_tokens` and `total_tokens` less `prompt_tokens`,
because Gemini's OpenAI-compatible endpoint leaves reasoning out of `completion_tokens`.
