# LLM API Extensions

Request fields and values agentgateway accepts beyond the official API specs, and what they do.

## OpenAI Chat Completions

### Vertex and Gemini

**Generation parameters**

```json
{
  "top_k": 40,
  "thinking_config": {"thinkingBudget": 2048},
  "cached_content": "projects/p/locations/l/cachedContents/123",
  "safety_settings": [{"category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_NONE"}],
  "labels": {"team": "x"}
}
```

* `top_k`: sets `generationConfig.topK`.
* `thinking_config`: Gemini `thinkingConfig`; takes precedence over `reasoning_effort`.
* `cached_content`: Gemini cached content; `system`, `tools` and `tool_choice` are dropped since the cache holds them.
* `safety_settings`: Gemini safety settings.
* `labels`: Vertex request labels.

camelCase spellings (`thinkingConfig`, `cachedContent`, `safetySettings`) are also accepted.

**`gs://` URIs** in `image_url.url`, `file.file_id`, or `file.file_data`.

```json
{"type": "image_url", "image_url": {"url": "gs://bucket/cat.jpg"}}
{"type": "file", "file": {"file_id": "gs://bucket/report.pdf"}}
```

The MIME type comes from a MIME hint, `file.filename`, or the object's extension, in that order.

**MIME hints**: `format`, `mime_type`, or `content_type` on `image_url` or `file`. Accepts a MIME type
(`image/png`) or an extension (`png`).

```json
{"type": "image_url", "image_url": {"url": "gs://bucket/obj", "format": "image/png"}}
```

**`file_data` without a media type**: raw base64, or a `data:` URI with no media type. The MIME type comes from a
MIME hint or `file.filename`.

```json
{"type": "file", "file": {"filename": "report.pdf", "file_data": "JVBERi0x..."}}
{"type": "file", "file": {"filename": "report.pdf", "file_data": "data:;base64,JVBERi0x..."}}
```

### Anthropic and Bedrock

**Replaying reasoning**: send back the `reasoning_content` and `reasoning_signature` returned on a previous
assistant message to preserve thinking across turns. Ignored without a signature.

```json
{"role": "assistant", "content": "4", "reasoning_content": "2+2...", "reasoning_signature": "EqQB..."}
```

## OpenAI Responses

### Bedrock

**`vendor_extensions.thinking_budget_tokens`**: enables Anthropic thinking with the given budget.

```json
{"input": "hi", "vendor_extensions": {"thinking_budget_tokens": 4096}}
```

**`input_file` without a remote URL**: raw base64 in `file_data` (format from `filename`), or a `data:` URI in
`file_url`.

```json
{"type": "input_file", "filename": "report.pdf", "file_data": "JVBERi0x..."}
{"type": "input_file", "file_url": "data:application/pdf;base64,JVBERi0x..."}
```

## OpenAI Embeddings

Provider parameters are set at the top level of the request.

### Vertex

`task_type`, `title`, `auto_truncate`.

```json
{"model": "text-embedding-005", "input": "x", "task_type": "RETRIEVAL_QUERY"}
```

### Bedrock

* Nova: `embedding_purpose`, `truncation_mode`
* Cohere: `input_type`, `truncate`
* Titan: `normalize`

```json
{"model": "cohere.embed-v4", "input": ["x"], "input_type": "search_document", "truncate": "END"}
```

## All formats

### Bedrock

**`x-bedrock-metadata` header**: a flat JSON object sent as Bedrock `requestMetadata`. Applies to Chat Completions,
Responses, and Anthropic Messages.

```
x-bedrock-metadata: {"team": "x"}
```
