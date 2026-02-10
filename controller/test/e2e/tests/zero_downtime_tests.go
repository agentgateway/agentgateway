//go:build e2e

package tests

import (
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/zero_downtime_rollout"
)

func ZeroDowntimeRolloutAgentgatewaySuiteRunner() e2e.SuiteRunner {
	zeroDowntimeSuiteRunner := e2e.NewSuiteRunner(false)
	zeroDowntimeSuiteRunner.Register("ZeroDowntimeRolloutAgentgateway", zero_downtime_rollout.NewTestingSuiteAgentgateway)
	return zeroDowntimeSuiteRunner
}
