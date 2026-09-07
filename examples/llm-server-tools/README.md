## Server Tools Example

This example lets a client's server-side tools work against a model that cannot execute them.

When a client declares a tool the provider is supposed to run itself, such as a coding agent's
`web_search`, a vLLM or SGLang backend has nothing to run: the translation drops the tool and the
search silently does nothing. With a mapping, the gateway presents the tool to the model as an
ordinary function tool, and when the model calls it, runs the mapped MCP tool, feeds the result
back, and re-sends the conversation. The client only sees the finished answer, in its own wire
format, streaming included.

```yaml
serverTools:
  maxIterations: 3
  keepaliveInterval: 15s
  failureMode: failClosed
  tools:
  - type: web_search*
    mcp:
      backend: search
      tool: search
  mcpServers:
  - label: search
    backend: search
```

`tools[].type` matches the type the client declares, with a trailing `*` as a wildcard, so
`web_search*` covers `web_search_20250305` from an Anthropic Messages client, `web_search` and
`web_search_preview` from an OpenAI Responses client, and the `web_search_options` field a Chat
Completions client sends. `mcpServers[]` matches a Responses `mcp` descriptor by label or URL and
offers the backend's tools to the model by name.

Only tools the client itself declared, and only those with a mapping, are touched. Nothing is
injected into requests, client function tools are never rewritten, and the tool types a client
runs itself, such as Anthropic's `bash_*` and `text_editor_*`, are guarded by `clientExecuted`.

### Running the example

Point `backends[].mcp.targets[].mcp` at an MCP server that serves a search tool, and `baseUrl` at
an OpenAI-compatible endpoint, then start agentgateway:

```bash
cargo run -- -f examples/llm-server-tools/config.yaml
```

Send a Messages request that declares the tool the way a coding agent does:

```bash
curl http://localhost:4000/v1/messages -H 'content-type: application/json' -d '{
  "model": "qwen3",
  "max_tokens": 1024,
  "tools": [{"type": "web_search_20250305", "name": "web_search", "max_uses": 3}],
  "messages": [{"role": "user", "content": "What shipped in the last agentgateway release?"}]
}'
```

The answer comes back with the searches that produced it in front of it, as `server_tool_use` and
`web_search_tool_result` blocks, so a client that draws citations has something to show.

### What the gateway does with the calls

The MCP calls go through the normal relay, so backend policies, `mcpAuthorization` rules,
`mcpGuardrails`, and MCP access logging all apply. The calls of one turn run concurrently, their
results are cut at `maxResultBytes`, and the loop ends at `maxIterations` or when the model
repeats a call: the pending calls get an error result, the gateway's tools are withdrawn, and one
more model call ends the turn with an answer. An MCP failure returns an error to the client with
`failureMode: failClosed`, the default, or an error tool result to the model with `failOpen`.
