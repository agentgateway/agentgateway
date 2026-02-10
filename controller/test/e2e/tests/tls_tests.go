//go:build e2e

package tests

import (
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/tls"
)

func TLSSuiteRunner() e2e.SuiteRunner {
	tlsSuiteRunner := e2e.NewSuiteRunner(false)
	tlsSuiteRunner.Register("ControlPlaneTLS", tls.NewTestingSuite)
	return tlsSuiteRunner
}
