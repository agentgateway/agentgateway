//go:build e2e

package e2e_test

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/onsi/gomega"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/testutils/testjwt"
)

func TestMCP(tt *testing.T) {
	t := New(tt)

	t.Run("Authn", func(t base.Test) {
		t.Apply(authnSetup...)
		testMCPAuthn(t)
	})
	t.Run("AuthnRoute", func(t base.Test) {
		t.Apply(authnRouteSetup...)
		testMCPAuthnRoute(t)
	})
	t.Run("Workflow", func(t base.Test) {
		t.Apply(staticSetup...)
		testMCPWorkflow(t)
	})
	t.Run("SSEEndpoint", func(t base.Test) {
		t.Apply(staticSetup...)
		testSSEEndpoint(t)
	})
	t.Run("DynamicAdminRouting", func(t base.Test) {
		t.Apply(dynamicSetup...)
		testDynamicMCPAdminRouting(t)
	})
	t.Run("DynamicUserRouting", func(t base.Test) {
		t.Apply(dynamicSetup...)
		testDynamicMCPUserRouting(t)
	})
	t.Run("DynamicDefaultRouting", func(t base.Test) {
		t.Apply(dynamicSetup...)
		testDynamicMCPDefaultRouting(t)
	})
	t.Run("DynamicAdminVsUserTools", func(t base.Test) {
		t.Apply(dynamicSetup...)
		testDynamicMCPAdminVsUserTools(t)
	})
}

func testMCPAuthn(t base.Test) {
	// Single test that does the full workflow with session management
	t.Log("Testing complete MCP workflow with session management")

	// Ensure static components are ready
	waitStaticReady(t)
	// Ensure auth0 server is ready
	waitAuth0Ready(t)

	// Wait for the authentication policy to be accepted before testing
	t.Log("Waiting for authentication policy to be accepted")
	assertions.EventuallyAgwPolicyCondition(t,
		"auth0-mcp-authn-policy",
		"default",
		"Accepted",
		metav1.ConditionTrue,
	)

	validAuthnHeader := map[string]string{"Authorization": "Bearer " + testjwt.OrgOneJWT}

	// Verify authentication is actually enforced (not just policy accepted)
	// by waiting for an unauthenticated request to return 401
	t.Log("Verifying authentication is enforced")
	waitForAuthnEnforced(t)

	// Test 1: Initialize without token should fail
	t.Log("Test 1: Initialize without Authorization header should return 401")
	testInitializeWithExpectedStatus(t, nil, 401, "missing token")

	// Test 2: Initialize with invalid token should fail
	t.Log("Test 2: Initialize with invalid token should return 401")
	invalidAuthnHeader := map[string]string{"Authorization": "Bearer " + "fake"}
	testInitializeWithExpectedStatus(t, invalidAuthnHeader, 401, "invalid token")

	// Test 3: Initialize with valid token should succeed
	t.Log("Test 3: Initialize with valid token should succeed")
	sessionID := initializeAndGetSessionID(t, validAuthnHeader)
	if sessionID == "" {
		t.Fatal("Failed to get session ID from initialize")
	}

	// Test 4: tools/list with valid token should succeed
	t.Log("Test 4: tools/list with valid token should succeed")
	testToolsListWithSession(t, sessionID, validAuthnHeader)

	// Test 5: tools/list with invalid token should fail
	t.Log("Test 5: tools/list with invalid token should fail")
	testUnauthorizedToolsListWithSession(t, sessionID, invalidAuthnHeader, 401)

	// Test 6: tools/list with missing token should fail
	t.Log("Test 6: tools/list with missing token should fail")
	testUnauthorizedToolsListWithSession(t, sessionID, nil, 401)
}

func testMCPAuthnRoute(t base.Test) {
	// Single test that does the full workflow with session management
	t.Log("Testing complete MCP workflow with session management")

	// Ensure static components are ready
	waitStaticReady(t)
	// Ensure auth0 server is ready
	waitAuth0Ready(t)

	// Wait for the authentication policy to be accepted before testing
	t.Log("Waiting for authentication policy to be accepted")
	assertions.EventuallyAgwPolicyCondition(t,
		"auth0-mcp-authn-policy",
		"default",
		"Accepted",
		metav1.ConditionTrue,
	)

	validAuthnHeader := map[string]string{"Authorization": "Bearer " + testjwt.OrgOneJWT}

	// Verify authentication is actually enforced (not just policy accepted)
	// by waiting for an unauthenticated request to return 401
	t.Log("Verifying authentication is enforced")
	waitForAuthnEnforced(t)

	// Test 1: Initialize without token should fail
	t.Log("Test 1: Initialize without Authorization header should return 401")
	testInitializeWithExpectedStatus(t, nil, 401, "missing token")

	// Test 2: Initialize with invalid token should fail
	t.Log("Test 2: Initialize with invalid token should return 401")
	invalidAuthnHeader := map[string]string{"Authorization": "Bearer " + "fake"}
	testInitializeWithExpectedStatus(t, invalidAuthnHeader, 401, "invalid token")

	// Test 3: Initialize with valid token should succeed
	t.Log("Test 3: Initialize with valid token should succeed")
	sessionID := initializeAndGetSessionID(t, validAuthnHeader)
	if sessionID == "" {
		t.Fatal("Failed to get session ID from initialize")
	}

	// Test 4: tools/list with valid token should succeed
	t.Log("Test 4: tools/list with valid token should succeed")
	testToolsListWithSession(t, sessionID, validAuthnHeader)

	// Test 5: tools/list with invalid token should fail
	t.Log("Test 5: tools/list with invalid token should fail")
	testUnauthorizedToolsListWithSession(t, sessionID, invalidAuthnHeader, 401)

	// Test 6: tools/list with missing token should fail
	t.Log("Test 6: tools/list with missing token should fail")
	testUnauthorizedToolsListWithSession(t, sessionID, nil, 401)
}

func testMCPWorkflow(t base.Test) {
	// Single test that does the full workflow with session management
	t.Log("Testing complete MCP workflow with session management")

	// Ensure static components are ready
	waitStaticReady(t)

	// Step 1: Initialize and get session ID
	sessionID := initializeAndGetSessionID(t, nil)
	if sessionID == "" {
		t.Fatal("Failed to get session ID from initialize")
	}

	// Step 2: Test tools/list with session ID
	testToolsListWithSession(t, sessionID, nil)
}

func testSSEEndpoint(t base.Test) {
	// Ensure static components are ready
	waitStaticReady(t)

	initBody := buildInitializeRequest("sse-client", 0)
	headers := mcpHeaders(nil)

	sendMCP(t, &testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		Headers: map[string]any{
			"Content-Type": gomega.MatchRegexp(`^text/event-stream(?:\s*;.*)?$`),
		},
	}, headers, initBody)

	_ = initializeSession(t, initBody, headers, "sse")
}

func testDynamicMCPAdminRouting(t base.Test) {
	waitDynamicReady(t)
	t.Log("Testing dynamic MCP routing for admin user")
	adminTools := runDynamicRoutingCase(t, "admin-client", map[string]string{"user-type": "admin"}, "admin")
	// Admin will have more than two tools
	if len(adminTools) < 2 {
		t.Fatalf("admin should expose at least two tools, got %d", len(adminTools))
	}
	t.Logf("admin tools: %s", strings.Join(adminTools, ", "))
	t.Log("Admin routing working correctly")
}

func testDynamicMCPUserRouting(t base.Test) {
	waitDynamicReady(t)
	t.Log("Testing dynamic MCP routing for regular user")
	userTools := runDynamicRoutingCase(t, "user-client", map[string]string{"user-type": "user"}, "user")
	// user should expose only one tool
	assert.Equal(t, len(userTools), 1, "user should expose exactly one tool")
	t.Logf("user tools: %s", strings.Join(userTools, ", "))
	t.Log("User routing working correctly")
}

func testDynamicMCPDefaultRouting(t base.Test) {
	waitDynamicReady(t)
	t.Log("Testing dynamic MCP routing with no header (default to user)")
	defTools := runDynamicRoutingCase(t, "default-client", map[string]string{}, "default")
	// default uses user backend and should expose only one tool available
	assert.Equal(t, len(defTools), 1, "default/user should expose exactly one tool")
	t.Logf("default tools: %s", strings.Join(defTools, ", "))
	t.Log("Default routing working correctly")
}

// TestDynamicMCPAdminVsUserTools initializes two sessions (admin and user) against the same
// dynamic route and compares the exposed tool sets. This gives positive proof that
// header-based routing is sending traffic to distinct backends.
func testDynamicMCPAdminVsUserTools(t base.Test) {
	waitDynamicReady(t)
	t.Log("Comparing admin vs user tool sets on dynamic MCP route")

	// Execute admin and user cases via shared helper
	adminTools := runDynamicRoutingCase(t, "compare-client", map[string]string{"user-type": "admin"}, "admin (compare)")
	userTools := runDynamicRoutingCase(t, "compare-client", map[string]string{"user-type": "user"}, "user (compare)")

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
		t.Logf("admin tools (%d found): %s", len(adminTools), strings.Join(adminTools, ", "))
		t.Logf("user tools (%d found): %s", len(userTools), strings.Join(userTools, ", "))
		t.Fatal("admin and user tool sets are identical; backend config should provide different tool sets")
	} else {
		t.Logf("admin tools (%d found): %s", len(adminTools), strings.Join(adminTools, ", "))
		t.Logf("user tools (%d found): %s", len(userTools), strings.Join(userTools, ", "))
	}
}

// runDynamicRoutingCase initializes a session with optional route headers, asserts
// initialize response correctness, warms the session, and returns the tool names.
func runDynamicRoutingCase(t base.Test, clientName string, routeHeaders map[string]string, label string) []string {
	initBody := buildInitializeRequest(clientName, 0)
	headers := withRouteHeaders(mcpHeaders(nil), routeHeaders)

	// nolint: bodyclose // false positive
	resp, body, payload, initResp := waitForDynamicInitialize(t, headers, initBody, routeHeaders, label)
	t.Logf("%s initialize body: %s", label, body)

	sid := ExtractMCPSessionID(resp)
	if sid == "" {
		t.Fatalf("%s initialize must return mcp-session-id header", label)
	}
	notifyInitializedWithHeaders(t, sid, routeHeaders)

	if initResp.Error != nil {
		t.Fatalf("%s initialize returned error: %+v", label, initResp.Error)
	}
	if initResp.Result == nil {
		t.Fatalf("%s initialize missing result", label)
	}

	// Update the global protocol version from the server response
	updateProtocolVersion(payload)

	// Now validate that the protocol version matches what we sent
	assert.Equal(t, mcpProto, initResp.Result.ProtocolVersion, "protocolVersion mismatch")
	if initResp.Result.ServerInfo.Name == "" {
		t.Fatal("serverInfo.name must be set")
	}

	tools := mustListTools(t, sid, label+" tools/list", routeHeaders)
	return tools
}

func waitForDynamicInitialize(
	t base.Test,
	headers map[string]string,
	initBody string,
	routeHeaders map[string]string,
	label string,
) (*http.Response, string, string, InitializeResponse) {
	t.Helper()

	var (
		resp     *http.Response
		body     string
		payload  string
		initResp InitializeResponse
	)
	expectedServer := expectedDynamicServer(routeHeaders)

	retry.UntilSuccessOrFail(t, func() error {
		// nolint: bodyclose // execCurlMCP closes the response body.
		gotResp, gotBody, err := execCurlMCP(t, headers, initBody)
		if err != nil {
			return fmt.Errorf("%s initialize failed: %w", label, err)
		}
		if gotResp.StatusCode != httpOKCode {
			return fmt.Errorf("%s initialize status=%d, want %d", label, gotResp.StatusCode, httpOKCode)
		}

		gotPayload, ok := FirstSSEDataPayload(gotBody)
		if !ok {
			return fmt.Errorf("%s initialize must return SSE payload", label)
		}

		var gotInitResp InitializeResponse
		if err := json.Unmarshal([]byte(gotPayload), &gotInitResp); err != nil {
			return fmt.Errorf("%s initialize payload must be JSON: %w", label, err)
		}
		if gotInitResp.Error != nil {
			return fmt.Errorf("%s initialize returned error: %+v", label, gotInitResp.Error)
		}
		if gotInitResp.Result == nil {
			return fmt.Errorf("%s initialize missing result", label)
		}
		if expectedServer != "" && gotInitResp.Result.ServerInfo.Name != expectedServer {
			return fmt.Errorf("%s dynamic route reached %q, want %q", label, gotInitResp.Result.ServerInfo.Name, expectedServer)
		}
		if ExtractMCPSessionID(gotResp) == "" {
			return fmt.Errorf("%s initialize must return mcp-session-id header", label)
		}

		resp = gotResp
		body = gotBody
		payload = gotPayload
		initResp = gotInitResp
		return nil
	}, retry.Timeout(10*time.Second), retry.Delay(100*time.Millisecond), retry.Message(fmt.Sprintf("%s initialize should reach expected backend", label)))

	return resp, body, payload, initResp
}

func expectedDynamicServer(routeHeaders map[string]string) string {
	if routeHeaders["user-type"] == "admin" {
		return "mcp-admin-server"
	}
	return "mcp-website-fetcher"
}

func waitDynamicReady(t base.Test) {
	assertions.EventuallyPodsRunning(t, "default",
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=testbox"},
	)
	assertions.EventuallyGatewayCondition(t, gatewayName, gatewayNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	assertions.EventuallyAgwBackendCondition(t, "admin-mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	assertions.EventuallyAgwBackendCondition(t, "user-mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	assertions.EventuallyHTTPRouteCondition(t, "dynamic-mcp-route", "default", gwv1.RouteConditionAccepted, metav1.ConditionTrue)
}

func waitStaticReady(t base.Test) {
	assertions.EventuallyPodsRunning(t, "default",
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=testbox"},
	)
	assertions.EventuallyGatewayCondition(t, gatewayName, gatewayNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	assertions.EventuallyAgwBackendCondition(t, "mcp-backend", "default", "Accepted", metav1.ConditionTrue)
	assertions.EventuallyHTTPRouteCondition(t, "mcp-route", "default", gwv1.RouteConditionAccepted, metav1.ConditionTrue)
}

func waitAuth0Ready(t base.Test) {
	assertions.EventuallyPodsRunning(t, "default",
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=testbox"},
	)
}
