//go:build e2e

package assertions

import (
	"context"

	"github.com/onsi/gomega"
	"istio.io/istio/pkg/test"

	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/cluster"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/install"
)

type Test interface {
	test.Failer
	E2EContext() context.Context
	E2EClusterContext() *cluster.Context
	E2EInstallContext() *install.Context
}

type provider struct {
	t test.Failer

	Gomega gomega.Gomega

	clusterContext *cluster.Context
	installContext *install.Context

	conditionHandlers []ConditionHandler
}

func newProvider(t test.Failer, clusterContext *cluster.Context, installContext *install.Context) *provider {
	return &provider{
		t:              t,
		Gomega:         gomega.NewWithT(t),
		clusterContext: clusterContext,
		installContext: installContext,
	}
}

func providerFor(t Test) *provider {
	return newProvider(t, t.E2EClusterContext(), t.E2EInstallContext())
}

func (p *provider) expectInstallContextDefined() {
	p.t.Helper()
	if p.installContext == nil {
		p.t.Fatal("assertion requires an agentgateway installation, but none was configured")
	}
}
