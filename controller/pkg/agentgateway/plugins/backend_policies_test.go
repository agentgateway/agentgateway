package plugins_test

import (
	"testing"

	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/plugins"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/testutils"
)

func TestBackendPolicies(t *testing.T) {
	testutils.RunForDirectory(t, "testdata/backendpolicy", func(t *testing.T, ctx plugins.PolicyCtx) (*gwv1.PolicyStatus, []plugins.AgwPolicy) {
		pol := testutils.GetTestResource(t, ctx.Collections.AgentgatewayPolicies)
		s, o := plugins.TranslateAgentgatewayPolicy(ctx.Krt, pol, ctx.Collections)
		return s, o
	})
}
