package plugins_test

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/slices"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
)

func TestTrafficPolicies(t *testing.T) {
	policyTest(t, "testdata/trafficpolicy")
}

func TestBackendPolicies(t *testing.T) {
	policyTest(t, "testdata/backendpolicy")
}

func TestFrontendPolicies(t *testing.T) {
	policyTest(t, "testdata/frontendpolicy")
}

func policyTest(t *testing.T, folder string) {
	t.Helper()
	testutils.RunForDirectory(t, folder, func(t *testing.T, ctx plugins.PolicyCtx) (any, []ir.AgwResource) {
		sq, ri := testutils.Syncer(t, ctx, "AgentgatewayPolicy")
		r := ri.Outputs.Resources.List()
		r = slices.FilterInPlace(r, func(resource ir.AgwResource) bool {
			x := ir.GetAgwResourceName(resource.Resource)
			return strings.HasPrefix(x, "policy/")
		})
		return sq.Dump(), slices.SortBy(r, func(a ir.AgwResource) string {
			return a.ResourceName()
		})
	})
}
