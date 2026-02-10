//go:build e2e

package tests

import (
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/listenerset"
)

func ListenerSetSuiteRunner() e2e.SuiteRunner {
	suiteRunner := e2e.NewSuiteRunner(false)
	suiteRunner.Register("ListenerSet", listenerset.NewTestingSuite)
	return suiteRunner
}
