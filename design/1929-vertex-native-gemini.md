# EP-1929: Native Gemini on Vertex

- Issue: [#1929](https://github.com/agentgateway/agentgateway/issues/1929)
- Related: N/A
- Status: proposed
- Date: 2026-05-28

> **Note:** This design reflects the proposal as of the date above. The current implementation may differ as the design
> is implemented, reviewed, or revised.

## Summary

Every Gemini request through the `Vertex` provider currently lands on Google's OpenAI-compatibility shim at `/v1/projects/{p}/locations/{l}/endpoints/openapi/chat/completions`. The shim costs us implicit prompt cache hits, adds a translation hop on Google's edge, and silently drops Gemini thought parts. That last one matters most: agentgateway already has `reasoning_content` plumbing for other providers at `llm/types/completions.rs:585`, and on the Vertex path it stays empty for Gemini.

This proposal adds a native code path on the `Vertex` provider that routes Gemini models to `:generateContent` / `:streamGenerateContent?alt=sse` and translates between OpenAI/Anthropic request and response shapes and Gemini's native ones. The path is opt-in behind one provider field, `geminiApi: native`. Existing deployments are unchanged.

## Background

Relevant code paths today:

- `vertex.rs:68` (`get_path_for_model`): returns the OpenAI-compat URL for any Vertex request that isn't `AnthropicTokenCount`, `Embeddings`, or Anthropic-on-Vertex. Gemini hits the catch-all at line 109.
- `llm/types/completions.rs:265` (`to_vertex`): returns raw OpenAI body passthrough for non-Anthropic models. `messages.rs:269` is the equivalent for Anthropic-shape input.
- `mod.rs:912-921`: the dispatch site. `AIProvider::Vertex(p)` forks only on `is_anthropic_model`; everything else hits compat.

The Anthropic-on-Vertex path is the template. It detects model family by name heuristic (`vertex.rs:127` `anthropic_model`), normalises the model identifier, translates the body, and routes to `:rawPredict` / `:streamRawPredict`. Gemini-on-Vertex wants the same structure with a different URL family and a different translator.

Worth reading before implementing: [#959](https://github.com/agentgateway/agentgateway/pull/959) (`anthropic_model` heuristic), [#1359](https://github.com/agentgateway/agentgateway/pull/1359) (Anthropic-on-Vertex stabilisation bugs, the Gemini path will hit equivalents), [#1465](https://github.com/agentgateway/agentgateway/pull/1465) (defensive-deserialisation patterns for missing fields).

`AIProvider::Gemini` (the direct Generative Language API) uses the same compat shim and has the same limitations. Out of scope here; the translator added is intended to be reusable.

## Goals

Operators who set `geminiApi: native` on a Gemini-on-Vertex backend get four things they don't get today: populated `reasoning_content` (and Anthropic `thinking` blocks) when they pass `reasoning_effort`; native `usageMetadata` token counts in `usage.*` and `gen_ai.usage.*` instead of the shim's reported numbers; implicit Vertex prompt cache hits across turns, surfacing as `usage.prompt_tokens_details.cached_tokens > 0`; and Gemini-only knobs reaching the model (`thinkingConfig`, `cachedContent`, `safetySettings`, `labels`, `responseSchema`).

Operators who don't set the field see no change. URL, request body, response shape, and token counts on the compat path stay identical to today.

Two new CEL/log fields, `llm.apiSurface` (`"openai_compat"` or `"native"`) and `llm.upstreamFinishReason` (raw Gemini value), are emitted on every Gemini-on-Vertex request so operators can tell which path served a request and alert on Gemini-specific finish reasons that collapse to the same OpenAI value.

All existing policies (rate limiting, prompt enrichment, prompt guards, guardrails including Google Model Armor, CEL telemetry, `LLMInfo`) work unchanged because they attach to the IR, which is upstream of the new translator.

## Non-Goals

Deferred to follow-up proposals:

- Native path on `AIProvider::Gemini`. Same translator with minor surface-drift adjustments (`labels`, Vertex-shaped `cachedContent`), but not in this PR.
- Native `:countTokens`. Pre-flight counts stay on the local-tokenizer fallback at `mod.rs:702`. Known cost: local BPE vs Gemini SentencePiece can disagree 10–30% on the same text.
- Other input formats. `InputFormat::Responses`, `Realtime`, `Embeddings` keep their current paths.
- Typed support for Gemini-only features (Live API, code execution, advanced safety). They round-trip via `rest` passthrough and `Part::Unknown`.
- Lifecycle management for explicit context caching. `cachedContent` is passthrough only; users create caches via the separate Vertex `cachedContents` API.

Compatibility work explicitly not in this PR:

- Flipping the default from `openaiCompat` to `native`. Future short proposal once the native path has community use.
- Removing the OpenAI-compat path. No committed timeline.
- Recalibrating token-based rate limits for opt-in users. Operator's responsibility; release notes call it out, and [#1759](https://github.com/agentgateway/agentgateway/pull/1759) (`tokenCosts` multipliers) is the recommended mechanism.

## API

One optional enum field on `vertex.Provider`, plumbed through local config, the xDS proto, and the CRD.

**Rust (`vertex.rs`):**

```rust
#[apply(schema!)]
#[non_exhaustive]
pub struct Provider {
    pub project_id: Strng,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Strng>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<Strng>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_api: Option<GeminiApiMode>,
}

#[apply(schema!)]
#[derive(Copy, PartialEq, Eq, Default)]
pub enum GeminiApiMode {
    #[default]
    OpenAiCompat,
    Native,
}
```

`#[non_exhaustive]` is added because `Provider` has a required `project_id` field, so `#[derive(Default)]` isn't a path, and every future field addition would otherwise be a hard compile break for downstream embedders constructing `Provider { project_id, model, region }`. See "Library API for embedders" under Compatibility.

**xDS (`crates/protos/proto/resource.proto`):**

```proto
message Vertex {
  optional string model = 1;
  string region = 2;
  string project_id = 3;
  GeminiApi gemini_api = 4;   // proto3 zero = OPENAI_COMPAT
}
enum GeminiApi { OPENAI_COMPAT = 0; NATIVE = 1; }
```

Proto3 zero handles cross-version compatibility: old control planes omit the field, new data planes see `OPENAI_COMPAT`, behaviour is unchanged.

**CRD (`controller/api/v1alpha1`):**

`VertexAIConfig` gains an optional `GeminiApi *VertexGeminiApi` field with `+kubebuilder:validation:Enum=openaiCompat;native` and `+kubebuilder:default=openaiCompat`. Admission rejects invalid values; absent CRDs apply cleanly.

**Example:**

```yaml
backends:
  - name: vertex-gemini
    ai:
      provider:
        vertex:
          projectId: my-project
          region: us-central1
          model: gemini-2.5-flash
          geminiApi: native
```

Setting `geminiApi: native` on a backend that resolves to a `claude-*` model or an embedding model is accepted by validation but is no-op at runtime; the routing heuristic ignores the flag for non-Gemini models.

The flag is provider-level, not per-request. Per-route already gives canarying, and a request-shape header would be more surface than the use case warrants.

## Runtime Design

### URL selection

`get_path_for_model` becomes a 3-tuple match with one new arm before the catch-all:

```rust
match (route, self.anthropic_model(req_model), self.gemini_native_model(req_model)) {
    (RouteType::AnthropicTokenCount, _, _) => /* anthropic count-tokens:rawPredict */,
    (RouteType::Embeddings, _, _)          => /* publishers/google/models/{m}:predict */,
    (_, Some(model), _)                    => /* anthropic rawPredict / streamRawPredict */,
    (_, None, Some(model))         /*NEW*/ => /* generateContent / streamGenerateContent?alt=sse */,
    _                                      => /* endpoints/openapi/chat/completions */,
}
```

`gemini_native_model` returns `Some(model)` only when `gemini_api == Native` and the model name matches the Gemini heuristic (mirrors `anthropic_model` at `vertex.rs:127-141`): strip `publishers/google/models/`, `models/`, `google/` prefixes; accept raw names starting with `gemini-` or `gemini@`. Exclude documented embedding prefixes (`text-embedding-`, `gemini-embedding-`, `text-multilingual-embedding-`, `textembedding-`, `multimodalembedding`) as defence-in-depth.

The `?alt=sse` query parameter is mandatory on the streaming endpoint. Without it Vertex returns a JSON array, not SSE, and the stream parser breaks. (Shipped bug in claude-code-router: [musistudio/claude-code-router#1315](https://github.com/musistudio/claude-code-router/issues/1315).) The path-with-query is passed through `set_path_and_query` at `mod.rs:475`, which already handles `?` correctly.

### OpenAI → Gemini

| OpenAI | Gemini |
|---|---|
| `messages[role=system]` | `systemInstruction.parts[].text` (concatenated if multiple) |
| `messages[role=user\|assistant]` | `contents[].{ role, parts }`; `assistant` becomes `model` |
| content-array `text` | `parts[].text` |
| content-array `image_url` (`data:`) | `parts[].inlineData { mimeType, data }` |
| content-array `image_url` (`gs://`) | `parts[].fileData { fileUri, mimeType }` |
| content-array `image_url` (`http://`, `https://`) | rejected with translator-level 400 |
| `messages[role=tool]` | `contents[role=user].parts[].functionResponse` (canonical v1 REST; not `role=function`) |
| `tool_calls[].id`, `tool_call_id` | `functionCall.id`, `functionResponse.id` (Gemini 3 only; dropped on 2.5) |
| `tools[].function` | `tools[].functionDeclarations[]` |
| `tool_choice` `auto`/`none`/`required`/`{type:function,...}` | `toolConfig.functionCallingConfig` `AUTO`/`NONE`/`ANY`/`ANY+allowedFunctionNames` |
| `temperature`, `top_p`, `frequency_penalty`, `presence_penalty` | `generationConfig.{temperature, topP, frequencyPenalty, presencePenalty}` |
| `rest.top_k` | `generationConfig.topK` |
| `max_completion_tokens` / `max_tokens` | `generationConfig.maxOutputTokens` |
| `stop`, `n`, `seed` | `generationConfig.{stopSequences, candidateCount, seed}` |
| `response_format` | `generationConfig.responseMimeType` and/or `responseSchema` (see below) |
| `reasoning_effort` or `rest.thinking_config` | `generationConfig.thinkingConfig` (see below) |
| `rest.cachedContent` | top-level `cachedContent` |
| `rest.safetySettings`, `rest.labels` | top-level passthrough (not nested in `generationConfig`) |

**Assistant message with both `content` and `tool_calls`** becomes one `contents[role=model]` entry whose `parts` is text parts followed by `functionCall` parts. Splitting into two `contents[]` entries violates Gemini's role-alternation requirement.

**Tool-result messages** become `role=user` with a `functionResponse` part. The translator must emit `role=user` (never `role=function`) but must accept `role=function` on inbound responses and normalise to `role=user`, since some SDKs use it.

**HTTP(S) image URLs are rejected.** Vertex's `fileData.fileUri` accepts `gs://` and a narrow set of Google-hosted URLs (mostly YouTube); arbitrary public HTTPS URLs produce 400s from `:generateContent`. Auto-fetching in-proxy would introduce implicit egress and unbounded memory cost the proxy doesn't gate elsewhere. Documented and rejectable; relaxable in a follow-up if there's demand.

**Structured outputs (`response_format`).** Three input shapes:

- `{type: "text"}` or absent: omit both Gemini fields.
- `{type: "json_object"}`: `responseMimeType: "application/json"`, no `responseSchema`.
- `{type: "json_schema", json_schema: {schema, strict, name, description}}`: `responseMimeType: "application/json"` plus `responseSchema: <schema>`. The translator unwraps the schema and drops `strict`/`name`/`description`; Gemini's enforcement is always strict when `responseSchema` is set.

### Anthropic → Gemini

Mirror the OpenAI translator from `llm::types::messages::Request`. Field mappings are 1:1: `system` becomes `systemInstruction`; `messages` becomes `contents` with the same role rewrite; `tool_use` and `tool_result` blocks become `functionCall` / `functionResponse` parts; `thinking` blocks become `parts[].thought = true`.

`tool_choice` (defined at `llm/types/messages.rs:969-990` as a `type`-tagged enum with `Auto` / `Any` / `Tool` / `None` variants, each carrying an optional `disable_parallel_tool_use`) maps as:

| Anthropic `tool_choice` | Gemini `toolConfig.functionCallingConfig` |
|---|---|
| `{type: "auto"}` | `mode: AUTO` |
| `{type: "any"}` | `mode: ANY` |
| `{type: "tool", name: "X"}` | `mode: ANY`, `allowedFunctionNames: ["X"]` |
| `{type: "none"}` | `mode: NONE` |

`disable_parallel_tool_use` has no Gemini equivalent and is dropped with a single warn-log per request. Gemini's serial-vs-parallel emission isn't operator-configurable; documenting the drop in release notes is sufficient.

### Thinking config

The translator does not auto-enable thinking. `thinkingConfig` is emitted only when the client signals it via OpenAI `reasoning_effort` or `rest.thinking_config`. Matches Anthropic-on-Vertex precedent and avoids surprising token bills on first switch.

For Gemini 3, `reasoning_effort` maps directly to `thinkingConfig.thinkingLevel` (`minimal`/`low`/`medium`/`high`).

For Gemini 2.5, it maps to `thinkingConfig.thinkingBudget` as an integer: `low → 1024`, `medium → 2048`, `high → 4096` (LiteLLM's published values for the shared range). `"minimal"` has no 2.5 analogue; coerced to `low`. `"none"` (community convention) omits `thinkingConfig` entirely rather than emitting `thinkingBudget: 0`, because Gemini 2.5 Pro's documented `thinkingBudget` range is 128–32,768 (default dynamic, `-1`) and `0` fails on Pro. Users wanting a specific budget pass `rest.thinking_config.thinking_budget`.

Release notes call this out: Gemini 2.5 Pro thinking cannot be fully disabled (minimum 128). `reasoning_effort: "none"` signals intent but doesn't zero the model's baseline budget.

`includeThoughts` defaults to true when thinking is enabled. Explicit `rest.thinking_config.include_thoughts: false` overrides.

### Gemini → OpenAI/Anthropic

| Gemini | OpenAI |
|---|---|
| `candidates[].content.parts[].text` (`thought = true`) | `choices[].message.reasoning_content` |
| `candidates[].content.parts[].text` (`thought` false or absent) | `choices[].message.content` |
| `candidates[].content.parts[].functionCall` | `choices[].message.tool_calls[]` |
| `usageMetadata.{promptTokenCount, candidatesTokenCount, totalTokenCount}` | `usage.{prompt_tokens, completion_tokens, total_tokens}` |
| `usageMetadata.cachedContentTokenCount` | `usage.prompt_tokens_details.cached_tokens` |
| `usageMetadata.thoughtsTokenCount` | `usage.completion_tokens_details.reasoning_tokens` |

`finishReason` mapping (full enum; unknown variants fall to `"stop"` and log):

| Gemini | OpenAI |
|---|---|
| `STOP` | `stop` (override: see below) |
| `MAX_TOKENS` | `length` |
| `SAFETY`, `RECITATION`, `LANGUAGE`, `BLOCKLIST`, `PROHIBITED_CONTENT`, `SPII`, `IMAGE_SAFETY`, `IMAGE_PROHIBITED_CONTENT`, `IMAGE_RECITATION`, `UNEXPECTED_TOOL_CALL`, `TOO_MANY_TOOL_CALLS` | `content_filter` |
| `MALFORMED_FUNCTION_CALL`, `IMAGE_OTHER`, `NO_IMAGE`, `OTHER`, `FINISH_REASON_UNSPECIFIED` | `stop` |

**Tool-call override.** Gemini emits `STOP` for both plain text completions and successful tool calls; only `parts[].functionCall` presence distinguishes them. After the table mapping, if the result is `"stop"` and the candidate contains at least one `functionCall`, override to `"tool_calls"` (Anthropic equivalent: `stop_reason: "tool_use"`). Without this, LangChain/AutoGen tool loops break. Same pattern as Bedrock's `StopReason::ToolUse → FinishReason::ToolCalls` at `conversion/bedrock.rs:874`.

The raw Gemini value is preserved in `llm.upstreamFinishReason` so operators can alert on `MALFORMED_FUNCTION_CALL` etc. without polluting wire responses with non-spec fields (an earlier draft used `finish_details`; rejected because strict OpenAI clients drop it).

For Anthropic-shaped clients, thought-text becomes `content[].type = "thinking"` blocks ahead of `text` blocks.

### Tool-call ID synthesis (Gemini 2.5)

Gemini 3 emits `functionCall.id` and `functionResponse.id`. Gemini 2.5 doesn't. The translator synthesises `call_{request_id}_{index_in_response}` on outbound and drops it on inbound (Gemini 2.5 matches by name and ordering). OpenAI conversations are stateless, so no cross-turn persistence is needed.

`args` deliberately doesn't participate. OpenAI SSE requires the `id` in the first delta chunk for that index, but `args` only arrive in later chunks; any args-dependent hash is non-computable at first-chunk time. Positional indexing alone disambiguates parallel calls with byte-identical args (a real agent-loop pattern: two `read_file({"path": "/tmp/a"})` in one turn).

Bedrock translator at `conversion/bedrock.rs:271-323` is the round-trip-pattern precedent.

### Streaming SSE

Each event is `data: <full GenerateContentResponse JSON>\n\n`. Per-chunk content is delta, not cumulative. Five things the streaming code has to get right:

1. **`delta.role` emitted exactly once** per choice index, on the first chunk. Carry a `role_emitted: bool` per index. Tool-call and reasoning_content deltas don't reset the flag. Matches Bedrock at `conversion/bedrock.rs:779`.
2. **`usageMetadata` is not on every chunk** and sometimes arrives in a trailing chunk with empty/absent `candidates`. The translator accumulates until upstream stream-end, not until `finishReason`. (Same family as [BerriAI/litellm#25389](https://github.com/BerriAI/litellm/issues/25389).)
3. **No `[DONE]` sentinel from Vertex.** Synthesise `data: [DONE]\n\n` on close for OpenAI clients; emit `message_stop` for Anthropic clients.
4. **Thought parts**: chunk-local partition into `delta.reasoning_content` vs `delta.content` for OpenAI; preserve Gemini's ordering.
5. **Anthropic-shape block ordering** is message-structured, not chunk-local. Carry per-stream state for current block kind and index. On first thought-text emit `content_block_start {type:"thinking"}`; on first non-thought text emit `content_block_stop` then `content_block_start {type:"text"}`. If a late `thought=true` chunk arrives after text has opened, route to OpenAI `reasoning_content` but drop from the Anthropic message stream and log warn; Anthropic's protocol can't represent a thinking block after a text block. Tool-call parts open a `tool_use` block at the next free index with its own lifecycle.

### Where it hooks in

`mod.rs:912-921`, one new branch:

```rust
AIProvider::Vertex(p) => {
    if p.is_anthropic_model(Some(request_model)) {
        let body = req.to_anthropic()?;
        p.prepare_anthropic_message_body(body)?
    } else if p.is_gemini_native_model(Some(request_model)) {
        req.to_vertex_gemini(p)?
    } else {
        req.to_vertex(p)?
    }
}
```

Five more sites in `mod.rs` currently fork on `is_anthropic_model` and need a parallel `is_gemini_native_model` branch. **Without these, the request goes out as native but the response is parsed as compat-shape and corrupts.** This is the highest-risk part of the PR; all five sites land in the same commit.

| Site | New branch |
|---|---|
| `mod.rs:1161-1170` (process_success, Messages) | Translate `GenerateContentResponse` to Anthropic shape via new `conversion/vertex_gemini::to_messages` |
| `mod.rs:1197-1206` (process_success, Completions) | Parse `GenerateContentResponse`, convert to `types::completions::Response` |
| `mod.rs:1234-1237` (streaming-dispatch precompute) | Compute `is_vertex_gemini_native` alongside `is_vertex_anthropic` |
| `mod.rs:1278/1297` (streaming dispatch) | Add `is_vertex_gemini_native` arms calling native streaming translators |
| `mod.rs:1405-1411`, `1423-1429` (error translation) | Existing `translate_google_error` is expected to work for native errors; covered by a fixture rather than assumed |

`count_tokens` is not in this list. Gemini native `:countTokens` stays out of scope; the existing fallback at `mod.rs:702` (`use_local`) keeps Gemini count-tokens on the local tokenizer regardless of `gemini_api`. A unit test asserts a native-mode count-tokens request hits the local fallback, not the Anthropic-count-tokens builder at `mod.rs:901-903`.

### Rust types

New DTOs in a new file `crates/agentgateway/src/llm/types/vertex_gemini.rs` (the existing `vertex.rs` is already taken by the embeddings `:predict` types — `PredictRequest`, `Prediction`, `EmbeddingsResult`, …): `GenerateContentRequest`, `GenerateContentResponse`, `Content`, `Part`, `FunctionCall`, `FunctionResponse`, `Tool`, `FunctionDeclaration`, `ToolConfig`, `GenerationConfig`, `ThinkingConfig`, `SafetySetting`, `UsageMetadata`. Each top-level type carries `#[serde(flatten)] rest: serde_json::Value` per the LLM-module README convention.

Translator entry points (additive methods, mirroring `to_anthropic` / `to_vertex`):

```rust
impl completions::Request { pub fn to_vertex_gemini(&self, provider: &vertex::Provider) -> Result<Vec<u8>, AIError>; }
impl messages::Request    { pub fn to_vertex_gemini(&self, provider: &vertex::Provider) -> Result<Vec<u8>, AIError>; }
```

The translation logic lives in a new `conversion/vertex_gemini/` module with `from_completions`, `from_messages`, `to_completions`, `to_messages` (mirrors `conversion/bedrock/`).

`Part` is the one type that needs care. It's a oneof with ~8 known shapes plus an orthogonal `thought` boolean. Use `#[serde(untagged)]`; serde discriminates by required-field presence, with variants declared most-common-first (`text`, `functionCall`, `functionResponse`, `inlineData`, `fileData`, code-execution) and `Unknown(serde_json::Value)` last as the round-trip-safe catch-all. Each variant struct carries `#[serde(flatten)] rest: serde_json::Value` so a new optional field Google adds to e.g. `TextPart` (a future `thoughtSignature` etc.) round-trips through `rest` instead of falling through to `Unknown` and turning the payload opaque. Do **not** use `#[serde(deny_unknown_fields)]` on the variants — that would defeat the drift tolerance the `rest` flatten is buying. `ContentPart` at `llm/types/messages.rs:45-55` is the precedent: it is `#[serde(untagged)]` with `Text { type, text, #[serde(flatten)] rest }` and a trailing `Unknown(Value)`, no `deny_unknown_fields`. This design follows that pattern; the only extension is the larger number of known variants.

## Controller and xDS

All mechanical:

- `crates/protos/proto/resource.proto`: add `gemini_api` enum field on `Vertex`.
- `controller/api/v1alpha1/agentgateway/agentgateway_backend_types.go`: add `GeminiApi *VertexGeminiApi` field on `VertexAIConfig` with the kubebuilder enum/default annotations.
- `controller/pkg/syncer/backend/backend_plugin.go:492`: copy the field onto the xDS `Vertex` message.
- `crates/agentgateway/src/types/agent_xds.rs:1113`: copy the field from xDS to IR. Update the two fixture tests at `:3685` (`test_vertex_provider_empty_region_is_none`) and `:3728` (`test_vertex_provider_with_region`) to round-trip both enum values.
- `crates/agentgateway/src/types/local.rs:2167`: copy the field for local-YAML ingestion.
- `controller/install/helm/agentgateway-crds/templates/agentgateway.dev_*.yaml`: regenerated via `make -C controller generate-all`. Not hand-edited.
- `schema/config.{json,md}`, `schema/cel.{json,md}`: regenerated via `make generate-schema`. Picks up the new field and the two new CEL bindings.

Controller-side unit tests in `backend_plugin_test.go` cover the VertexAI translation around `:426`; add a `geminiApi: native` case, an absent-field case (expected to default to `OPENAI_COMPAT`), and rely on existing CRD-validation conformance tests for the invalid-value case.

## Policy Attachment

No new attachment surfaces. Every policy that attaches to an AI backend today (rate limiting, prompt enrichment, prompt guard request and response, guardrails including Google Model Armor, CEL telemetry, `LLMInfo`, model alias) runs against the IR (`LLMRequest` / `LLMResponse`), which is upstream of the new translator on the request path and downstream of it on the response path. The wire shape is invisible to policy logic. Switching a route from compat to native requires no policy reconfiguration. Existing pipeline order at `llm/mod.rs:860-925` is unchanged. The only ordering note worth pulling out: model alias resolution runs before the Gemini-native heuristic in `get_path_for_model`, so an alias like `fast-model → gemini-2.5-flash` correctly routes through the native arm.

Two new CEL/log bindings (`llm.apiSurface`, `llm.upstreamFinishReason`) are registered through the `ContextBuilder` so `make generate-schema` picks them up. `gen_ai.provider.name` stays `gcp.vertex_ai` so existing billing dashboards don't have to change.

## Compatibility and Migration

The bar: no existing config or wire payload changes meaning after this PR merges. Configs without `geminiApi` produce byte-identical upstream URLs, request bodies, and response shapes. Old control planes against new data planes get the proto3 zero (`OPENAI_COMPAT`) and behave as today. New control planes against old data planes have the field silently ignored.

Operators opting in get new URL, new request body, native `usageMetadata` token counts (likely different from shim numbers), populated `reasoning_content` when they pass `reasoning_effort`, and a one-time prompt-cache warm-up on first turn. Token-based rate limits should be re-validated; the recommended recalibration mechanism is `tokenCosts` multipliers in [#1759](https://github.com/agentgateway/agentgateway/pull/1759).

### Rollout

1. This PR lands the opt-in. Default stays `openaiCompat`. No deprecations.
2. Future short proposal flips the default once the native path has community use (no open native-path bugs for a release, positive feedback, no regressions). Likely adds a compat-path deprecation log at that point.
3. Future proposal removes the OpenAI-compat path after a full deprecation cycle. No timeline.

### Library API for embedders

`vertex::Provider` becomes `#[non_exhaustive]`. Downstream embedders constructing it via struct-literal (`Provider { project_id, model, region }`) will get a compile error. Migration is one line per construction site, using chained setters added in the same PR:

```rust
impl Provider {
    pub fn new(project_id: Strng) -> Self;
    pub fn with_model(self, model: Strng) -> Self;
    pub fn with_region(self, region: Strng) -> Self;
    pub fn with_gemini_api(self, mode: GeminiApiMode) -> Self;
}

// Before: Provider { project_id, model: Some(m), region: Some(r) }
// After:  Provider::new(project_id).with_model(m).with_region(r)
```

Each future field gets a matching `with_<field>` setter. No public `ProviderBuilder` type: it would add its own `#[non_exhaustive]` versioning concern without solving anything the chained setters don't already solve.

## Risks and Tradeoffs

| Risk | Severity | Mitigation |
|---|---|---|
| Missing parallel `is_gemini_native_model` branches at the five response-side dispatch sites cause native bodies to be parsed as compat shape. | High | All five sites land in the same commit as the request-side branch. Integration tests cover request and response on completions and messages, streaming and non-streaming. |
| Trailing `usageMetadata`-only SSE chunk dropped if translator closes on `finishReason`. | Medium | Accumulate until upstream stream-end, not `finishReason`. Fixture covers trailing-chunk usage. |
| Tool-call fidelity in parallel calls or multi-tool turns (same family as [#1988](https://github.com/agentgateway/agentgateway/issues/1988)'s Bedrock `arguments: null` first-chunk bug). | Medium | Explicit tests: parallel tool calls in one turn, tool result followed by another tool call, JSON-typed args, synthetic id round-trip for 2.5, non-null `arguments` on first streaming chunk. |
| Local-tokenizer pre-flight counts diverge from Gemini's SentencePiece by 10–30%. | Medium | Documented in release notes. Response-side counts come from native `usageMetadata` and are accurate. Native `:countTokens` follow-up closes the gap. |
| Cost surprise on Gemini 2.5 Pro thinking (cannot be fully disabled; min 128, default dynamic). | Medium | Translator never auto-enables thinking. Bare native request emits no `thinkingConfig`. Release notes call out the Pro asymmetry. |
| Vertex API drift before or after merge (new `Part` variant, new `finishReason` value, deprecated field). | Low–Medium | `Part::Unknown(Value)` catch-all, `_ => "stop"` fallback, `#[serde(flatten)] rest` on every wire DTO. Translator surface is small enough that drift is fixable in a small follow-up. |
| Gemini 2.5 Flash emits thought content as `"THOUGHT:"`-prefixed text with `part.thought = None` ([googleapis/python-genai#2121](https://github.com/googleapis/python-genai/issues/2121)). | Low | Inline workaround: a text part starting with `"THOUGHT:"` and `part.thought` absent routes to `reasoning_content`. Remove when upstream fixes. |
| Downstream OpenAI clients hang waiting for `[DONE]`. | Low | Always synthesise the sentinel; streaming integration test asserts it. |

### Why not detect Gemini automatically without a flag

A user passing OpenAI-only knobs through `rest` would see silent behavioural change on upgrade. A flag is one provider field and gives operators a kill switch when a translator bug ships. Not worth saving the field.

### Why not piggyback on `AIProvider::Gemini`

Vertex auth (GCP ADC), regional hosts, project/region quoting are Vertex-specific. Users already on Vertex shouldn't have to switch providers and reconfigure auth.

### Why not just change the URL and leave the body OpenAI-shaped

`:generateContent` doesn't accept OpenAI chat completions JSON. The 400s would be instant.

### Why not auto-fetch HTTPS image URLs and inline them

Auto-fetching introduces implicit egress, an unbounded fetch surface area, and unbounded memory cost per request, none of which the proxy gates elsewhere. Reject is documented and relaxable in a follow-up.

## Test Plan

Unit tests cover: URL selection for the new model heuristic (including embedding-prefix exclusions) on streaming and non-streaming, both modes, global and regional; OpenAI ↔ Gemini round-trip across all mapping-table rows; Anthropic ↔ Gemini round-trip; every documented `finishReason` value mapped per the response table, including the `STOP`+functionCall override and the `MALFORMED_FUNCTION_CALL` / `UNEXPECTED_TOOL_CALL` cases that preserve the original value in `llm.upstreamFinishReason`; `tool_choice` mapping all four shapes; tool-result shape (`role=user` outbound, `role=function` accepted inbound); function-call id round-trip including parallel-identical-args; `cachedContent` passthrough; `THOUGHT:` prefix workaround; `response_format` for all three input shapes; thinking config defaults and `reasoning_effort` mapping for 2.5 and 3.x; streaming sub-cases (usage in same chunk as `finishReason`, usage in trailing chunk, mid-`data:` boundary split, `delta.role` emitted exactly once, synthesised id on first tool-call chunk); Anthropic block-ordering for interleaved thought/text chunks.

Response-dispatch tests cover all five sites in `mod.rs` for both Completions and Messages, native path, including the error-translation fixture. A unit test asserts native-mode count-tokens hits the local fallback.

Integration tests in `tests/integration.rs` use wiremock with recorded `:generateContent` and `:streamGenerateContent` payloads. They assert upstream URL (including `?alt=sse`), upstream body shape, downstream OpenAI- and Anthropic-shape responses, synthesised `data: [DONE]\n\n`, and `llm.apiSurface` / `llm.upstreamFinishReason` emission via the CEL/log harness.

Config tests extend the Vertex proto-to-IR fixtures at `agent_xds.rs:3685` and `:3728` to assert `gemini_api` round-trips with both enum values. A new `examples/vertex-gemini-native/config.yaml` is parsed by `validate_examples.rs`. `make generate-schema` regen is committed and must include `llm.apiSurface` and `llm.upstreamFinishReason` in `schema/cel.md`.

## Open Questions

None outstanding. The non-obvious design decisions and their rationale are captured under Risks and Tradeoffs (the "Why not" subsections) and inline in the relevant Runtime Design sections (thinking config divergences from LiteLLM, tool-call ID synthesis, `STOP`+functionCall override, `finish_details` rejection in favour of `llm.upstreamFinishReason`).
