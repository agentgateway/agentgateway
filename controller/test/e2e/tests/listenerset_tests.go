//go:build e2e

package tests

import (
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/listenerset"
)

func ListenerSetSuiteRunner() e2e.SuiteRunner {
	suiteRunner := e2e.NewSuiteRunner(false)
	suiteRunner.Register("ListenerSet", listenerset.NewTestingSuite)
	return suiteRunner
}
