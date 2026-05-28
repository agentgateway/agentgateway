//go:build e2e

// nolint: bodyclose
package e2e_test

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const extMcpGatewayHost = "extmcp.example.com"

var extMcpSetupManifest = manifest("extmcp", "extmcp.yaml")

// ToolsCallResponse is a minimal projection of the JSON-RPC tools/call result.
type extMcpToolsCallResponse struct {
	Result *struct {
		IsError bool `json:"isError"`
	} `json:"result,omitempty"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error,omitempty"`
}

func TestExtMCP(tt *testing.T) {
	t := New(tt)
	t.Run("RequestDeniesForbiddenTool", func(t base.Test) {
		t.Apply(extMcpSetupManifest)
		testExtMcpRequestDeniesForbiddenTool(t)
	})
	t.Run("RequestAllowsAllowedTool", func(t base.Test) {
		t.Apply(extMcpSetupManifest)
		testExtMcpRequestAllowsAllowedTool(t)
	})
	t.Run("ResponseMutatesToolsListDesc", func(t base.Test) {
		t.Apply(extMcpSetupManifest)
		testExtMcpResponseMutatesToolsListDesc(t)
	})
}

// testExtMcpRequestDeniesForbiddenTool verifies the request-phase policy:
// the ext-mcp server denies tools/call where the tool name contains "forbidden",
// and the gateway rejects the request before it reaches the upstream MCP backend.
func testExtMcpRequestDeniesForbiddenTool(t base.Test) {
	sid := extMcpInitializeSession(t)

	argsJSON, _ := json.Marshal(map[string]any{"url": "https://example.com"})
	body := fmt.Sprintf(`{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":%q,"arguments":%s}}`, "forbidden-tool", string(argsJSON))
	headers := withSessionID(MCPHeaders(extMcpGatewayHost, mcpProto, nil), sid)

	resp, raw, err := execCurlMCPHost(t, extMcpGatewayHost, headers, body)
	if err != nil {
		t.Fatalf("tools/call curl failed: %v", err)
	}
	if resp.StatusCode != http.StatusBadRequest {
		t.Fatalf("denied tools/call should return 400, got %d body=%s", resp.StatusCode, raw)
	}
	if !strings.Contains(strings.ToLower(raw), "forbidden-tool") {
		t.Fatalf("deny response should name the forbidden tool, got %s", raw)
	}
}

// testExtMcpRequestAllowsAllowedTool verifies the request-phase policy lets
// the tool name "fetch" through to the upstream and returns a successful
// JSON-RPC result.
func testExtMcpRequestAllowsAllowedTool(t base.Test) {
	sid := extMcpInitializeSession(t)
	resp := extMcpCallTool(t, sid, "fetch", map[string]any{"url": "https://example.com"})
	if resp.Error != nil {
		t.Fatalf("fetch should pass the extMcp request phase, got error %+v", resp.Error)
	}
	if resp.Result == nil {
		t.Fatal("fetch should produce a result")
	}
}

// testExtMcpResponseMutatesToolsListDesc verifies the response-phase policy:
// the ext-mcp server appends " [extmcp]" to every tool description, and the
// gateway substitutes the mutated payload before returning to the client.
func testExtMcpResponseMutatesToolsListDesc(t base.Test) {
	sid := extMcpInitializeSession(t)

	body := `{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}`
	headers := withSessionID(MCPHeaders(extMcpGatewayHost, mcpProto, nil), sid)
	sendMCPHost(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, extMcpGatewayHost, headers, body)

	_, raw, err := execCurlMCPHost(t, extMcpGatewayHost, headers, body)
	if err != nil {
		t.Fatalf("tools/list curl failed: %v", err)
	}
	payload, ok := FirstSSEDataPayload(raw)
	if !ok {
		t.Fatalf("tools/list expected SSE payload, got: %s", raw)
	}
	var resp ToolsListResponse
	if err := json.Unmarshal([]byte(payload), &resp); err != nil {
		t.Fatalf("tools/list unmarshal failed: %v payload=%s", err, payload)
	}
	if resp.Error != nil {
		t.Fatalf("tools/list returned error: %+v", resp.Error)
	}
	if resp.Result == nil || len(resp.Result.Tools) == 0 {
		t.Fatal("expected at least one tool")
	}
	for _, tool := range resp.Result.Tools {
		if !strings.HasSuffix(tool.Description, "[extmcp]") {
			t.Fatalf("tool %q description %q missing extmcp mutation suffix", tool.Name, tool.Description)
		}
	}
}

func extMcpInitializeSession(t base.Test) string {
	headers := MCPHeaders(extMcpGatewayHost, mcpProto, nil)
	body := fmt.Sprintf(`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":%q,"capabilities":{"roots":{}},"clientInfo":{"name":"extmcp-e2e","version":"1.0.0"}}}`, mcpProto)

	// Wait for the route + policy to settle.
	sendMCPHost(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, extMcpGatewayHost, headers, body)

	httpResp, raw, err := execCurlMCPHost(t, extMcpGatewayHost, headers, body)
	if err != nil {
		t.Fatalf("initialize curl failed: %v", err)
	}
	payload, ok := FirstSSEDataPayload(raw)
	if !ok {
		t.Fatalf("initialize expected SSE payload, got: %s", raw)
	}
	updateProtocolVersion(payload)

	sid := ExtractMCPSessionID(httpResp)
	if sid == "" {
		t.Fatal("initialize must return mcp-session-id")
	}

	// notifications/initialized to register the session before the first RPC.
	notify := `{"jsonrpc":"2.0","method":"notifications/initialized"}`
	_, _, _ = execCurlMCPHost(t, extMcpGatewayHost, withSessionID(headers, sid), notify)
	time.Sleep(warmupTime)
	return sid
}

func extMcpCallTool(t base.Test, sessionID, name string, args map[string]any) extMcpToolsCallResponse {
	argsJSON, _ := json.Marshal(args)
	body := fmt.Sprintf(`{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":%q,"arguments":%s}}`, name, string(argsJSON))

	headers := withSessionID(MCPHeaders(extMcpGatewayHost, mcpProto, nil), sessionID)
	sendMCPHost(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, extMcpGatewayHost, headers, body)

	_, raw, err := execCurlMCPHost(t, extMcpGatewayHost, headers, body)
	if err != nil {
		t.Fatalf("tools/call curl failed: %v", err)
	}
	payload, ok := FirstSSEDataPayload(raw)
	if !ok {
		t.Fatalf("tools/call expected SSE payload, got: %s", raw)
	}
	var resp extMcpToolsCallResponse
	if err := json.Unmarshal([]byte(payload), &resp); err != nil {
		t.Fatalf("tools/call unmarshal failed: %v payload=%s", err, payload)
	}
	return resp
}
