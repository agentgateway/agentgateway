//go:build e2e

package tests

import (
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/metrics"
)

func KGatewayMetricsSuiteRunner() e2e.SuiteRunner {
	metricsSuiteRunner := e2e.NewSuiteRunner(false)

	metricsSuiteRunner.Register("Metrics", metrics.NewTestingSuite)

	return metricsSuiteRunner
}
