//go:build e2e

package e2e_test

import (
	"encoding/base64"
	"net/http"
	"testing"

	"github.com/onsi/gomega"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const (
	modelRoutingGatewayName = "model-routing"
	modelRoutingNamespace   = "default"
)

var modelRoutingSetupManifest = manifest("modelrouting", "setup.yaml")

func TestAgentgatewayModelRouting(tt *testing.T) {
	t := New(tt, base.WithMinGwApiVersion(base.GwApiRequireRouteNames))
	t.Apply(modelRoutingSetupManifest)
	t.GatewayReady(modelRoutingGatewayName, modelRoutingNamespace)

	gw := modelRoutingGateway(t)

	t.Run("Status", func(t base.Test) {
		testAgentgatewayModelStatus(t)
	})
	t.Run("Visibility", func(t base.Test) {
		testAgentgatewayModelVisibility(t, gw)
	})
	t.Run("GatewayBoundModelsUseDefaultRoute", func(t base.Test) {
		testGatewayBoundModelsUseDefaultRoute(t, gw)
	})
	t.Run("WeightedVirtualModel", func(t base.Test) {
		testAgentgatewayModelWeightedRouting(t, gw)
	})
	t.Run("ConditionalVirtualModel", func(t base.Test) {
		testAgentgatewayModelConditionalRouting(t, gw)
	})
	t.Run("WildcardTargetTransformations", func(t base.Test) {
		testAgentgatewayModelWildcardTargetTransformations(t, gw)
	})
	t.Run("FailoverVirtualModel", func(t base.Test) {
		testAgentgatewayModelFailoverRouting(t, gw)
	})
	t.Run("FailoverVirtualModelAuthorization", func(t base.Test) {
		testAgentgatewayModelFailoverAuthorization(t, gw)
	})
	t.Run("PathScopedModelRoute", func(t base.Test) {
		testPathScopedModelRoute(t, gw)
	})
	t.Run("MultiplePathScopedModelRoutes", func(t base.Test) {
		testMultiplePathScopedModelRoutes(t, gw)
	})
}

func testAgentgatewayModelStatus(t base.Test) {
	assertions.EventuallyAgentgatewayModelCondition(t, "public-direct", modelRoutingNamespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)
	assertions.EventuallyAgentgatewayModelCondition(t, "public-direct", modelRoutingNamespace, gwv1.RouteConditionResolvedRefs, metav1.ConditionTrue)
	assertions.EventuallyAgentgatewayModelCondition(t, "invalid-target", modelRoutingNamespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)
	assertions.EventuallyAgentgatewayModelCondition(t, "invalid-target", modelRoutingNamespace, gwv1.RouteConditionResolvedRefs, metav1.ConditionFalse)
}

func testAgentgatewayModelVisibility(t base.Test, gw base.Gateway) {
	gw.Send(
		t,
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body: gomega.And(
				gomega.ContainSubstring(`"id":"public-direct"`),
				gomega.ContainSubstring(`"id":"weighted-fast"`),
				gomega.ContainSubstring(`"id":"smart"`),
				gomega.ContainSubstring(`"id":"resilient"`),
				gomega.Not(gomega.ContainSubstring(`"id":"internal-fast"`)),
				gomega.Not(gomega.ContainSubstring(`"id":"internal-premium"`)),
			),
		},
		curl.WithPath("/v1/models"),
	)

	gw.Send(
		t,
		&testmatchers.HttpResponse{
			StatusCode: http.StatusNotFound,
			Body:       gomega.ContainSubstring(`model_not_found`),
		},
		modelRoutingCompletion("internal-fast")...,
	)
}

// A model bound directly to a Gateway listener must work without an HTTPRoute.
// This is the default path for users who do not need path-scoped model sets.
func testGatewayBoundModelsUseDefaultRoute(t base.Test, gw base.Gateway) {
	gw.Send(
		t,
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring(`"id":"public-direct"`),
		},
		curl.WithPath("/v1/models"),
	)

	gw.Send(
		t,
		modelRoutingExpectModel("agw-public-direct"),
		modelRoutingCompletion("public-direct")...,
	)
}

func testAgentgatewayModelWeightedRouting(t base.Test, gw base.Gateway) {
	gw.Send(
		t,
		modelRoutingExpectModel("agw-internal-fast"),
		modelRoutingCompletion("weighted-fast")...,
	)
}

func testAgentgatewayModelConditionalRouting(t base.Test, gw base.Gateway) {
	gw.Send(
		t,
		modelRoutingExpectModel("agw-internal-premium"),
		append(
			modelRoutingCompletion("smart"),
			curl.WithHeader("x-model-tier", "premium"),
		)...,
	)
	gw.Send(
		t,
		modelRoutingExpectModel("agw-internal-fast"),
		modelRoutingCompletion("smart")...,
	)
}

func testAgentgatewayModelWildcardTargetTransformations(t base.Test, gw base.Gateway) {
	gw.Send(
		t,
		modelRoutingExpectModel("agw-wildcard-selected"),
		modelRoutingCompletion("wildcard-virtual")...,
	)
}

func testAgentgatewayModelFailoverRouting(t base.Test, gw base.Gateway) {
	gw.Send(
		t,
		modelRoutingExpectModel("agw-internal-fast"),
		modelRoutingCompletion("resilient")...,
	)
}

func testAgentgatewayModelFailoverAuthorization(t base.Test, gw base.Gateway) {
	gw.Send(
		t,
		base.ExpectForbidden(gomega.ContainSubstring("authorization failed")),
		modelRoutingCompletion("protected-resilient")...,
	)

	gw.Send(
		t,
		modelRoutingExpectModel("agw-protected-fallback"),
		append(
			modelRoutingCompletion("protected-resilient"),
			curl.WithHeader("x-model-access", "allowed"),
		)...,
	)
}

func testPathScopedModelRoute(t base.Test, gw base.Gateway) {
	t.HTTPRouteAccepted("scoped-models", modelRoutingNamespace)

	gw.Send(
		t,
		base.Expect(http.StatusUnauthorized),
		curl.WithPath("/scoped/v1/models"),
	)
	gw.Send(
		t,
		base.Expect(http.StatusUnauthorized),
		append(
			modelRoutingCompletion("scoped-model"),
			curl.WithPath("/scoped/v1/chat/completions"),
		)...,
	)

	gw.Send(
		t,
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body: gomega.And(
				gomega.ContainSubstring(`"id":"scoped-model"`),
				gomega.Not(gomega.ContainSubstring(`"id":"tenant-a-model"`)),
			),
		},
		curl.WithPath("/scoped/v1/models"),
		curl.WithHeader("Authorization", basicAuthHeader("scoped", "alicepassword")),
	)

	gw.Send(
		t,
		modelRoutingExpectModel("agw-scoped-model"),
		append(
			modelRoutingCompletion("scoped-model"),
			curl.WithPath("/scoped/v1/chat/completions"),
			curl.WithHeader("Authorization", basicAuthHeader("scoped", "alicepassword")),
		)...,
	)
}

func testMultiplePathScopedModelRoutes(t base.Test, gw base.Gateway) {
	t.HTTPRouteAccepted("tenant-a-models", modelRoutingNamespace)
	t.HTTPRouteAccepted("tenant-b-models", modelRoutingNamespace)

	for _, tt := range []struct {
		path          string
		user          string
		password      string
		model         string
		otherModel    string
		upstreamModel string
	}{
		{"/tenant-a", "tenant-a", "alicepassword", "tenant-a-model", "tenant-b-model", "agw-tenant-a-model"},
		{"/tenant-b", "tenant-b", "bobpassword", "tenant-b-model", "tenant-a-model", "agw-tenant-b-model"},
	} {
		t.Run(tt.model, func(t base.Test) {
			gw.Send(
				t,
				&testmatchers.HttpResponse{
					StatusCode: http.StatusOK,
					Body: gomega.And(
						gomega.ContainSubstring(`"id":"`+tt.model+`"`),
						gomega.Not(gomega.ContainSubstring(`"id":"`+tt.otherModel+`"`)),
					),
				},
				curl.WithPath(tt.path+"/v1/models"),
				curl.WithHeader("Authorization", basicAuthHeader(tt.user, tt.password)),
			)

			gw.Send(
				t,
				modelRoutingExpectModel(tt.upstreamModel),
				append(
					modelRoutingCompletion(tt.model),
					curl.WithPath(tt.path+"/v1/chat/completions"),
					curl.WithHeader("Authorization", basicAuthHeader(tt.user, tt.password)),
				)...,
			)
		})
	}

	gw.Send(
		t,
		base.Expect(http.StatusUnauthorized),
		curl.WithPath("/tenant-a/v1/models"),
		curl.WithHeader("Authorization", basicAuthHeader("tenant-b", "bobpassword")),
	)
}

func modelRoutingGateway(t base.Test) base.Gateway {
	name := types.NamespacedName{Name: modelRoutingGatewayName, Namespace: modelRoutingNamespace}
	return base.Gateway{
		NamespacedName: name,
		Address:        base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, name),
	}
}

func modelRoutingCompletion(model string) []curl.Option {
	return []curl.Option{
		curl.WithPath("/v1/chat/completions"),
		curl.WithPostBody(`{"model":"` + model + `","messages":[{"role":"user","content":"What is the name of this project?"}]}`),
		curl.WithHeader("Content-Type", "application/json"),
	}
}

func modelRoutingExpectModel(selectedModel string) *testmatchers.HttpResponse {
	return &testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		Body:       gomega.ContainSubstring(`The name of this project is agentgateway`),
		Headers: map[string]any{
			"x-agw-request-model":  selectedModel,
			"x-agw-response-model": "gpt-4o-mini",
		},
	}
}

func basicAuthHeader(username, password string) string {
	return "Basic " + base64.StdEncoding.EncodeToString([]byte(username+":"+password))
}
