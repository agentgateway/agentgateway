//go:build e2e

package mcp

import (
	"context"
	"encoding/json"
	"regexp"
	"strings"
	"time"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
)

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, map[string]*base.TestCase{
			// Static tests
			"TestMCPWorkflow": &staticSetup,
			"TestSSEEndpoint": &staticSetup,
			// Dynamic tests
			"TestDynamicMCPAdminRouting":     &dynamicSetup,
			"TestDynamicMCPUserRouting":      &dynamicSetup,
			"TestDynamicMCPDefaultRouting":   &dynamicSetup,
			"TestDynamicMCPAdminVsUserTools": &dynamicSetup,
			// Authn tests
			"TestMCPAuthn": &authnSetup,
		}),
	}
}

func (s *testingSuite) TestMCPAuthn() {
	// Single test that does the full workflow with session management
	s.T().Log("Testing complete MCP workflow with session management")

	// Ensure static components are ready
	s.waitStaticReady()
	// Ensure auth0 server is ready
	s.waitAuth0Ready()

	// Wait for the authentication policy to be accepted before testing
	s.T().Log("Waiting for authentication policy to be accepted")
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwPolicyCondition(
		s.Ctx,
		"auth0-mcp-authn-policy",
		"default",
		"Accepted",
		metav1.ConditionTrue,
	)

	// The token is hard coded in the mock auth0 server
	testJwt := "eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzNTAyMzEyMTkzMDYwMzg2OTIiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzExNjM0MDcsIm5iZiI6MTc2MzU3OTQwNywiaWF0IjoxNzYzNTc5NDA3fQ.TsHCCdd0_629wibU4EviEi1-_UXaFUX1NuLgXCrC-tr7kqlcnUJIJC0WSab1EgXKtF8gTfwTUeQcAQNrunwngQU-K9DFcH5-2vnGeiXV3_X3SokkPq74ceRrCFEL2d7YNaGfhq_UNyvKRJsRz-pwdKK7QIPXALmWaUHn7EV7zU-CcPCKNwmt62P88qNp5HYSbgqz_WfnzIIH8LANpCC8fUqVedgTJMJ86E06pfDNUuuXe_fhjgMQXlfyDeUxIuzJunvS2qIqt4IYMzjcQbl2QI1QK3xz37tridSP_WVuuMUe2Lqo0oDjWVpxqPb5fb90W6a6khRP59Pf6qKMbQ9SQg"
	validAuthnHeader := map[string]string{"Authorization": "Bearer " + testJwt}

	// Verify authentication is actually enforced (not just policy accepted)
	// by waiting for an unauthenticated request to return 401
	s.T().Log("Verifying authentication is enforced")
	s.waitForAuthnEnforced()

	// Test 1: Initialize without token should fail
	s.T().Log("Test 1: Initialize without Authorization header should return 401")
	s.testInitializeWithExpectedStatus(nil, 401, "missing token")

	// Test 2: Initialize with invalid token should fail
	s.T().Log("Test 2: Initialize with invalid token should return 401")
	invalidAuthnHeader := map[string]string{"Authorization": "Bearer " + "fake"}
	s.testInitializeWithExpectedStatus(invalidAuthnHeader, 401, "invalid token")

	// Test 3: Initialize with valid token should succeed
	s.T().Log("Test 3: Initialize with valid token should succeed")
	sessionID := s.initializeAndGetSessionID(validAuthnHeader)
	s.Require().NotEmpty(sessionID, "Failed to get session ID from initialize")

	// Test 4: tools/list with valid token should succeed
	s.T().Log("Test 4: tools/list with valid token should succeed")
	s.testToolsListWithSession(sessionID, validAuthnHeader)

	// Test 5: tools/list with invalid token should fail
	s.T().Log("Test 5: tools/list with invalid token should fail")
	s.testUnauthorizedToolsListWithSession(sessionID, invalidAuthnHeader, 401)

	// Test 6: tools/list with missing token should fail
	s.T().Log("Test 6: tools/list with missing token should fail")
	s.testUnauthorizedToolsListWithSession(sessionID, nil, 401)
}

func (s *testingSuite) TestMCPWorkflow() {
	// Single test that does the full workflow with session management
	s.T().Log("Testing complete MCP workflow with session management")

	// Ensure static components are ready
	s.waitStaticReady()

	// Step 1: Initialize and get session ID
	sessionID := s.initializeAndGetSessionID(nil)
	s.Require().NotEmpty(sessionID, "Failed to get session ID from initialize")

	// Step 2: Test tools/list with session ID
	s.testToolsListWithSession(sessionID, nil)
}

func (s *testingSuite) TestSSEEndpoint() {
	// Ensure static components are ready
	s.waitStaticReady()

	initBody := buildInitializeRequest("sse-client", 0)

	headers := mcpHeaders(nil)

	out, err := s.execCurlMCP(headers, initBody, "--max-time", "8")
	s.Require().NoError(err, "SSE initialize curl failed")
	s.requireHTTPStatus(out, httpOKCode)
	// Match header w/ or w/o '-v' prefix, any casing, and optional params.
	headerCT := regexp.MustCompile(`(?mi)^\s*(?:<\s*)?content-type\s*:\s*text/event-stream(?:\s*;.*)?\s*$`)
	if headerCT.FindStringIndex(out) == nil {
		// Fallback to curl -w line (we print: "Content-Type:%{content_type}")
		wCT := regexp.MustCompile(`(?i)^Content-Type:\s*text/event-stream\b`)
		if !wCT.MatchString(out) {
			s.T().Logf("missing text/event-stream content-type: %s", out)
			s.Require().Fail("expected Content-Type: text/event-stream in response headers (or curl -w output)")
		}
	}
	_ = s.initializeSession(initBody, headers, "sse")
}

func (s *testingSuite) TestDynamicMCPAdminRouting() {
	s.waitDynamicReady()
	s.T().Log("Testing dynamic MCP routing for admin user")
	adminTools := s.runDynamicRoutingCase("admin-client", map[string]string{"user-type": "admin"}, "admin")
	// Admin will have more than two tools
	s.Require().GreaterOrEqual(len(adminTools), 2, "admin should expose than two tools")
	s.T().Logf("admin tools: %s", strings.Join(adminTools, ", "))
	s.T().Log("Admin routing working correctly")
}

func (s *testingSuite) TestDynamicMCPUserRouting() {
	s.waitDynamicReady()
	s.T().Log("Testing dynamic MCP routing for regular user")
	userTools := s.runDynamicRoutingCase("user-client", map[string]string{"user-type": "user"}, "user")
	// user should expose only one tool
	s.Require().Equal(len(userTools), 1, "user should expose exactly one tool")
	s.T().Logf("user tools: %s", strings.Join(userTools, ", "))
	s.T().Log("User routing working correctly")
}

func (s *testingSuite) TestDynamicMCPDefaultRouting() {
	s.waitDynamicReady()
	s.T().Log("Testing dynamic MCP routing with no header (default to user)")
	defTools := s.runDynamicRoutingCase("default-client", map[string]string{}, "default")
	// default uses user backend and should expose only one tool available
	s.Require().Equal(len(defTools), 1, "default/user should expose exactly one tool")
	s.T().Logf("default tools: %s", strings.Join(defTools, ", "))
	s.T().Log("Default routing working correctly")
}

// TestDynamicMCPAdminVsUserTools initializes two sessions (admin and user) against the same
// dynamic route and compares the exposed tool sets. This gives positive proof that
// header-based routing is sending traffic to distinct backends.
func (s *testingSuite) TestDynamicMCPAdminVsUserTools() {
	s.waitDynamicReady()
	s.T().Log("Comparing admin vs user tool sets on dynamic MCP route")

	// Execute admin and user cases via shared helper
	adminTools := s.runDynamicRoutingCase("compare-client", map[string]string{"user-type": "admin"}, "admin (compare)")
	userTools := s.runDynamicRoutingCase("compare-client", map[string]string{"user-type": "user"}, "user (compare)")

	// Compare sets; admin should be a superset or at least different.
	adminSet := make(map[string]struct{}, len(adminTools))
	for _, n := range adminTools {
		adminSet[n] = struct{}{}
	}
	same := len(adminTools) == len(userTools)
	if same {
		for _, n := range userTools {
			if _, ok := adminSet[n]; !ok {
				same = false
				break
			}
		}
	}
	if same {
		s.T().Logf("admin tools (%d found): %s", len(adminTools), strings.Join(adminTools, ", "))
		s.T().Logf("user tools (%d found): %s", len(userTools), strings.Join(userTools, ", "))
		s.Require().Fail("admin and user tool sets are identical; backend config should provide different tool sets")
	} else {
		s.T().Logf("admin tools (%d found): %s", len(adminTools), strings.Join(adminTools, ", "))
		s.T().Logf("user tools (%d found): %s", len(userTools), strings.Join(userTools, ", "))
	}
}

// runDynamicRoutingCase initializes a session with optional route headers, asserts
// initialize response correctness, warms the session, and returns the tool names.
func (s *testingSuite) runDynamicRoutingCase(clientName string, routeHeaders map[string]string, label string) []string {
	initBody := buildInitializeRequest(clientName, 0)
	headers := withRouteHeaders(mcpHeaders(nil), routeHeaders)

	// Deterministic 200 with retry/backoff
	s.waitForMCP200(8080, headers, initBody, label,
		100*time.Millisecond, 250*time.Millisecond, 500*time.Millisecond, 1*time.Second)

	// Get full response for logging + session extraction
	out, err := s.execCurlMCP(headers, initBody, "--max-time", "10")
	s.Require().NoError(err, "%s initialize failed", label)
	s.T().Logf("%s initialize: %s", label, out)

	sid := ExtractMCPSessionID(out)
	s.Require().NotEmpty(sid, "%s initialize must return mcp-session-id header", label)
	s.notifyInitializedWithHeaders(sid, routeHeaders)

	payload, ok := FirstSSEDataPayload(out)
	s.Require().True(ok, "%s initialize must return SSE payload", label)

	var initResp InitializeResponse
	s.Require().NoError(json.Unmarshal([]byte(payload), &initResp), "%s initialize payload must be JSON", label)
	s.Require().Nil(initResp.Error, "%s initialize returned error: %+v", label, initResp.Error)
	s.Require().NotNil(initResp.Result, "%s initialize missing result", label)

	// Update the global protocol version from the server response
	updateProtocolVersion(payload)

	// Now validate that the protocol version matches what we sent
	s.Require().Equal(mcpProto, initResp.Result.ProtocolVersion, "protocolVersion mismatch")
	s.Require().NotEmpty(initResp.Result.ServerInfo.Name, "serverInfo.name must be set")

	tools := s.mustListTools(sid, label+" tools/list", routeHeaders)
	return tools
}

func (s *testingSuite) waitDynamicReady() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(
		s.Ctx, "default",
		metav1.ListOptions{LabelSelector: "app=mcp-admin-server"},
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(
		s.Ctx, "default",
		metav1.ListOptions{LabelSelector: "app=mcp-website-fetcher"},
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(s.Ctx, gatewayName, gatewayNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwBackendCondition(s.Ctx, "admin-mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwBackendCondition(s.Ctx, "user-mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx, "dynamic-mcp-route", "default",
		gwv1.RouteConditionAccepted, metav1.ConditionTrue,
	)
}

func (s *testingSuite) waitStaticReady() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(
		s.Ctx, "default",
		metav1.ListOptions{LabelSelector: "app=mcp-website-fetcher"},
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(s.Ctx, gatewayName, gatewayNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwBackendCondition(s.Ctx, "mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(s.Ctx, "mcp-route", "default", gwv1.RouteConditionAccepted, metav1.ConditionTrue)
}

func (s *testingSuite) waitAuth0Ready() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(
		s.Ctx, "default",
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=auth0-mock"},
	)
}
