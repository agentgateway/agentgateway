# MCP Response Compression

MCP tool call responses frequently contain large JSON payloads — lists of database rows, API results, file metadata — that consume significant context window tokens when consumed by LLM-based agents. Response compression converts these JSON payloads into compact tabular formats (markdown, TSV, CSV) at the proxy layer, reducing token usage while preserving the tabular structure. Note that nested objects are summarized as `{...}` and large arrays are truncated, so this is a lossy transformation optimized for LLM consumption rather than lossless encoding.

This document covers the design and architecture of the compression pipeline.

## Design Decisions

### Why at the proxy layer

Compression could happen at the MCP server, the client, or the proxy. Doing it at the proxy has several advantages:

* **No upstream changes required.** MCP servers return standard JSON; compression is transparent.
* **Per-target configuration.** Different backends may benefit from different formats — a data-heavy API might use TSV while a human-readable tool uses markdown.
* **Consistent behavior.** All clients benefit without each needing its own compression logic.

The tradeoff is that the proxy must parse and re-serialize JSON, adding latency proportional to response size. In practice this is small relative to the upstream call and LLM processing time.

### Format selection

The three formats target different consumption patterns:

* **Markdown** — best for LLMs that handle markdown well (most do). Preserves readability.
* **TSV** — minimal overhead, no escaping needed for most data. Good for structured pipelines.
* **CSV** — standard interchange format with proper escaping. Useful when downstream tooling expects CSV.

The `none` default means compression is opt-in; existing behavior is unchanged.

### What gets compressed

Compression targets `CallToolResult` messages containing text content that parses as JSON. The converter handles three shapes:

* **Arrays of objects** — the common case (e.g., database query results). Each object becomes a row, keys become column headers.
* **Wrapper objects with array fields** — scalar fields are preserved as header lines above the table, array fields are rendered as tables. This handles paginated API responses that wrap results in metadata.
* **Nested values** — arrays of 5 or fewer items are shown inline; larger arrays show the first 5 items plus a count. Nested objects display as `{...}`.

Non-tabular JSON (scalars, deeply nested structures) passes through unchanged. This is intentional — forcing non-tabular data into a table would lose information.

## Architecture

### Configuration flow

Response compression follows the same configuration pattern as other per-target settings:

1. **CRD** — `responseCompression` on [`AgentgatewayBackend`](../controller/api/v1alpha1/agentgateway/agentgateway_backend_types.go) MCP targets, with `enabled` (bool) and `format` (string) fields.
2. **Controller** — [`translate.go`](../controller/pkg/syncer/backend/translate.go) maps the CRD field into the xDS [`MCPTarget.ResponseCompression`](../crates/protos/proto/resource.proto) proto message.
3. **xDS → IR** — [`agent_xds.rs`](../crates/agentgateway/src/types/agent_xds.rs) converts the proto format string to the internal `CompressionFormat` enum on [`McpTarget`](../crates/agentgateway/src/types/agent.rs).

This maintains the project's design philosophy of nearly direct CRD → xDS → IR mappings.

### Runtime pipeline

The compression module lives in [`mcp/compress.rs`](../crates/agentgateway/src/mcp/compress.rs). At runtime:

1. The MCP handler in [`handler.rs`](../crates/agentgateway/src/mcp/handler.rs) checks the target's `response_compression` field.
2. If enabled, `compress_stream()` wraps the upstream response stream. For each `ServerJsonRpcMessage` containing a `CallToolResult` with text content, it calls `compress_response()`.
3. `compress_response()` attempts JSON parsing. If the content is valid JSON with tabular structure, it converts to the target format. Otherwise the content passes through unchanged.
4. Metrics are recorded for each attempt — see the [metrics section](#metrics) below.

The stream wrapping approach means compression happens inline without buffering the entire response, though individual tool call results are fully parsed.

### Metrics

Compression exposes Prometheus metrics through the standard agentgateway metrics registry in [`telemetry/metrics.rs`](../crates/agentgateway/src/telemetry/metrics.rs):

* `mcp_response_compression_total` / `mcp_response_compression_skipped_total` — counts of compressed vs. skipped responses.
* `mcp_response_compression_original_bytes` / `mcp_response_compression_compressed_bytes` — size histograms.
* `mcp_response_compression_ratio` — compression ratio (0.0–1.0).

All metrics carry `target` and `format` labels.

## Testing

Unit tests in [`compress_tests.rs`](../crates/agentgateway/src/mcp/compress_tests.rs) cover the core conversion logic: arrays of objects, nested values, wrapper objects, and non-convertible inputs. Integration with the handler is tested through the existing MCP test infrastructure in [`mcp_tests.rs`](../crates/agentgateway/src/mcp/mcp_tests.rs).
