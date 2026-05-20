//go:build e2e

package e2e_test

import (
	"encoding/json"
	"net/http"
	"strings"
	"testing"

	"github.com/onsi/gomega"
	"istio.io/istio/pkg/test/util/assert"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/testutils/testjwt"
)

func TestMCP(tt *testing.T) {
	t := New(tt)

	t.Run("Authn", func(t base.Test) {
		s := &mcpSuite{Test: t}
		s.Apply(authnSetup...)
		s.TestMCPAuthn()
	})
	t.Run("AuthnRoute", func(t base.Test) {
		s := &mcpSuite{Test: t}
		s.Apply(authnRouteSetup...)
		s.TestMCPAuthnRoute()
	})
	t.Run("Workflow", func(t base.Test) {
		s := &mcpSuite{Test: t}
		s.Apply(staticSetup...)
		s.TestMCPWorkflow()
	})
	t.Run("SSEEndpoint", func(t base.Test) {
		s := &mcpSuite{Test: t}
		s.Apply(staticSetup...)
		s.TestSSEEndpoint()
	})
	t.Run("DynamicAdminRouting", func(t base.Test) {
		s := &mcpSuite{Test: t}
		s.Apply(dynamicSetup...)
		s.TestDynamicMCPAdminRouting()
	})
	t.Run("DynamicUserRouting", func(t base.Test) {
		s := &mcpSuite{Test: t}
		s.Apply(dynamicSetup...)
		s.TestDynamicMCPUserRouting()
	})
	t.Run("DynamicDefaultRouting", func(t base.Test) {
		s := &mcpSuite{Test: t}
		s.Apply(dynamicSetup...)
		s.TestDynamicMCPDefaultRouting()
	})
	t.Run("DynamicAdminVsUserTools", func(t base.Test) {
		s := &mcpSuite{Test: t}
		s.Apply(dynamicSetup...)
		s.TestDynamicMCPAdminVsUserTools()
	})
}

func (s *mcpSuite) TestMCPAuthn() {
	// Single test that does the full workflow with session management
	s.Log("Testing complete MCP workflow with session management")

	// Ensure static components are ready
	s.waitStaticReady()
	// Ensure auth0 server is ready
	s.waitAuth0Ready()

	// Wait for the authentication policy to be accepted before testing
	s.Log("Waiting for authentication policy to be accepted")
	s.TestInstallation.AssertionsT(s).EventuallyAgwPolicyCondition(
		s.Ctx,
		"auth0-mcp-authn-policy",
		"default",
		"Accepted",
		metav1.ConditionTrue,
	)

	validAuthnHeader := map[string]string{"Authorization": "Bearer " + testjwt.OrgOneJWT}

	// Verify authentication is actually enforced (not just policy accepted)
	// by waiting for an unauthenticated request to return 401
	s.Log("Verifying authentication is enforced")
	s.waitForAuthnEnforced()

	// Test 1: Initialize without token should fail
	s.Log("Test 1: Initialize without Authorization header should return 401")
	s.testInitializeWithExpectedStatus(nil, 401, "missing token")

	// Test 2: Initialize with invalid token should fail
	s.Log("Test 2: Initialize with invalid token should return 401")
	invalidAuthnHeader := map[string]string{"Authorization": "Bearer " + "fake"}
	s.testInitializeWithExpectedStatus(invalidAuthnHeader, 401, "invalid token")

	// Test 3: Initialize with valid token should succeed
	s.Log("Test 3: Initialize with valid token should succeed")
	sessionID := s.initializeAndGetSessionID(validAuthnHeader)
	if sessionID == "" {
		s.Fatal("Failed to get session ID from initialize")
	}

	// Test 4: tools/list with valid token should succeed
	s.Log("Test 4: tools/list with valid token should succeed")
	s.testToolsListWithSession(sessionID, validAuthnHeader)

	// Test 5: tools/list with invalid token should fail
	s.Log("Test 5: tools/list with invalid token should fail")
	s.testUnauthorizedToolsListWithSession(sessionID, invalidAuthnHeader, 401)

	// Test 6: tools/list with missing token should fail
	s.Log("Test 6: tools/list with missing token should fail")
	s.testUnauthorizedToolsListWithSession(sessionID, nil, 401)
}

func (s *mcpSuite) TestMCPAuthnRoute() {
	// Single test that does the full workflow with session management
	s.Log("Testing complete MCP workflow with session management")

	// Ensure static components are ready
	s.waitStaticReady()
	// Ensure auth0 server is ready
	s.waitAuth0Ready()

	// Wait for the authentication policy to be accepted before testing
	s.Log("Waiting for authentication policy to be accepted")
	s.TestInstallation.AssertionsT(s).EventuallyAgwPolicyCondition(
		s.Ctx,
		"auth0-mcp-authn-policy",
		"default",
		"Accepted",
		metav1.ConditionTrue,
	)

	validAuthnHeader := map[string]string{"Authorization": "Bearer " + testjwt.OrgOneJWT}

	// Verify authentication is actually enforced (not just policy accepted)
	// by waiting for an unauthenticated request to return 401
	s.Log("Verifying authentication is enforced")
	s.waitForAuthnEnforced()

	// Test 1: Initialize without token should fail
	s.Log("Test 1: Initialize without Authorization header should return 401")
	s.testInitializeWithExpectedStatus(nil, 401, "missing token")

	// Test 2: Initialize with invalid token should fail
	s.Log("Test 2: Initialize with invalid token should return 401")
	invalidAuthnHeader := map[string]string{"Authorization": "Bearer " + "fake"}
	s.testInitializeWithExpectedStatus(invalidAuthnHeader, 401, "invalid token")

	// Test 3: Initialize with valid token should succeed
	s.Log("Test 3: Initialize with valid token should succeed")
	sessionID := s.initializeAndGetSessionID(validAuthnHeader)
	if sessionID == "" {
		s.Fatal("Failed to get session ID from initialize")
	}

	// Test 4: tools/list with valid token should succeed
	s.Log("Test 4: tools/list with valid token should succeed")
	s.testToolsListWithSession(sessionID, validAuthnHeader)

	// Test 5: tools/list with invalid token should fail
	s.Log("Test 5: tools/list with invalid token should fail")
	s.testUnauthorizedToolsListWithSession(sessionID, invalidAuthnHeader, 401)

	// Test 6: tools/list with missing token should fail
	s.Log("Test 6: tools/list with missing token should fail")
	s.testUnauthorizedToolsListWithSession(sessionID, nil, 401)
}

func (s *mcpSuite) TestMCPWorkflow() {
	// Single test that does the full workflow with session management
	s.Log("Testing complete MCP workflow with session management")

	// Ensure static components are ready
	s.waitStaticReady()

	// Step 1: Initialize and get session ID
	sessionID := s.initializeAndGetSessionID(nil)
	if sessionID == "" {
		s.Fatal("Failed to get session ID from initialize")
	}

	// Step 2: Test tools/list with session ID
	s.testToolsListWithSession(sessionID, nil)
}

func (s *mcpSuite) TestSSEEndpoint() {
	// Ensure static components are ready
	s.waitStaticReady()

	initBody := buildInitializeRequest("sse-client", 0)
	headers := mcpHeaders(nil)

	s.sendMCP(&testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		Headers: map[string]any{
			"Content-Type": gomega.MatchRegexp(`^text/event-stream(?:\s*;.*)?$`),
		},
	}, headers, initBody)

	_ = s.initializeSession(initBody, headers, "sse")
}

func (s *mcpSuite) TestDynamicMCPAdminRouting() {
	s.waitDynamicReady()
	s.Log("Testing dynamic MCP routing for admin user")
	adminTools := s.runDynamicRoutingCase("admin-client", map[string]string{"user-type": "admin"}, "admin")
	// Admin will have more than two tools
	if len(adminTools) < 2 {
		s.Fatalf("admin should expose at least two tools, got %d", len(adminTools))
	}
	s.Logf("admin tools: %s", strings.Join(adminTools, ", "))
	s.Log("Admin routing working correctly")
}

func (s *mcpSuite) TestDynamicMCPUserRouting() {
	s.waitDynamicReady()
	s.Log("Testing dynamic MCP routing for regular user")
	userTools := s.runDynamicRoutingCase("user-client", map[string]string{"user-type": "user"}, "user")
	// user should expose only one tool
	assert.Equal(s, len(userTools), 1, "user should expose exactly one tool")
	s.Logf("user tools: %s", strings.Join(userTools, ", "))
	s.Log("User routing working correctly")
}

func (s *mcpSuite) TestDynamicMCPDefaultRouting() {
	s.waitDynamicReady()
	s.Log("Testing dynamic MCP routing with no header (default to user)")
	defTools := s.runDynamicRoutingCase("default-client", map[string]string{}, "default")
	// default uses user backend and should expose only one tool available
	assert.Equal(s, len(defTools), 1, "default/user should expose exactly one tool")
	s.Logf("default tools: %s", strings.Join(defTools, ", "))
	s.Log("Default routing working correctly")
}

// TestDynamicMCPAdminVsUserTools initializes two sessions (admin and user) against the same
// dynamic route and compares the exposed tool sets. This gives positive proof that
// header-based routing is sending traffic to distinct backends.
func (s *mcpSuite) TestDynamicMCPAdminVsUserTools() {
	s.waitDynamicReady()
	s.Log("Comparing admin vs user tool sets on dynamic MCP route")

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
		s.Logf("admin tools (%d found): %s", len(adminTools), strings.Join(adminTools, ", "))
		s.Logf("user tools (%d found): %s", len(userTools), strings.Join(userTools, ", "))
		s.Fatal("admin and user tool sets are identical; backend config should provide different tool sets")
	} else {
		s.Logf("admin tools (%d found): %s", len(adminTools), strings.Join(adminTools, ", "))
		s.Logf("user tools (%d found): %s", len(userTools), strings.Join(userTools, ", "))
	}
}

// runDynamicRoutingCase initializes a session with optional route headers, asserts
// initialize response correctness, warms the session, and returns the tool names.
func (s *mcpSuite) runDynamicRoutingCase(clientName string, routeHeaders map[string]string, label string) []string {
	initBody := buildInitializeRequest(clientName, 0)
	headers := withRouteHeaders(mcpHeaders(nil), routeHeaders)

	// Deterministic 200 with retry/backoff
	s.waitForMCP200(headers, initBody, label)

	// Get full response for logging + session extraction
	// nolint: bodyclose // false positive
	resp, body, err := s.execCurlMCP(headers, initBody)
	if err != nil {
		s.Fatalf("%s initialize failed: %v", label, err)
	}
	s.Logf("%s initialize body: %s", label, body)

	sid := ExtractMCPSessionID(resp)
	if sid == "" {
		s.Fatalf("%s initialize must return mcp-session-id header", label)
	}
	s.notifyInitializedWithHeaders(sid, routeHeaders)

	payload, ok := FirstSSEDataPayload(body)
	if !ok {
		s.Fatalf("%s initialize must return SSE payload", label)
	}

	var initResp InitializeResponse
	if err := json.Unmarshal([]byte(payload), &initResp); err != nil {
		s.Fatalf("%s initialize payload must be JSON: %v", label, err)
	}
	if initResp.Error != nil {
		s.Fatalf("%s initialize returned error: %+v", label, initResp.Error)
	}
	if initResp.Result == nil {
		s.Fatalf("%s initialize missing result", label)
	}

	// Update the global protocol version from the server response
	updateProtocolVersion(payload)

	// Now validate that the protocol version matches what we sent
	assert.Equal(s, mcpProto, initResp.Result.ProtocolVersion, "protocolVersion mismatch")
	if initResp.Result.ServerInfo.Name == "" {
		s.Fatal("serverInfo.name must be set")
	}

	tools := s.mustListTools(sid, label+" tools/list", routeHeaders)
	return tools
}

func (s *mcpSuite) waitDynamicReady() {
	s.TestInstallation.AssertionsT(s).EventuallyPodsRunning(
		s.Ctx, "default",
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=testbox"},
	)
<<<<<<< HEAD
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(s.Ctx, gatewayName, gatewayNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAllAccepted(s.Ctx, []client.Object{
		&agentgateway.AgentgatewayBackend{ObjectMeta: metav1.ObjectMeta{Name: "admin-mcp-backend", Namespace: "default"}},
		&agentgateway.AgentgatewayBackend{ObjectMeta: metav1.ObjectMeta{Name: "user-mcp-backend", Namespace: "default"}},
		&gwv1.HTTPRoute{ObjectMeta: metav1.ObjectMeta{Name: "dynamic-mcp-route", Namespace: "default"}},
	})
=======
	s.TestInstallation.AssertionsT(s).EventuallyGatewayCondition(s.Ctx, gatewayName, gatewayNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s).EventuallyAgwBackendCondition(s.Ctx, "admin-mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s).EventuallyAgwBackendCondition(s.Ctx, "user-mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s).EventuallyHTTPRouteCondition(
		s.Ctx, "dynamic-mcp-route", "default",
		gwv1.RouteConditionAccepted, metav1.ConditionTrue,
	)
>>>>>>> 29c781ee8 (More cleanup)
}

func (s *mcpSuite) waitStaticReady() {
	s.TestInstallation.AssertionsT(s).EventuallyPodsRunning(
		s.Ctx, "default",
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=testbox"},
	)
<<<<<<< HEAD
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(s.Ctx, gatewayName, gatewayNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAllAccepted(s.Ctx, []client.Object{
		&agentgateway.AgentgatewayBackend{ObjectMeta: metav1.ObjectMeta{Name: "mcp-backend", Namespace: "default"}},
		&gwv1.HTTPRoute{ObjectMeta: metav1.ObjectMeta{Name: "mcp-route", Namespace: "default"}},
	})
=======
	s.TestInstallation.AssertionsT(s).EventuallyGatewayCondition(s.Ctx, gatewayName, gatewayNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s).EventuallyAgwBackendCondition(s.Ctx, "mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	s.TestInstallation.AssertionsT(s).EventuallyHTTPRouteCondition(s.Ctx, "mcp-route", "default", gwv1.RouteConditionAccepted, metav1.ConditionTrue)
>>>>>>> 29c781ee8 (More cleanup)
}

func (s *mcpSuite) waitAuth0Ready() {
	s.TestInstallation.AssertionsT(s).EventuallyPodsRunning(
		s.Ctx, "default",
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=testbox"},
	)
}
