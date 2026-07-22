//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
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

	t.Run("Visibility", func(t base.Test) {
		testAgentgatewayModelVisibility(t, gw)
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
