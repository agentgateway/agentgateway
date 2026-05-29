//go:build e2e

// nolint: bodyclose
package e2e_test

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"testing"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const extMcpGatewayHost = "extmcp.example.com"

var (
	extMcpSetupManifest = manifest("extmcp", "extmcp.yaml")
	extMcpHostHeader    = map[string]string{"Host": extMcpGatewayHost}
)

func TestExtMCP(tt *testing.T) {
	t := New(tt)
	t.Apply(extMcpSetupManifest)
	t.Run("RequestDeniesForbiddenTool", func(t base.Test) {
		testExtMcpRequestDeniesForbiddenTool(t)
	})
	t.Run("RequestAllowsAllowedTool", func(t base.Test) {
		testExtMcpRequestAllowsAllowedTool(t)
	})
	t.Run("ResponseMutatesToolsListDesc", func(t base.Test) {
		testExtMcpResponseMutatesToolsListDesc(t)
	})
}

// The ext-mcp testbox denies tools/call when the tool name contains "forbidden".
func testExtMcpRequestDeniesForbiddenTool(t base.Test) {
	sid := initializeAndGetSessionID(t, extMcpHostHeader)
	headers := withSessionID(mcpHeaders(extMcpHostHeader), sid)
	body := fmt.Sprintf(`{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":%q,"arguments":{}}}`, "forbidden-tool")

	// Gate on the ext-mcp policy backend being reachable before asserting the body:
	// the retrying Send warms up the (otherwise cold) gRPC connection, mirroring the
	// allowed/mutate cases. Without it, the first extmcp call in the suite can hang on
	// connect until curl's timeout.
	sendMCP(t, &testmatchers.HttpResponse{StatusCode: http.StatusBadRequest}, headers, body)
	resp, raw, err := execCurlMCP(t, headers, body)
	if err != nil {
		t.Fatalf("tools/call: %v", err)
	}
	if resp.StatusCode != http.StatusBadRequest {
		t.Fatalf("denied tools/call: status=%d body=%s", resp.StatusCode, raw)
	}
	if !strings.Contains(strings.ToLower(raw), "forbidden-tool") {
		t.Fatalf("deny response should name the forbidden tool: %s", raw)
	}
}

// The ext-mcp testbox allows tools/call for "fetch".
func testExtMcpRequestAllowsAllowedTool(t base.Test) {
	sid := initializeAndGetSessionID(t, extMcpHostHeader)
	headers := withSessionID(mcpHeaders(extMcpHostHeader), sid)
	args, _ := json.Marshal(map[string]any{"url": "https://example.com"})
	body := fmt.Sprintf(`{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"fetch","arguments":%s}}`, string(args))

	sendMCP(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, headers, body)
	_, raw, err := execCurlMCP(t, headers, body)
	if err != nil {
		t.Fatalf("tools/call: %v", err)
	}
	payload, ok := FirstSSEDataPayload(raw)
	if !ok {
		t.Fatalf("tools/call expected SSE payload: %s", raw)
	}
	var resp struct {
		Result *json.RawMessage `json:"result,omitempty"`
		Error  *struct {
			Code    int    `json:"code"`
			Message string `json:"message"`
		} `json:"error,omitempty"`
	}
	if err := json.Unmarshal([]byte(payload), &resp); err != nil {
		t.Fatalf("tools/call unmarshal: %v payload=%s", err, payload)
	}
	if resp.Error != nil {
		t.Fatalf("fetch should pass: %+v", resp.Error)
	}
	if resp.Result == nil {
		t.Fatal("fetch should produce a result")
	}
}

// The ext-mcp testbox appends " [extmcp]" to every tool description.
func testExtMcpResponseMutatesToolsListDesc(t base.Test) {
	sid := initializeAndGetSessionID(t, extMcpHostHeader)
	headers := withSessionID(mcpHeaders(extMcpHostHeader), sid)
	body := buildToolsListRequest(3)

	sendMCP(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, headers, body)
	_, raw, err := execCurlMCP(t, headers, body)
	if err != nil {
		t.Fatalf("tools/list: %v", err)
	}
	payload, ok := FirstSSEDataPayload(raw)
	if !ok {
		t.Fatalf("tools/list expected SSE payload: %s", raw)
	}
	var resp ToolsListResponse
	if err := json.Unmarshal([]byte(payload), &resp); err != nil {
		t.Fatalf("tools/list unmarshal: %v payload=%s", err, payload)
	}
	if resp.Error != nil {
		t.Fatalf("tools/list: %+v", resp.Error)
	}
	if resp.Result == nil || len(resp.Result.Tools) == 0 {
		t.Fatal("expected at least one tool")
	}
	for _, tool := range resp.Result.Tools {
		if !strings.HasSuffix(tool.Description, "[extmcp]") {
			t.Fatalf("tool %q missing [extmcp] mutation: %q", tool.Name, tool.Description)
		}
	}
}
