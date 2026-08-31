# Optional TOA verify for upstream MCP

Example only. Copy into a workflow that promotes or attaches an upstream MCP server behind agentgateway.

Agentgateway handles transport, auth, and CEL authorization ([mcp-authorization](../mcp-authorization/README.md)).
[TOA](https://github.com/Carmel-Labs-Inc/toa) (`toa/0.1`) verifies signed tool delivery evidence when `toa.json` is present.

TOA does not replace `mcpAuthorization`. No AgentStatus account is required to verify.

See [`toa-after-upstream.yml`](./toa-after-upstream.yml) and [architecture/toa-optional-upstream-gate.md](../../architecture/toa-optional-upstream-gate.md).
