//go:build e2e

// nolint: bodyclose // Too many false positives to handle
package mcp

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
	"strconv"
	"strings"
	"time"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
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

// mcpHeaders returns a base set of headers for MCP requests.
// Accept includes both JSON and SSE to support initializing responses and streaming.
// Extra headers can be provided to include auth headers, etc.
func mcpHeaders(extraHeaders map[string]string) map[string]string {
	baseHeaders := map[string]string{
		"Content-Type":         "application/json",
		"Accept":               "application/json, text/event-stream",
		"MCP-Protocol-Version": mcpProto,
	}
	maps.Copy(baseHeaders, extraHeaders)
	return baseHeaders
}

// withSessionID returns a copy of headers including mcp-session-id.
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

func (s *testingSuite) initializeAndGetSessionID(extraHeaders map[string]string) string {
	// Delegate to initializeSession, then warm the session to avoid races
	initBody := buildInitializeRequest("test-client", 1)
	headers := mcpHeaders(extraHeaders)
	sid := s.initializeSession(initBody, headers, "workflow")
	s.notifyInitialized(sid, extraHeaders)
	return sid
}

func (s *testingSuite) testUnauthorizedToolsListWithSession(sessionID string, extraHeaders map[string]string, expectedStatus int) {
	s.T().Log("Testing tools/list with session ID")

	mcpRequest := buildToolsListRequest(3)
	headers := withSessionID(mcpHeaders(extraHeaders), sessionID)
	s.sendMCP(&testmatchers.HttpResponse{StatusCode: expectedStatus}, headers, mcpRequest, "--max-time", "10")

	if expectedStatus != httpOKCode {
		return
	}

	// If session was replaced, some gateways emit a JSON error as SSE payload (HTTP 200).
	// So parse SSE first, then decide.
	_, body, err := s.execCurlMCP(headers, mcpRequest, "--max-time", "10")
	s.Require().NoError(err, "tools/list request failed")
	payload, ok := FirstSSEDataPayload(body)
	if !ok {
		s.T().Log("No SSE payload from tools/list; sending notifications/initialized and retrying once")
		s.notifyInitialized(sessionID, extraHeaders)
		s.sendMCP(&testmatchers.HttpResponse{StatusCode: httpOKCode}, headers, mcpRequest, "--max-time", "10")
		_, body, err = s.execCurlMCP(headers, mcpRequest, "--max-time", "10")
		s.Require().NoError(err, "tools/list retry request failed")
		payload, ok = FirstSSEDataPayload(body)
	}
	s.Require().True(ok, "expected SSE data payload in tools/list (after retry)")
	s.Require().True(IsJSONValid(payload), "tools/list SSE payload is not valid JSON")

	var toolsResp ToolsListResponse
	s.Require().NoError(json.Unmarshal([]byte(payload), &toolsResp), "tools/list unmarshal failed")
	if toolsResp.Error != nil && strings.Contains(toolsResp.Error.Message, "Session not found") {
		// Re-init and retry once
		s.T().Log("Session expired; re-initializing and retrying tools/list")
		newID := s.initializeAndGetSessionID(extraHeaders)
		s.testToolsListWithSession(newID, extraHeaders)
		return
	}
}

func (s *testingSuite) testToolsListWithSession(sessionID string, extraHeaders map[string]string) {
	s.T().Log("Testing tools/list with session ID")

	mcpRequest := buildToolsListRequest(3)
	headers := withSessionID(mcpHeaders(extraHeaders), sessionID)
	resp, body, err := s.execCurlMCP(headers, mcpRequest, "--max-time", "10")
	s.Require().NoError(err, "tools/list request failed")
	s.Require().NotNil(resp, "tools/list expected HTTP response")
	s.Require().Equal(httpOKCode, resp.StatusCode, "tools/list unexpected status code")

	// If session was replaced, some gateways emit a JSON error as SSE payload (HTTP 200).
	// So parse SSE first, then decide.
	payload, ok := FirstSSEDataPayload(body)
	if !ok {
		s.T().Log("No SSE payload from tools/list; sending notifications/initialized and retrying once")
		s.notifyInitialized(sessionID, extraHeaders)
		resp, body, err = s.execCurlMCP(headers, mcpRequest, "--max-time", "10")
		s.Require().NoError(err, "tools/list retry request failed")
		s.Require().NotNil(resp, "tools/list retry expected HTTP response")
		s.Require().Equal(httpOKCode, resp.StatusCode, "tools/list retry unexpected status code")
		payload, ok = FirstSSEDataPayload(body)
	}
	s.Require().True(ok, "expected SSE data payload in tools/list (after retry)")
	s.Require().True(IsJSONValid(payload), "tools/list SSE payload is not valid JSON")

	var toolsResp ToolsListResponse
	s.Require().NoError(json.Unmarshal([]byte(payload), &toolsResp), "tools/list unmarshal failed")
	if toolsResp.Error != nil {
		if isRetryableToolsListError(toolsResp.Error.Message) {
			// Re-init and retry once on transient legacy-SSE/session failures.
			s.T().Logf("Transient tools/list error; re-initializing and retrying: %s", toolsResp.Error.Message)
			newID := s.initializeAndGetSessionID(extraHeaders)
			mcpRequest = buildToolsListRequest(4)
			headers = withSessionID(mcpHeaders(extraHeaders), newID)
			resp, body, err = s.execCurlMCP(headers, mcpRequest, "--max-time", "10")
			s.Require().NoError(err, "tools/list retry request failed")
			s.Require().NotNil(resp, "tools/list reinit retry expected HTTP response")
			s.Require().Equal(httpOKCode, resp.StatusCode, "tools/list reinit retry unexpected status code")
			payload, ok = FirstSSEDataPayload(body)
			s.Require().True(ok, "expected SSE data payload in tools/list (reinit retry)")
			s.Require().True(IsJSONValid(payload), "tools/list SSE payload is not valid JSON (reinit retry)")
			s.Require().NoError(json.Unmarshal([]byte(payload), &toolsResp), "tools/list reinit retry unmarshal failed")
			if toolsResp.Error == nil {
				s.Require().NotNil(toolsResp.Result, "tools/list missing result")
				s.T().Logf("tools: %d", len(toolsResp.Result.Tools))
				s.Require().GreaterOrEqual(len(toolsResp.Result.Tools), 1, "expected at least one tool")
				return
			}
			s.Require().Failf("tools/list", "tools/list returned error after reinit: %d %s", toolsResp.Error.Code, toolsResp.Error.Message)
			return
		}
		s.Require().Failf("tools/list", "tools/list returned error: %d %s", toolsResp.Error.Code, toolsResp.Error.Message)
	}

	s.Require().NotNil(toolsResp.Result, "tools/list missing result")
	s.T().Logf("tools: %d", len(toolsResp.Result.Tools))
	s.Require().GreaterOrEqual(len(toolsResp.Result.Tools), 1, "expected at least one tool")
}

// notifyInitialized sends the "notifications/initialized" message once for a session.
func (s *testingSuite) notifyInitialized(sessionID string, extraHeaders map[string]string) {
	mcpRequest := buildNotifyInitializedRequest()
	headers := withSessionID(mcpHeaders(extraHeaders), sessionID)

	resp, _, err := s.execCurlMCP(headers, mcpRequest, "--max-time", "2")
	if err == nil && resp != nil && resp.StatusCode == http.StatusUnauthorized {
		s.T().Log("notifyInitialized hit 401; session likely already GC’d")
	}

	// Allow the gateway to register the session before the first RPC.
	time.Sleep(warmupTime)
}

func (s *testingSuite) sendMCP(match *testmatchers.HttpResponse, headers map[string]string, body string, extraArgs ...string) {
	common.BaseGateway.Send(s.T(), match, s.mcpCurlOptions(headers, body, extraArgs...)...)
}

func (s *testingSuite) mcpCurlOptions(headers map[string]string, body string, extraArgs ...string) []curl.Option {
	return mcpCurlOptionsWithPort(80, headers, body, extraArgs...)
}

func mcpCurlOptionsWithPort(port int, headers map[string]string, body string, extraArgs ...string) []curl.Option {
	timeoutSec := parseMaxTimeSeconds(extraArgs, 10)
	if port == 8080 {
		port = 80
	}

	opts := []curl.Option{
		curl.WithPort(port),
		curl.WithPath("/mcp"),
		curl.WithMethod(http.MethodPost),
		curl.WithConnectionTimeout(timeoutSec),
	}
	for k, v := range headers {
		opts = append(opts, curl.WithHeader(k, v))
	}
	if body != "" {
		opts = append(opts, curl.WithBody(body))
	}
	return opts
}

// helper to run a request to a given path and return response and body text.
func (s *testingSuite) execCurl(path string, headers map[string]string, body string, extraArgs ...string) (*http.Response, string, error) {
	timeoutSec := parseMaxTimeSeconds(extraArgs, 10)
	opts := []curl.Option{
		curl.WithHost(common.BaseGateway.Address),
		curl.WithPort(80),
		curl.WithPath(path),
		curl.WithMethod(http.MethodPost),
		curl.WithConnectionTimeout(timeoutSec),
	}
	for k, v := range headers {
		opts = append(opts, curl.WithHeader(k, v))
	}
	if body != "" {
		opts = append(opts, curl.WithBody(body))
	}

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
	s.T().Logf("mcp response status=%d content-type=%q body=%s", resp.StatusCode, resp.Header.Get("Content-Type"), bodyText)
	return resp, bodyText, nil
}

// helper to run a POST to /mcp with optional headers and body
func (s *testingSuite) execCurlMCP(headers map[string]string, body string, extraArgs ...string) (*http.Response, string, error) {
	return s.execCurl("/mcp", headers, body, extraArgs...)
}

func parseMaxTimeSeconds(extraArgs []string, defaultSeconds int) int {
	for i := 0; i < len(extraArgs)-1; i++ {
		if extraArgs[i] == "--max-time" {
			if parsed, err := strconv.Atoi(extraArgs[i+1]); err == nil && parsed > 0 {
				return parsed
			}
		}
	}
	return defaultSeconds
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
func (s *testingSuite) mustListTools(sessionID, label string, routeHeaders map[string]string) []string {
	mcpRequest := buildToolsListRequest(999)
	headers := withRouteHeaders(withSessionID(mcpHeaders(nil), sessionID), routeHeaders)
	resp, body, err := s.execCurlMCP(headers, mcpRequest, "--max-time", "10")
	s.Require().NoError(err, "%s request failed", label)
	s.Require().NotNil(resp, "%s expected HTTP response", label)
	s.Require().Equal(httpOKCode, resp.StatusCode, "%s unexpected status code", label)

	payload, ok := FirstSSEDataPayload(body)
	s.Require().True(ok, "%s expected SSE data payload", label)

	var toolsResp ToolsListResponse
	if err := json.Unmarshal([]byte(payload), &toolsResp); err != nil {
		s.Require().Failf(label, "unmarshal failed: %v\npayload: %s", err, payload)
	}

	if toolsResp.Error != nil {
		// Common transient: session not warm yet or legacy SSE stream was dropped.
		if isRetryableToolsListError(toolsResp.Error.Message) {
			s.notifyInitializedWithHeaders(sessionID, routeHeaders)
			resp, body, err = s.execCurlMCP(headers, mcpRequest, "--max-time", "10")
			s.Require().NoError(err, "%s retry request failed", label)
			s.Require().NotNil(resp, "%s retry expected HTTP response", label)
			s.Require().Equal(httpOKCode, resp.StatusCode, "%s retry unexpected status code", label)
			payload, ok = FirstSSEDataPayload(body)
			s.Require().True(ok, "%s expected SSE data payload (retry)", label)
			s.Require().NoError(json.Unmarshal([]byte(payload), &toolsResp), "%s unmarshal failed (retry)", label)
		}
	}
	if toolsResp.Error != nil {
		s.Require().Failf(label, "tools/list returned error: %d %s", toolsResp.Error.Code, toolsResp.Error.Message)
	}

	s.Require().NotNil(toolsResp.Result, "%s missing result", label)
	names := make([]string, 0, len(toolsResp.Result.Tools))
	for _, tool := range toolsResp.Result.Tools {
		names = append(names, tool.Name)
	}
	return names
}

func isRetryableToolsListError(message string) bool {
	lower := strings.ToLower(message)
	return strings.Contains(lower, "session not found") ||
		strings.Contains(lower, "start sse client")
}

func isRetryableInitializeError(message string) bool {
	lower := strings.ToLower(message)
	return strings.Contains(lower, "start sse client") ||
		strings.Contains(lower, "all upstreams failed to respond to fanout")
}

func (s *testingSuite) notifyInitializedWithHeaders(sessionID string, routeHeaders map[string]string) {
	mcpRequest := buildNotifyInitializedRequest()
	headers := withRouteHeaders(withSessionID(mcpHeaders(nil), sessionID), routeHeaders)
	_, _, _ = s.execCurlMCP(headers, mcpRequest, "--max-time", "5")

	// Allow the gateway to register the session before the first RPC.
	time.Sleep(warmupTime)
}

func (s *testingSuite) initializeSession(initBody string, hdr map[string]string, label string) string {
	// One deterministic probe with retry to ensure the endpoint is ready
	s.waitForMCP200(8080, hdr, initBody, label)

	backoffs := []time.Duration{
		100 * time.Millisecond,
		250 * time.Millisecond,
		500 * time.Millisecond,
		1 * time.Second,
	}
	for attempt := 0; attempt <= len(backoffs); attempt++ {
		s.sendMCP(&testmatchers.HttpResponse{StatusCode: httpOKCode}, hdr, initBody, "--max-time", "10")
		resp, body, err := s.execCurlMCP(hdr, initBody, "--max-time", "10")
		s.Require().NoError(err, "%s initialize failed", label)

		payload, ok := FirstSSEDataPayload(body)
		if ok && strings.TrimSpace(payload) != "" {
			var initResp InitializeResponse
			s.Require().NoError(json.Unmarshal([]byte(payload), &initResp), "%s initialize payload unmarshal failed", label)
			if initResp.Error == nil && initResp.Result != nil {
				// Update the global protocol version from the server response
				updateProtocolVersion(payload)
				sid := ExtractMCPSessionID(resp)
				s.Require().NotEmpty(sid, "%s initialize must return mcp-session-id header", label)
				return sid
			}
			if initResp.Error != nil {
				if !isRetryableInitializeError(initResp.Error.Message) {
					s.Require().Failf(label, "initialize returned error: %v", initResp.Error)
				}
				s.T().Logf("%s initialize transient error (attempt %d/%d): %s", label, attempt+1, len(backoffs)+1, initResp.Error.Message)
			}
		}

		if attempt < len(backoffs) {
			time.Sleep(backoffs[attempt])
		} else {
			s.Require().Failf(label, "initialize returned no SSE payload")
		}
	}
	return "" // unreachable
}

func (s *testingSuite) waitForMCP200(
	port int,
	headers map[string]string,
	body string,
	label string,
) {
	opts := append(
		[]curl.Option{curl.WithHost(common.BaseGateway.Address)},
		mcpCurlOptionsWithPort(port, headers, body, "--max-time", "10")...,
	)
	common.BaseGateway.Send(s.T(), &testmatchers.HttpResponse{StatusCode: httpOKCode}, opts...)
	s.T().Logf("%s init ready (status=%d)", label, httpOKCode)
}

func (s *testingSuite) testInitializeWithExpectedStatus(headers map[string]string, expectedStatus int, _ string) {
	initBody := buildInitializeRequest("test-client", 1)
	hdr := mcpHeaders(headers)
	s.sendMCP(&testmatchers.HttpResponse{StatusCode: expectedStatus}, hdr, initBody, "--max-time", "10")
}

// waitForAuthnEnforced waits for authentication to actually be enforced by making
// unauthenticated requests until we get a 401 response. This ensures the authentication
// policy is not just accepted, but configured in the dataplane.
func (s *testingSuite) waitForAuthnEnforced() {
	initBody := buildInitializeRequest("authn-check", 0)
	hdr := mcpHeaders(nil)
	s.sendMCP(&testmatchers.HttpResponse{StatusCode: http.StatusUnauthorized}, hdr, initBody, "--max-time", "10")
	s.T().Log("waitForAuthnEnforced: authentication is enforced (got 401)")
}
