//go:build e2e

package tests

import (
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/metrics"
)

func KGatewayMetricsSuiteRunner() e2e.SuiteRunner {
	metricsSuiteRunner := e2e.NewSuiteRunner(false)

	metricsSuiteRunner.Register("Metrics", metrics.NewTestingSuite)

	return metricsSuiteRunner
}
