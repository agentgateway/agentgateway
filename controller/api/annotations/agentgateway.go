package annotations

// LegacyMCPServiceHTTPPath is the legacy annotation used to specify the HTTP path for the MCP service. Users should switch to MCPServiceHTTPPath.
const LegacyMCPServiceHTTPPath = "kgateway.dev/mcp-path"

// MCPServiceHTTPPath is the annotation used to specify the HTTP path for the MCP service
const MCPServiceHTTPPath = "agentgateway.dev/mcp-path"
