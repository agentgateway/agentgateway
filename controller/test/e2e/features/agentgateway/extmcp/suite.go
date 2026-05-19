//go:build e2e

// nolint: bodyclose
package extmcp

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/stretchr/testify/suite"

	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/agentgateway/mcp"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, base.TestCase{}, map[string]*base.TestCase{
			"TestExtMcpRequestDeniesForbiddenTool":   &extMcpSetup,
			"TestExtMcpRequestAllowsAllowedTool":     &extMcpSetup,
			"TestExtMcpResponseMutatesToolsListDesc": &extMcpSetup,
		}),
	}
}

// TestExtMcpRequestDeniesForbiddenTool verifies the request-phase policy:
// the ext-mcp server denies tools/call where the tool name contains "forbidden",
// and the gateway rejects the request before it reaches the upstream MCP backend.
func (s *testingSuite) TestExtMcpRequestDeniesForbiddenTool() {
	sid := s.initializeSession()

	argsJSON, _ := json.Marshal(map[string]any{"url": "https://example.com"})
	body := fmt.Sprintf(`{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":%q,"arguments":%s}}`, "forbidden-tool", string(argsJSON))
	headers := mcp.WithSessionID(mcp.MCPHeaders(gatewayHost, mcpProto, nil), sid)

	resp, raw, err := mcp.ExecCurlMCP(s.T(), gatewayHost, headers, body)
	s.Require().NoError(err, "tools/call curl failed")
	s.Require().Equal(http.StatusBadRequest, resp.StatusCode, "denied tools/call should return 400, body=%s", raw)
	s.Require().Contains(strings.ToLower(raw), "forbidden-tool", "deny response should name the forbidden tool")
}

// TestExtMcpRequestAllowsAllowedTool verifies the request-phase policy lets
// the tool name "fetch" through to the upstream and returns a successful
// JSON-RPC result.
func (s *testingSuite) TestExtMcpRequestAllowsAllowedTool() {
	sid := s.initializeSession()

	resp := s.callTool(sid, "fetch", map[string]any{"url": "https://example.com"})
	s.Require().Nil(resp.Error, "fetch should pass the extMcp request phase, got error %+v", resp.Error)
	s.Require().NotNil(resp.Result, "fetch should produce a result")
}

// TestExtMcpResponseMutatesToolsListDesc verifies the response-phase policy:
// the ext-mcp server appends " [extmcp]" to every tool description, and the
// gateway substitutes the mutated payload before returning to the client.
func (s *testingSuite) TestExtMcpResponseMutatesToolsListDesc() {
	sid := s.initializeSession()

	tools := s.listTools(sid)
	s.Require().NotEmpty(tools, "expected at least one tool")
	for _, t := range tools {
		s.Require().True(
			strings.HasSuffix(t.Description, "[extmcp]"),
			"tool %q description %q missing extmcp mutation suffix",
			t.Name, t.Description,
		)
	}
}

// --- session helpers ---

func (s *testingSuite) initializeSession() string {
	headers := mcp.MCPHeaders(gatewayHost, mcpProto, nil)
	body := fmt.Sprintf(`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":%q,"capabilities":{"roots":{}},"clientInfo":{"name":"extmcp-e2e","version":"1.0.0"}}}`, mcpProto)

	// Wait for the route + policy to settle.
	mcp.SendMCP(s.T(), &testmatchers.HttpResponse{StatusCode: httpOK}, gatewayHost, headers, body)

	httpResp, raw, err := mcp.ExecCurlMCP(s.T(), gatewayHost, headers, body)
	s.Require().NoError(err, "initialize curl failed")

	payload, ok := mcp.FirstSSEDataPayload(raw)
	s.Require().True(ok, "initialize expected SSE payload, got: %s", raw)

	var initResp mcp.InitializeResponse
	s.Require().NoError(json.Unmarshal([]byte(payload), &initResp), "initialize unmarshal failed")
	if initResp.Result != nil && initResp.Result.ProtocolVersion != "" {
		mcpProto = initResp.Result.ProtocolVersion
	}

	sid := mcp.ExtractMCPSessionID(httpResp)
	s.Require().NotEmpty(sid, "initialize must return mcp-session-id")

	// notifications/initialized to register the session before the first RPC.
	notify := `{"jsonrpc":"2.0","method":"notifications/initialized"}`
	_, _, _ = mcp.ExecCurlMCP(s.T(), gatewayHost, mcp.WithSessionID(headers, sid), notify)
	time.Sleep(warmupTime)

	return sid
}

func (s *testingSuite) callTool(sessionID, name string, args map[string]any) ToolsCallResponse {
	argsJSON, _ := json.Marshal(args)
	body := fmt.Sprintf(`{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":%q,"arguments":%s}}`, name, string(argsJSON))

	headers := mcp.WithSessionID(mcp.MCPHeaders(gatewayHost, mcpProto, nil), sessionID)
	mcp.SendMCP(s.T(), &testmatchers.HttpResponse{StatusCode: httpOK}, gatewayHost, headers, body)

	_, raw, err := mcp.ExecCurlMCP(s.T(), gatewayHost, headers, body)
	s.Require().NoError(err, "tools/call curl failed")

	payload, ok := mcp.FirstSSEDataPayload(raw)
	s.Require().True(ok, "tools/call expected SSE payload, got: %s", raw)

	var resp ToolsCallResponse
	s.Require().NoError(json.Unmarshal([]byte(payload), &resp), "tools/call unmarshal failed: %s", payload)
	return resp
}

func (s *testingSuite) listTools(sessionID string) []struct {
	Name        string `json:"name"`
	Description string `json:"description,omitempty"`
} {
	body := `{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}`
	headers := mcp.WithSessionID(mcp.MCPHeaders(gatewayHost, mcpProto, nil), sessionID)
	mcp.SendMCP(s.T(), &testmatchers.HttpResponse{StatusCode: httpOK}, gatewayHost, headers, body)

	_, raw, err := mcp.ExecCurlMCP(s.T(), gatewayHost, headers, body)
	s.Require().NoError(err, "tools/list curl failed")

	payload, ok := mcp.FirstSSEDataPayload(raw)
	s.Require().True(ok, "tools/list expected SSE payload, got: %s", raw)

	var resp mcp.ToolsListResponse
	s.Require().NoError(json.Unmarshal([]byte(payload), &resp), "tools/list unmarshal failed: %s", payload)
	s.Require().Nil(resp.Error, "tools/list returned error: %+v", resp.Error)
	s.Require().NotNil(resp.Result, "tools/list missing result")
	return resp.Result.Tools
}
