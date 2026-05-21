//go:build e2e

package tests_test

import (
	"context"
	"os"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/envutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	. "github.com/agentgateway/agentgateway/controller/test/e2e/tests"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/install"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

// TestControlPlaneTLSPlaintext tests the plaintext control plane integration functionality.
func TestControlPlaneTLSPlaintext(t *testing.T) {
	cleanupCtx := context.Background()
	installNs, nsEnvPredefined := envutils.LookupOrDefault(testutils.InstallNamespace, "agentgateway-tls-plaintext-test")

	testInstallation := e2e.CreateTestInstallation(
		t,
		&install.Context{
			InstallNamespace:          installNs,
			ProfileValuesManifestFile: e2e.EmptyValuesManifestPath,
			ValuesManifestFile:        e2e.ControlPlaneTLSPlaintextManifestPath,
			ExtraHelmArgs: []string{
				"--set", "controller.extraEnv.KGW_GLOBAL_POLICY_NAMESPACE=" + installNs,
			},
		},
	)
	if !nsEnvPredefined {
		os.Setenv(testutils.InstallNamespace, installNs)
	}

	testutils.Cleanup(t, func() {
		if !nsEnvPredefined {
			os.Unsetenv(testutils.InstallNamespace)
		}
		testInstallation.Uninstall(cleanupCtx, t)
	})
	testInstallation.InstallFromLocalChart(t.Context(), t)

	TLSPlaintextSuiteRunner().Run(t.Context(), t, testInstallation)
}
