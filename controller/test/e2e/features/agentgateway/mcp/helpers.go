//go:build e2e

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
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
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
	out, err := s.execCurlMCP(headers, mcpRequest, "-N", "--max-time", "10")
	s.Require().NoError(err, "tools/list curl failed")

	// For non-200 status codes (like 401), check HTTP status directly without parsing SSE
	if expectedStatus != httpOKCode {
		s.requireHTTPStatus(out, expectedStatus)
		return
	}

	// Session is warmed during initialize; 401 retry no longer needed here.

	// If session was replaced, some gateways emit a JSON error as SSE payload (HTTP 200).
	// So parse SSE first, then decide.
	payload, ok := FirstSSEDataPayload(out)
	if !ok {
		s.T().Log("No SSE payload from tools/list; sending notifications/initialized and retrying once")
		s.notifyInitialized(sessionID, extraHeaders)
		out, err = s.execCurlMCP(headers, mcpRequest, "-N", "--max-time", "10")
		s.Require().NoError(err, "tools/list retry curl failed")
		s.requireHTTPStatus(out, httpOKCode)
		payload, ok = FirstSSEDataPayload(out)
	}
	s.Require().True(ok, "expected SSE data payload in tools/list (after retry)")
	s.Require().True(IsJSONValid(payload), "tools/list SSE payload is not valid JSON")

	var resp ToolsListResponse
	_ = json.Unmarshal([]byte(payload), &resp)

	if resp.Error != nil && strings.Contains(resp.Error.Message, "Session not found") {
		// Re-init and retry once
		s.T().Log("Session expired; re-initializing and retrying tools/list")
		newID := s.initializeAndGetSessionID(extraHeaders)
		s.testToolsListWithSession(newID, extraHeaders)
		return
	}

	s.requireHTTPStatus(out, expectedStatus)
}

func (s *testingSuite) testToolsListWithSession(sessionID string, extraHeaders map[string]string) {
	s.T().Log("Testing tools/list with session ID")

	mcpRequest := buildToolsListRequest(3)

	headers := withSessionID(mcpHeaders(extraHeaders), sessionID)
	out, err := s.execCurlMCP(headers, mcpRequest, "-N", "--max-time", "10")
	s.Require().NoError(err, "tools/list curl failed")

	// Session is warmed during initialize; 401 retry no longer needed here.

	// If session was replaced, some gateways emit a JSON error as SSE payload (HTTP 200).
	// So parse SSE first, then decide.
	payload, ok := FirstSSEDataPayload(out)
	if !ok {
		s.T().Log("No SSE payload from tools/list; sending notifications/initialized and retrying once")
		s.notifyInitialized(sessionID, extraHeaders)
		out, err = s.execCurlMCP(headers, mcpRequest, "-N", "--max-time", "10")
		s.Require().NoError(err, "tools/list retry curl failed")
		s.requireHTTPStatus(out, httpOKCode)
		payload, ok = FirstSSEDataPayload(out)
	}
	s.Require().True(ok, "expected SSE data payload in tools/list (after retry)")
	s.Require().True(IsJSONValid(payload), "tools/list SSE payload is not valid JSON")

	var resp ToolsListResponse
	_ = json.Unmarshal([]byte(payload), &resp)

	if resp.Error != nil && strings.Contains(resp.Error.Message, "Session not found") {
		// Re-init and retry once
		s.T().Log("Session expired; re-initializing and retrying tools/list")
		newID := s.initializeAndGetSessionID(extraHeaders)
		s.testToolsListWithSession(newID, extraHeaders)
		return
	}

	s.requireHTTPStatus(out, httpOKCode)
	s.Require().NotNil(resp.Result, "tools/list missing result")
	s.T().Logf("tools: %d", len(resp.Result.Tools))
	// If you expect at least one tool:
	s.Require().GreaterOrEqual(len(resp.Result.Tools), 1, "expected at least one tool")
}

// notifyInitialized sends the "notifications/initialized" message once for a session.
func (s *testingSuite) notifyInitialized(sessionID string, extraHeaders map[string]string) {
	mcpRequest := buildNotifyInitializedRequest()
	headers := withSessionID(mcpHeaders(extraHeaders), sessionID)
	// We don't care about the body; just make sure it doesn't 401.
	out, _ := s.execCurlMCP(headers, mcpRequest, "-N", "--max-time", "2")
	if strings.Contains(out, "401 Unauthorized") || strings.Contains(out, "HTTP_STATUS:401") {
		s.T().Log("notifyInitialized hit 401; session likely already GC’d")
	}
	// Allow the gateway to register the session before the first RPC.
	time.Sleep(warmupTime)
}

// helper to run a request to a given path and return combined output that
// includes response headers, body, and an HTTP_STATUS sentinel line.
func (s *testingSuite) execCurl(path string, headers map[string]string, body string, extraArgs ...string) (string, error) {
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
		return "", err
	}
	defer resp.Body.Close()

	bodyBytes, readErr := io.ReadAll(resp.Body)
	if readErr != nil && !isTimeoutError(readErr) {
		return "", readErr
	}

	out := formatHTTPOutput(resp, string(bodyBytes))
	s.T().Logf("mcp response: %s", out)
	return out, nil
}

// helper to run a POST to /mcp with optional headers and body and return combined output
func (s *testingSuite) execCurlMCP(headers map[string]string, body string, extraArgs ...string) (string, error) {
	out, err := s.execCurl("/mcp", headers, body, extraArgs...)
	s.T().Logf("execCurlMCP:\n%s", out) // always print
	return out, err
}

// execCurlMCPStatus returns just the HTTP status code deterministically.
func (s *testingSuite) execCurlMCPStatus(port int, headers map[string]string, body string, extraArgs ...string) (string, error) {
	timeoutSec := parseMaxTimeSeconds(extraArgs, 10)
	if port == 8080 {
		port = 80
	}
	opts := []curl.Option{
		curl.WithHost(common.BaseGateway.Address),
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

	resp, err := curl.ExecuteRequest(opts...)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	_, _ = io.Copy(io.Discard, resp.Body)
	return strconv.Itoa(resp.StatusCode), nil
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

func formatHTTPOutput(resp *http.Response, body string) string {
	var sb strings.Builder
	for k, values := range resp.Header {
		for _, v := range values {
			sb.WriteString(strings.ToLower(k))
			sb.WriteString(": ")
			sb.WriteString(v)
			sb.WriteString("\n")
		}
	}
	sb.WriteString("\n")
	sb.WriteString(body)
	if body != "" && !strings.HasSuffix(body, "\n") {
		sb.WriteString("\n")
	}
	sb.WriteString("HTTP_STATUS:")
	sb.WriteString(strconv.Itoa(resp.StatusCode))
	sb.WriteString("\n")
	sb.WriteString("Content-Type:")
	sb.WriteString(resp.Header.Get("Content-Type"))
	sb.WriteString("\n")
	return sb.String()
}

// helper to assert HTTP status from verbose curl output (supports HTTP/1.1 and HTTP/2)
func (s *testingSuite) requireHTTPStatus(out string, code int) {
	// Match "HTTP_STATUS:200"
	re := regexp.MustCompile(fmt.Sprintf(`(?m)^HTTP_STATUS:%d$`, code))
	if re.FindStringIndex(out) == nil {
		// Always log the body on mismatch to make failures actionable.
		s.T().Logf("HTTP status mismatch (wanted %d): %s", code, out)
		s.Require().Failf("HTTP status check", "expected HTTP %d; full output logged above", code)
	}
}

// ExtractMCPSessionID finds the mcp-session-id header value in a verbose curl output.
func ExtractMCPSessionID(out string) string {
	// Session IDs used to be UUIDs (hex + '-'), but are now an encoded state blob
	// (base64 / base64url / encrypted) which may include non-hex characters.
	// Capture the full header value up to whitespace/end-of-line.
	re := regexp.MustCompile(`(?mi)^\s*(?:<\s*)?mcp-session-id\s*:\s*([^\s\r\n]+)\s*$`)
	m := re.FindStringSubmatch(out)
	if len(m) > 1 {
		return strings.TrimSpace(m[1])
	}
	return ""
}

// FirstSSEDataPayload returns the first full SSE "data:" event payload (coalescing multi-line data:)
// from a verbose curl output or raw SSE stream.
func FirstSSEDataPayload(out string) (string, bool) {
	sc := bufio.NewScanner(strings.NewReader(out))
	var buf bytes.Buffer
	got := false
	for sc.Scan() {
		raw := sc.Text()
		// Curl verbose sometimes prefixes body lines with "<" or "< ".
		line := strings.TrimSpace(raw)
		// Find "data:" anywhere on the line (handles "data:", "<data:", "< data:", etc.)
		if _, after, ok := strings.Cut(line, "data:"); ok {
			got = true
			payload := strings.TrimSpace(after)
			if buf.Len() > 0 {
				buf.WriteByte('\n')
			}
			buf.WriteString(payload)
			continue
		}
		// Blank line after we started -> end of this SSE event
		if got && strings.TrimSpace(line) == "" {
			break
		}
	}
	s := strings.TrimSpace(buf.String())
	if s == "" {
		return "", false
	}
	return s, true
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
	out, err := s.execCurlMCP(headers, mcpRequest, "-N", "--max-time", "10")
	s.Require().NoError(err, "%s curl failed", label)
	s.requireHTTPStatus(out, httpOKCode)

	payload, ok := FirstSSEDataPayload(out)
	s.Require().True(ok, "%s expected SSE data payload", label)

	var resp ToolsListResponse

	if err := json.Unmarshal([]byte(payload), &resp); err != nil {
		s.Require().Failf(label, "unmarshal failed: %v\npayload: %s", err, payload)
	}
	if resp.Error != nil {
		// Common transient: session not warm yet; give it one nudge and retry once.
		if strings.Contains(strings.ToLower(resp.Error.Message), "session not found") ||
			strings.Contains(strings.ToLower(resp.Error.Message), "start sse client") {
			s.notifyInitializedWithHeaders(sessionID, routeHeaders)
			out, err = s.execCurlMCP(headers, mcpRequest, "-N", "--max-time", "10")
			s.Require().NoError(err, "%s retry curl failed", label)
			s.requireHTTPStatus(out, httpOKCode)
			payload, ok = FirstSSEDataPayload(out)
			s.Require().True(ok, "%s expected SSE data payload (retry)", label)
			s.Require().NoError(json.Unmarshal([]byte(payload), &resp), "%s unmarshal failed (retry)", label)
		}
	}
	if resp.Error != nil {
		s.Require().Failf(label, "tools/list returned error: %d %s", resp.Error.Code, resp.Error.Message)
	}
	s.Require().NotNil(resp.Result, "%s missing result", label)
	names := make([]string, 0, len(resp.Result.Tools))
	for _, t := range resp.Result.Tools {
		names = append(names, t.Name)
	}
	return names
}

func (s *testingSuite) notifyInitializedWithHeaders(sessionID string, routeHeaders map[string]string) {
	mcpRequest := buildNotifyInitializedRequest()
	headers := withRouteHeaders(withSessionID(mcpHeaders(nil), sessionID), routeHeaders)
	_, _ = s.execCurlMCP(headers, mcpRequest, "-N", "--max-time", "5")
	// Allow the gateway to register the session before the first RPC.
	time.Sleep(warmupTime)
}

func (s *testingSuite) initializeSession(initBody string, hdr map[string]string, label string) string {
	// One deterministic probe with retry to ensure the endpoint is ready
	s.waitForMCP200(8080, hdr, initBody, label,
		100*time.Millisecond, 250*time.Millisecond, 500*time.Millisecond, 1*time.Second)

	backoffs := []time.Duration{
		100 * time.Millisecond,
		250 * time.Millisecond,
		500 * time.Millisecond,
		1 * time.Second,
	}
	for attempt := 0; attempt <= len(backoffs); attempt++ {
		// Fetch the full response and parse
		out, err := s.execCurlMCP(hdr, initBody, "--max-time", "10")
		s.Require().NoError(err, "%s initialize failed", label)

		payload, ok := FirstSSEDataPayload(out)
		if ok && strings.TrimSpace(payload) != "" {
			var init InitializeResponse
			_ = json.Unmarshal([]byte(payload), &init)
			if init.Error == nil && init.Result != nil {
				// Update the global protocol version from the server response
				updateProtocolVersion(payload)
				sid := ExtractMCPSessionID(out)
				s.Require().NotEmpty(sid, "%s initialize must return mcp-session-id header", label)
				return sid
			}
			if init.Error != nil && !strings.Contains(strings.ToLower(init.Error.Message), "start sse client") {
				s.Require().Failf(label, "initialize returned error: %v", init.Error)
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
	backoffs ...time.Duration,
) {
	if len(backoffs) == 0 {
		backoffs = []time.Duration{
			100 * time.Millisecond, 250 * time.Millisecond,
			500 * time.Millisecond, 1 * time.Second,
		}
	}

	var (
		status string
		err    error
	)
	for attempt := 0; attempt <= len(backoffs); attempt++ {
		status, err = s.execCurlMCPStatus(port, headers, body, "--max-time", "10")
		if err == nil && strings.TrimSpace(status) == strconv.Itoa(httpOKCode) {
			s.T().Logf("%s init ready (status=%s)", label, status)
			return
		}
		if attempt < len(backoffs) {
			if err != nil {
				s.T().Logf("%s init status probe err: %v", label, err)
			}
			s.T().Logf("%s init status=%q; retrying in %s", label, status, backoffs[attempt])
			time.Sleep(backoffs[attempt])
			continue
		}
		s.Require().NoError(err, "%s initialize status probe failed", label)
		s.Require().Equal(httpOKCode, strings.TrimSpace(status), "expected HTTP "+strconv.Itoa(httpOKCode))
	}
}

// testInitializeWithExpectedStatus tests an initialize request and expects a specific HTTP status code
// It retries with backoff only for transient errors (503, 502, connection errors), not for getting
// a different status code when the gateway is clearly responding (e.g., 200 when expecting 401).
func (s *testingSuite) testInitializeWithExpectedStatus(headers map[string]string, expectedStatus int, label string) {
	initBody := buildInitializeRequest("test-client", 1)
	hdr := mcpHeaders(headers)

	backoffs := []time.Duration{
		100 * time.Millisecond,
		250 * time.Millisecond,
		500 * time.Millisecond,
		1 * time.Second,
	}

	var out string
	var err error
	for attempt := 0; attempt <= len(backoffs); attempt++ {
		out, err = s.execCurlMCP(hdr, initBody, "--max-time", "10")
		if err != nil {
			if attempt < len(backoffs) {
				s.T().Logf("%s initialize curl failed (attempt %d): %v; retrying", label, attempt+1, err)
				time.Sleep(backoffs[attempt])
				continue
			}
			s.Require().NoError(err, "%s initialize curl failed", label)
		}

		// Check if we got the expected status
		statusRe := regexp.MustCompile(`(?m)^HTTP_STATUS:(\d+)$`)
		matches := statusRe.FindStringSubmatch(out)
		if len(matches) > 1 {
			actualStatus, parseErr := strconv.Atoi(matches[1])
			if parseErr == nil {
				if actualStatus == expectedStatus {
					s.T().Logf("%s got expected status %d", label, expectedStatus)
					return
				}
				// Only retry for transient errors (503, 502, 504), not for wrong status codes
				// when the gateway is clearly responding (e.g., 200 when expecting 401)
				isTransientError := actualStatus == 503 || actualStatus == 502 || actualStatus == 504
				if isTransientError && attempt < len(backoffs) {
					s.T().Logf("%s got transient error %d, expected %d (attempt %d); retrying", label, actualStatus, expectedStatus, attempt+1)
					time.Sleep(backoffs[attempt])
					continue
				}
				// If we got a non-transient status code that doesn't match, fail immediately
				s.T().Logf("HTTP status mismatch (wanted %d, got %d): %s", expectedStatus, actualStatus, out)
				s.requireHTTPStatus(out, expectedStatus)
				return
			}
		}

		// If we couldn't parse the status and haven't exhausted retries, retry
		if attempt < len(backoffs) {
			s.T().Logf("%s could not parse HTTP status (attempt %d); retrying", label, attempt+1)
			time.Sleep(backoffs[attempt])
			continue
		}
	}

	// Final assertion after all retries exhausted (shouldn't reach here normally)
	s.T().Logf("HTTP status check failed after retries (wanted %d): %s", expectedStatus, out)
	s.requireHTTPStatus(out, expectedStatus)
}

// waitForAuthnEnforced waits for authentication to actually be enforced by making
// unauthenticated requests until we get a 401 response. This ensures the authentication
// policy is not just accepted, but configured in the dataplane.
func (s *testingSuite) waitForAuthnEnforced() {
	initBody := buildInitializeRequest("authn-check", 0)
	hdr := mcpHeaders(nil)

	backoffs := []time.Duration{
		200 * time.Millisecond,
		500 * time.Millisecond,
		1 * time.Second,
		2 * time.Second,
		4 * time.Second,
		8 * time.Second,
	}

	for attempt := 0; attempt <= len(backoffs); attempt++ {
		out, err := s.execCurlMCP(hdr, initBody, "--max-time", "10")
		if err != nil {
			if attempt < len(backoffs) {
				s.T().Logf("waitForAuthnEnforced: curl failed (attempt %d): %v; retrying", attempt+1, err)
				time.Sleep(backoffs[attempt])
				continue
			}
			s.Require().NoError(err, "waitForAuthnEnforced: curl failed")
		}

		statusRe := regexp.MustCompile(`(?m)^HTTP_STATUS:(\d+)$`)
		matches := statusRe.FindStringSubmatch(out)
		if len(matches) > 1 {
			actualStatus, parseErr := strconv.Atoi(matches[1])
			if parseErr == nil {
				if actualStatus == 401 {
					// Authentication is enforced!
					s.T().Logf("waitForAuthnEnforced: authentication is enforced (got 401)")
					return
				}
				// If we got 200, authentication is not enforced yet - retry
				if actualStatus == httpOKCode && attempt < len(backoffs) {
					s.T().Logf("waitForAuthnEnforced: got 200 (auth not enforced yet, attempt %d); retrying in %v", attempt+1, backoffs[attempt])
					time.Sleep(backoffs[attempt])
					continue
				}
				// If we got a transient error, retry
				isTransientError := actualStatus == 503 || actualStatus == 502 || actualStatus == 504
				if isTransientError && attempt < len(backoffs) {
					s.T().Logf("waitForAuthnEnforced: got transient error %d (attempt %d); retrying", actualStatus, attempt+1)
					time.Sleep(backoffs[attempt])
					continue
				}
				// Got unexpected status code
				s.Require().Failf("waitForAuthnEnforced failed", "unauthenticated request got status %d, expected 401 (auth not enforced)", actualStatus)
			}
		}

		// If we couldn't parse the status and haven't exhausted retries, retry
		if attempt < len(backoffs) {
			s.T().Logf("waitForAuthnEnforced: could not parse HTTP status (attempt %d); retrying", attempt+1)
			time.Sleep(backoffs[attempt])
			continue
		}
	}

	// Shouldn't reach here, but fail if we do
	s.Require().Fail("waitForAuthnEnforced: exhausted retries without getting 401")
}
