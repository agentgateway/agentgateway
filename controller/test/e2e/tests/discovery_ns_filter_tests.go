//go:build e2e

package tests

import (
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/agentgateway/discoverynsfilter"
)

func DiscoveryNSFilterSuiteRunner() e2e.SuiteRunner {
	runner := e2e.NewSuiteRunner(false)
	runner.Register("DiscoveryNamespaceFilter", discoverynsfilter.NewTestingSuite)
	return runner
}
