package annotations

// LegacyMCPServiceHTTPPath is the legacy annotation used to specify the HTTP path for the MCP service. Users should switch to MCPServiceHTTPPath.
const LegacyMCPServiceHTTPPath = "kgateway.dev/mcp-path"

// MCPServiceHTTPPath is the annotation used to specify the HTTP path for the MCP service
const MCPServiceHTTPPath = "agentgateway.dev/mcp-path"

// MCPServiceTargetName is the annotation used to specify the target name for the MCP service.
// The value must be a valid Gateway API SectionName.
const MCPServiceTargetName = "agentgateway.dev/mcp-target-name"
