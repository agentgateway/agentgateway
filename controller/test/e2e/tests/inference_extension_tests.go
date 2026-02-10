//go:build e2e

package tests

import (
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/inferenceextension"
)

func InferenceExtensionSuiteRunner() e2e.SuiteRunner {
	infExtSuiteRunner := e2e.NewSuiteRunner(false)

	infExtSuiteRunner.Register("InferenceExtension", inferenceextension.NewTestingSuite)
	return infExtSuiteRunner
}
