//go:build e2e

package assertions

import (
	"context"

	"istio.io/istio/pkg/test"

	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/cluster"
)

type Test interface {
	test.Failer
	E2EContext() context.Context
	E2EClusterContext() *cluster.Context
}
