//go:build e2e

package tests

import (
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/tls"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/tlsplaintext"
)

func TLSSuiteRunner() e2e.SuiteRunner {
	tlsSuiteRunner := e2e.NewSuiteRunner(false)
	tlsSuiteRunner.Register("ControlPlaneTLS", tls.NewTestingSuite)
	return tlsSuiteRunner
}

func TLSPlaintextSuiteRunner() e2e.SuiteRunner {
	tlsPlaintextSuiteRunner := e2e.NewSuiteRunner(false)
	tlsPlaintextSuiteRunner.Register("ControlPlaneTLSPlaintext", tlsplaintext.NewTestingSuite)
	return tlsPlaintextSuiteRunner
}
