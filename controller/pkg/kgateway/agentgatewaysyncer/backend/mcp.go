package agentgatewaybackend

const (
	// mcpProtocol specifies that streamable HTTP protocol is to be used for the MCP target
	mcpProtocol = "agentgateway.dev/mcp"

	// mcpProtocolSSE specifies that Server-Sent Events (SSE) protocol is to be used for the MCP target
	mcpProtocolSSE = "agentgateway.dev/mcp-sse"

	// mcpProtocolLegacy is the legacy protocol name for streamable HTTP, kept for backwards compatibility
	mcpProtocolLegacy = "kgateway.dev/mcp"

	// mcpProtocolSSELegacy is the legacy protocol name for SSE, kept for backwards compatibility
	mcpProtocolSSELegacy = "kgateway.dev/mcp-sse"
)
