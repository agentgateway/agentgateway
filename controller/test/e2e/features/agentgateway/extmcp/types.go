//go:build e2e

package extmcp

import (
	"path/filepath"
	"time"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

type testingSuite struct {
	*base.BaseTestingSuite
}

var _ e2e.NewSuiteFunc = NewTestingSuite

var (
	mcpProto = "2025-03-26"
	httpOK   = 200

	gatewayHost = "extmcp.example.com"

	warmupTime = 75 * time.Millisecond

	extMcpManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "extmcp.yaml")

	extMcpSetup = base.TestCase{
		Manifests: []string{extMcpManifest},
	}
)

// ToolsCallResponse is a minimal projection of the JSON-RPC tools/call result.
// InitializeResponse and ToolsListResponse are reused from the sibling mcp package.
type ToolsCallResponse struct {
	Result *struct {
		IsError bool `json:"isError"`
	} `json:"result,omitempty"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error,omitempty"`
}
