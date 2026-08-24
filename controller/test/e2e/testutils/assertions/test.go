//go:build e2e

package assertions

import (
	"context"

	"istio.io/istio/pkg/test"
	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/cluster"
)

type Test interface {
	test.Failer
	E2EContext() context.Context
	E2EClusterContext() *cluster.Context
}

// AGWTest exposes Agentgateway resource GVKs to test helpers.
type AGWTest interface {
	Test
	AgentgatewayBackendGVK() schema.GroupVersionKind
	AgentgatewayPolicyGVK() schema.GroupVersionKind
}
