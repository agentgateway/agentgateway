//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const (
	// Ref: https://github.com/agentgateway/agentgateway/blob/0eff44a748b80030141ebe1d3626c780d05b0265/crates/agentgateway/src/llm/policy.rs#L502
	agwDefaultPromptGuardResponse = "The request was rejected due to inappropriate content"

	guardrailsWebhookBlockResponse = "request blocked"

	maskedPatternResponse = "****ing"
)

var (
	// manifests
	setupManifest            = manifest("aibackend", "setup.yaml")
	failoverEvictionManifest = manifest("aibackend", "failover_eviction.yaml")

	// test cases
	aiBackendSetup = base.TestCase{
		Manifests: []string{
			setupManifest,
		},
	}
)

func TestAIBackend(t *testing.T) {
	agw := New(t)
	agw.MinGwApiVersion = base.GwApiRequireRouteNames
	agw.ApplyConfig(aiBackendSetup)

	agw.Run("Routing", func() {
		testAIBackendRouting(agw)
	})
	agw.Run("PromptGuard", func() {
		testAIBackendPromptGuard(agw)
	})
	agw.Run("Webhook", func() {
		testAIBackendWebhook(agw)
	})
	agw.Run("Failover", func() {
		agw.ApplyConfig(base.TestCase{
			Manifests: []string{failoverEvictionManifest},
		})
		testAIBackendFailover(agw)
	})
}

func testAIBackendRouting(agw *base.BaseTestingSuite) {
	base.BaseGateway.Send(
		agw.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring(`The name of this project is agentgateway`),
		},
		curl.WithPath("/v1/chat/completions"),
		curl.WithPostBody(`{"messages": [{"role": "user", "content": "What is the name of this project?"}]}`),
		curl.WithHeader("Content-Type", "application/json"),
	)
}

func testAIBackendPromptGuard(agw *base.BaseTestingSuite) {
	// Test request guard
	base.BaseGateway.Send(
		agw.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusForbidden,
			Body:       gomega.ContainSubstring(`request rejected`),
		},
		curl.WithPath("/v1/chat/completions"),
		curl.WithPostBody(`{"messages": [{"role": "user", "content": "Return an example credit card number"}]}`),
		curl.WithHeader("Content-Type", "application/json"),
	)

	// Test response guard
	base.BaseGateway.Send(
		agw.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusForbidden,
			Body:       gomega.ContainSubstring(agwDefaultPromptGuardResponse),
		},
		curl.WithPath("/v1/chat/completions"),
		curl.WithPostBody(`{"messages": [{"role": "user", "content": "Return an example SSN"}]}`),
		curl.WithHeader("Content-Type", "application/json"),
	)
}

func testAIBackendWebhook(agw *base.BaseTestingSuite) {
	// Test request webhook
	base.BaseGateway.Send(
		agw.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusForbidden,
			Body:       gomega.ContainSubstring(guardrailsWebhookBlockResponse),
		},
		curl.WithPath("/v1/messages"),
		curl.WithPostBody(`{"messages": [{"role": "user", "content": "return blocked content"}]}`),
		curl.WithHeaders(map[string]string{
			"Content-Type": "application/json",
			"x-direction":  "request", // matches request webhook route
			// below headers are required due to https://github.com/agentgateway/agentgateway/issues/509
			"x-api-key":         "fake",
			"anthropic-version": "fake",
		}),
	)

	// Test response webhook
	base.BaseGateway.Send(
		agw.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring(maskedPatternResponse),
		},
		curl.WithPath("/v1/messages"),
		curl.WithPostBody(`{"messages": [{"role": "user", "content": "Explain data masking"}]}`),
		curl.WithHeaders(map[string]string{
			"Content-Type": "application/json",
			"x-direction":  "response", // matches response webhook route
			// below headers are required due to https://github.com/agentgateway/agentgateway/issues/509
			"x-api-key":         "fake",
			"anthropic-version": "fake",
		}),
	)
}

func testAIBackendFailover(agw *base.BaseTestingSuite) {
	expectedResponse := "The name of this project is agentgateway"

	// The failover backend has two groups:
	//   Priority 0 (primary): mock-llm-primary Service with replicas=0 (no endpoints → connection error)
	//   Priority 1 (fallback): shared testbox LLM server
	//
	// The health policy evicts the primary after 3 consecutive unhealthy responses (threshold: 3).
	// Send will retry until the primary is evicted and the fallback returns the expected response.
	base.BaseGateway.Send(
		agw.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring(expectedResponse),
			Headers:    map[string]any{"x-backend-model": "gpt-4o-mini", "x-model-swapped": "true"},
		},
		curl.WithPath("/v1/chat/completions"),
		curl.WithPostBody(`{"messages": [{"role": "user", "content": "What is the name of this project?"}]}`),
		curl.WithHeader("Content-Type", "application/json"),
		curl.WithHeader("x-test-failover", "1"),
	)
}
