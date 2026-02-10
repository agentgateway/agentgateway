//go:build e2e

package tests

import (
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/tls"
)

func TLSSuiteRunner() e2e.SuiteRunner {
	tlsSuiteRunner := e2e.NewSuiteRunner(false)
	tlsSuiteRunner.Register("ControlPlaneTLS", tls.NewTestingSuite)
	return tlsSuiteRunner
}
