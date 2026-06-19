//go:build e2e

// nolint: bodyclose // Too many false positives to handle
package e2e_test

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"maps"
	"net"
	"net/http"
	"strings"
	"time"

	"istio.io/istio/pkg/test/util/assert"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

// buildInitializeRequest is a helper function to build the initialize request for the MCP server
func buildInitializeRequest(clientName string, id int) string {
	return fmt.Sprintf(`{
		"method": "initialize",
		"params": {
			"protocolVersion": "%s",
			"capabilities": {"roots": {}},
			"clientInfo": {"name": "%s", "version": "1.0.0"}
		},
		"jsonrpc": "2.0",
		"id": %d
	}`, mcpProto, clientName, id)
}

// buildToolsListRequest is a helper function to build the tools list request for the MCP server
func buildToolsListRequest(id int) string {
	return fmt.Sprintf(`{
	  "method": "tools/list",
	  "params": {"_meta": {"progressToken": 1}},
	  "jsonrpc": "2.0",
	  "id": %d
	}`, id)
}

func buildNotifyInitializedRequest() string {
	return `{"jsonrpc":"2.0","method":"notifications/initialized"}`
}

// mcpHeaders returns the standard MCP request headers. If extraHeaders contains
// a "Host" entry it is forwarded as the HTTP Host header by mcpCurlOptions.
func mcpHeaders(extraHeaders map[string]string) map[string]string {
	h := map[string]string{
		"Content-Type":         "application/json",
		"Accept":               "application/json, text/event-stream",
		"MCP-Protocol-Version": mcpProto,
	}
	maps.Copy(h, extraHeaders)
	return h
}

func withSessionID(headers map[string]string, sessionID string) map[string]string {
	cp := make(map[string]string, len(headers)+1)
	maps.Copy(cp, headers)
	if sessionID != "" {
		cp["mcp-session-id"] = sessionID
	}
	return cp
}

// withRouteHeaders merges route-specific headers (like user-type) into a copy.
func withRouteHeaders(headers map[string]string, extras map[string]string) map[string]string {
	if len(extras) == 0 {
		return headers
	}
	cp := make(map[string]string, len(headers)+len(extras))
	maps.Copy(cp, headers)
	maps.Copy(cp, extras)
	return cp
}

func initializeAndGetSessionID(t base.Test, extraHeaders map[string]string) string {
	// Delegate to initializeSession, then warm the session to avoid races
	initBody := buildInitializeRequest("test-client", 1)
	headers := mcpHeaders(extraHeaders)
	sid := initializeSession(t, initBody, headers, "workflow")
	notifyInitialized(t, sid, extraHeaders)
	return sid
}

// nolint: unparam
func testUnauthorizedToolsListWithSession(t base.Test, sessionID string, extraHeaders map[string]string, expectedStatus int) {
	t.Log("Testing tools/list with session ID")

	mcpRequest := buildToolsListRequest(3)
	headers := withSessionID(mcpHeaders(extraHeaders), sessionID)
	sendMCP(t, &testmatchers.HttpResponse{StatusCode: expectedStatus}, headers, mcpRequest)

	if expectedStatus != httpOKCode {
		return
	}

	// If session was replaced, some gateways emit a JSON error as SSE payload (HTTP 200).
	// So parse SSE first, then decide.
	_, body, err := execCurlMCP(t, headers, mcpRequest)
	assert.NoError(t, err)
	payload, ok := FirstSSEDataPayload(body)
	if !ok {
		t.Log("No SSE payload from tools/list; sending notifications/initialized and retrying once")
		notifyInitialized(t, sessionID, extraHeaders)
		sendMCP(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, headers, mcpRequest)
		_, body, err = execCurlMCP(t, headers, mcpRequest)
		assert.NoError(t, err)
		payload, ok = FirstSSEDataPayload(body)
	}
	if !ok {
		t.Fatal("expected SSE data payload in tools/list (after retry)")
	}
	if !IsJSONValid(payload) {
		t.Fatalf("tools/list SSE payload is not valid JSON: %s", payload)
	}

	var toolsResp ToolsListResponse
	_ = json.Unmarshal([]byte(payload), &toolsResp)
	if toolsResp.Error != nil && strings.Contains(toolsResp.Error.Message, "Session not found") {
		// Re-init and retry once
		t.Log("Session expired; re-initializing and retrying tools/list")
		newID := initializeAndGetSessionID(t, extraHeaders)
		testToolsListWithSession(t, newID, extraHeaders)
		return
	}
}

func testToolsListWithSession(t base.Test, sessionID string, extraHeaders map[string]string) {
	t.Log("Testing tools/list with session ID")

	mcpRequest := buildToolsListRequest(3)
	headers := withSessionID(mcpHeaders(extraHeaders), sessionID)
	sendMCP(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, headers, mcpRequest)

	_, body, err := execCurlMCP(t, headers, mcpRequest)
	assert.NoError(t, err)

	// If session was replaced, some gateways emit a JSON error as SSE payload (HTTP 200).
	// So parse SSE first, then decide.
	payload, ok := FirstSSEDataPayload(body)
	if !ok {
		t.Log("No SSE payload from tools/list; sending notifications/initialized and retrying once")
		notifyInitialized(t, sessionID, extraHeaders)
		sendMCP(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, headers, mcpRequest)
		_, body, err = execCurlMCP(t, headers, mcpRequest)
		assert.NoError(t, err)
		payload, ok = FirstSSEDataPayload(body)
	}
	if !ok {
		t.Fatal("expected SSE data payload in tools/list (after retry)")
	}
	if !IsJSONValid(payload) {
		t.Fatalf("tools/list SSE payload is not valid JSON: %s", payload)
	}

	var toolsResp ToolsListResponse
	_ = json.Unmarshal([]byte(payload), &toolsResp)
	if toolsResp.Error != nil && strings.Contains(toolsResp.Error.Message, "Session not found") {
		// Re-init and retry once
		t.Log("Session expired; re-initializing and retrying tools/list")
		newID := initializeAndGetSessionID(t, extraHeaders)
		testToolsListWithSession(t, newID, extraHeaders)
		return
	}

	if toolsResp.Result == nil {
		t.Fatal("tools/list missing result")
	}
	t.Logf("tools: %d", len(toolsResp.Result.Tools))
	if len(toolsResp.Result.Tools) < 1 {
		t.Fatal("expected at least one tool")
	}
}

// notifyInitialized sends the "notifications/initialized" message once for a session.
func notifyInitialized(t base.Test, sessionID string, extraHeaders map[string]string) {
	mcpRequest := buildNotifyInitializedRequest()
	headers := withSessionID(mcpHeaders(extraHeaders), sessionID)

	resp, _, err := execCurlMCP(t, headers, mcpRequest)
	if err == nil && resp != nil && resp.StatusCode == http.StatusUnauthorized {
		t.Log("notifyInitialized hit 401; session likely already GC’d")
	}

	// Allow the gateway to register the session before the first RPC.
	time.Sleep(warmupTime)
}

func sendMCP(t base.Test, match *testmatchers.HttpResponse, headers map[string]string, body string) {
	base.BaseGateway.Send(t, match, mcpCurlOptions(headers, body)...)
}

func mcpCurlOptions(headers map[string]string, body string) []curl.Option {
	return curlPostOptions("/mcp", headers, body)
}

// curlPostOptions builds POST options for path with headers. A "Host" entry in
// headers is applied via curl.WithHostHeader so the gateway can route on it.
func curlPostOptions(path string, headers map[string]string, body string) []curl.Option {
	opts := []curl.Option{
		curl.WithPath(path),
		curl.WithMethod(http.MethodPost),
	}
	if host := headers["Host"]; host != "" {
		opts = append(opts, curl.WithHostHeader(host))
	}
	for k, v := range headers {
		if strings.EqualFold(k, "Host") {
			continue
		}
		opts = append(opts, curl.WithHeader(k, v))
	}
	if body != "" {
		opts = append(opts, curl.WithBody(body))
	}
	return opts
}

// execCurl runs a POST to path and returns the response and body text.
func execCurl(t base.Test, path string, headers map[string]string, body string) (*http.Response, string, error) {
	opts := append(
		base.GatewayAddressOptions(base.BaseGateway.ResolvedAddress()),
		curl.WithTimeout(30*time.Second),
	)
	opts = append(opts, curlPostOptions(path, headers, body)...)
	resp, err := curl.ExecuteRequest(opts...)
	if err != nil {
		return nil, "", err
	}
	defer resp.Body.Close()

	bodyBytes, readErr := io.ReadAll(resp.Body)
	if readErr != nil && !isTimeoutError(readErr) {
		return nil, "", readErr
	}

	bodyText := string(bodyBytes)
	t.Logf("mcp response status=%d content-type=%q body=%s", resp.StatusCode, resp.Header.Get("Content-Type"), bodyText)
	return resp, bodyText, nil
}

func execCurlMCP(t base.Test, headers map[string]string, body string) (*http.Response, string, error) {
	return execCurl(t, "/mcp", headers, body)
}

func isTimeoutError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return true
	}
	var netErr net.Error
	return errors.As(err, &netErr) && netErr.Timeout()
}

// ExtractMCPSessionID finds the mcp-session-id response header value.
func ExtractMCPSessionID(resp *http.Response) string {
	if resp == nil {
		return ""
	}
	return strings.TrimSpace(resp.Header.Get("mcp-session-id"))
}

// FirstSSEDataPayload returns the first full SSE "data:" event payload (coalescing multi-line data:)
// from a raw SSE response body.
func FirstSSEDataPayload(body string) (string, bool) {
	sc := bufio.NewScanner(strings.NewReader(body))
	var buf bytes.Buffer
	got := false

	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if after, ok := strings.CutPrefix(line, "data:"); ok {
			got = true
			payload := strings.TrimSpace(after)
			if buf.Len() > 0 {
				buf.WriteByte('\n')
			}
			buf.WriteString(payload)
			continue
		}
		if got && line == "" {
			break
		}
	}

	payload := strings.TrimSpace(buf.String())
	if payload == "" {
		return "", false
	}
	return payload, true
}

// IsJSONValid is a small helper to check the payload is valid JSON
func IsJSONValid(s string) bool {
	var js json.RawMessage
	return json.Unmarshal([]byte(s), &js) == nil
}

// updateProtocolVersion extracts and updates the global mcpProto from an initialize response
func updateProtocolVersion(payload string) {
	var initResp InitializeResponse
	if err := json.Unmarshal([]byte(payload), &initResp); err == nil {
		if initResp.Result != nil && initResp.Result.ProtocolVersion != "" {
			mcpProto = initResp.Result.ProtocolVersion
		}
	}
}

// mustListTools issues tools/list with an existing session and returns tool names.
// Pass routeHeaders (e.g., map[string]string{"user-type":"admin"}) so the gateway
// picks the same backend as the initialize call.
func mustListTools(t base.Test, sessionID, label string, routeHeaders map[string]string) []string {
	mcpRequest := buildToolsListRequest(999)
	headers := withRouteHeaders(withSessionID(mcpHeaders(nil), sessionID), routeHeaders)
	sendMCP(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, headers, mcpRequest)

	_, body, err := execCurlMCP(t, headers, mcpRequest)
	if err != nil {
		t.Fatalf("%s request failed: %v", label, err)
	}

	payload, ok := FirstSSEDataPayload(body)
	if !ok {
		t.Fatalf("%s expected SSE data payload", label)
	}

	var toolsResp ToolsListResponse
	if err := json.Unmarshal([]byte(payload), &toolsResp); err != nil {
		t.Fatalf("%s unmarshal failed: %v\npayload: %s", label, err, payload)
	}

	if toolsResp.Error != nil {
		// Common transient: session not warm yet; give it one nudge and retry once.
		if strings.Contains(strings.ToLower(toolsResp.Error.Message), "session not found") ||
			strings.Contains(strings.ToLower(toolsResp.Error.Message), "start sse client") {
			notifyInitializedWithHeaders(t, sessionID, routeHeaders)
			sendMCP(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, headers, mcpRequest)
			_, body, err = execCurlMCP(t, headers, mcpRequest)
			if err != nil {
				t.Fatalf("%s retry request failed: %v", label, err)
			}
			payload, ok = FirstSSEDataPayload(body)
			if !ok {
				t.Fatalf("%s expected SSE data payload (retry)", label)
			}
			if err := json.Unmarshal([]byte(payload), &toolsResp); err != nil {
				t.Fatalf("%s unmarshal failed (retry): %v", label, err)
			}
		}
	}
	if toolsResp.Error != nil {
		t.Fatalf("%s tools/list returned error: %d %s", label, toolsResp.Error.Code, toolsResp.Error.Message)
	}

	if toolsResp.Result == nil {
		t.Fatalf("%s missing result", label)
	}
	names := make([]string, 0, len(toolsResp.Result.Tools))
	for _, tool := range toolsResp.Result.Tools {
		names = append(names, tool.Name)
	}
	return names
}

func notifyInitializedWithHeaders(t base.Test, sessionID string, routeHeaders map[string]string) {
	mcpRequest := buildNotifyInitializedRequest()
	headers := withRouteHeaders(withSessionID(mcpHeaders(nil), sessionID), routeHeaders)
	_, _, _ = execCurlMCP(t, headers, mcpRequest)

	// Allow the gateway to register the session before the first RPC.
	time.Sleep(warmupTime)
}

func initializeSession(t base.Test, initBody string, hdr map[string]string, label string) string {
	// One deterministic probe with retry to ensure the endpoint is ready
	waitForMCP200(t, hdr, initBody, label)

	backoffs := []time.Duration{
		100 * time.Millisecond,
		250 * time.Millisecond,
		500 * time.Millisecond,
		1 * time.Second,
	}
	for attempt := 0; attempt <= len(backoffs); attempt++ {
		sendMCP(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, hdr, initBody)
		resp, body, err := execCurlMCP(t, hdr, initBody)
		if err != nil {
			t.Fatalf("%s initialize failed: %v", label, err)
		}

		payload, ok := FirstSSEDataPayload(body)
		if ok && strings.TrimSpace(payload) != "" {
			var initResp InitializeResponse
			_ = json.Unmarshal([]byte(payload), &initResp)
			if initResp.Error == nil && initResp.Result != nil {
				// Update the global protocol version from the server response
				updateProtocolVersion(payload)
				sid := ExtractMCPSessionID(resp)
				if sid == "" {
					t.Fatalf("%s initialize must return mcp-session-id header", label)
				}
				return sid
			}
			if initResp.Error != nil && !strings.Contains(strings.ToLower(initResp.Error.Message), "start sse client") {
				t.Fatalf("%s initialize returned error: %v", label, initResp.Error)
			}
		}

		if attempt < len(backoffs) {
			time.Sleep(backoffs[attempt])
		} else {
			t.Fatalf("%s initialize returned no SSE payload", label)
		}
	}
	return "" // unreachable
}

func waitForMCP200(t base.Test,
	headers map[string]string,
	body string,
	label string,
) {
	opts := append(
		base.GatewayAddressOptions(base.BaseGateway.ResolvedAddress()),
		mcpCurlOptions(headers, body)...,
	)
	base.BaseGateway.Send(t, &testmatchers.HttpResponse{StatusCode: httpOKCode}, opts...)
	t.Logf("%s init ready (status=%d)", label, httpOKCode)
}

// nolint: unparam
func testInitializeWithExpectedStatus(t base.Test, headers map[string]string, expectedStatus int, _ string) {
	initBody := buildInitializeRequest("test-client", 1)
	hdr := mcpHeaders(headers)
	sendMCP(t, &testmatchers.HttpResponse{StatusCode: expectedStatus}, hdr, initBody)
}

// waitForAuthnEnforced waits for authentication to actually be enforced by making
// unauthenticated requests until we get a 401 response. This ensures the authentication
// policy is not just accepted, but configured in the dataplane.
func waitForAuthnEnforced(t base.Test) {
	initBody := buildInitializeRequest("authn-check", 0)
	hdr := mcpHeaders(nil)
	sendMCP(t, &testmatchers.HttpResponse{StatusCode: http.StatusUnauthorized}, hdr, initBody)
	t.Log("waitForAuthnEnforced: authentication is enforced (got 401)")
}
