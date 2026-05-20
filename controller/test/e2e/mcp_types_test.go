//go:build e2e

package e2e_test

import (
	"time"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

type mcpSuite struct {
	*base.BaseTestingSuite
}

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
	// Gateway defaults used by this feature suite
	gatewayName      = "gateway"
	gatewayNamespace = "agentgateway-base"

	// manifests
	staticSetupManifest      = manifest("mcp", "static.yaml")
	dynamicSetupManifest     = manifest("mcp", "dynamic.yaml")
	authnPolicyManifest      = manifest("mcp", "remote-authn-auth0.yaml")
	routeAuthnPolicyManifest = manifest("mcp", "remote-route-authn-auth0.yaml")

	// Dynamic test setup (only dynamic-specific resources)
	dynamicSetup = base.TestCase{
		Manifests: []string{dynamicSetupManifest},
	}

	// Static test setup (resources needed for non-dynamic tests)
	staticSetup = base.TestCase{
		Manifests: []string{staticSetupManifest},
	}

	// MCP authn keycloak test setup (resources needed for non-dynamic tests)
	authnSetup = base.TestCase{
		Manifests: []string{authnPolicyManifest},
	}

	// MCP authn keycloak test setup (resources needed for non-dynamic tests)
	authnRouteSetup = base.TestCase{
		Manifests: []string{routeAuthnPolicyManifest},
	}
)
