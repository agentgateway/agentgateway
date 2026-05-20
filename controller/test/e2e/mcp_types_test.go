//go:build e2e

package e2e_test

import "time"

type ToolsListResponse struct {
	JSONRPC string `json:"jsonrpc"`
	Result  *struct {
		Tools []struct {
			Name        string `json:"name"`
			Description string `json:"description,omitempty"`
		} `json:"tools"`
	} `json:"result,omitempty"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error,omitempty"`
}

type ResourcesListResponse struct {
	JSONRPC string `json:"jsonrpc"`
	Result  *struct {
		Resources []struct {
			URI  string `json:"uri"`
			Name string `json:"name,omitempty"`
		} `json:"resources"`
	} `json:"result,omitempty"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error,omitempty"`
}

// InitializeResponse models the MCP initialize payload.
type InitializeResponse struct {
	JSONRPC string `json:"jsonrpc"`
	ID      int    `json:"id"`
	Result  *struct {
		ProtocolVersion string         `json:"protocolVersion"`
		Capabilities    map[string]any `json:"capabilities"`
		ServerInfo      struct {
			Name    string `json:"name"`
			Version string `json:"version"`
		} `json:"serverInfo"`
		Instructions string `json:"instructions,omitempty"`
	} `json:"result,omitempty"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error,omitempty"`
}

// mcpProto is the protocol version for the MCP server
// This will be set dynamically from the initialize response

var (
	mcpProto   = "2025-03-26" // Default fallback, will be updated dynamically
	httpOKCode = 200
	warmupTime = 75 * time.Millisecond
)

var (
	// Gateway defaults used by these tests.
	gatewayName      = "gateway"
	gatewayNamespace = "agentgateway-base"

	// manifests
	staticSetupManifest      = manifest("mcp", "static.yaml")
	dynamicSetupManifest     = manifest("mcp", "dynamic.yaml")
	authnPolicyManifest      = manifest("mcp", "remote-authn-auth0.yaml")
	routeAuthnPolicyManifest = manifest("mcp", "remote-route-authn-auth0.yaml")

	dynamicSetup    = []string{dynamicSetupManifest}
	staticSetup     = []string{staticSetupManifest}
	authnSetup      = []string{authnPolicyManifest}
	authnRouteSetup = []string{routeAuthnPolicyManifest}
)
