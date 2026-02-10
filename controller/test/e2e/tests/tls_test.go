//go:build e2e

package tests_test

import (
	"context"
	"fmt"
	"os"
	"testing"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/envutils"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/features/tls"
	. "github.com/kgateway-dev/kgateway/v2/test/e2e/tests"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/install"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

// TestControlPlaneTLS tests the TLS control plane integration functionality.
// This test requires a dedicated installation with TLS enabled for xDS communication.
func TestControlPlaneTLS(t *testing.T) {
	cleanupCtx := context.Background()
	installNs, nsEnvPredefined := envutils.LookupOrDefault(testutils.InstallNamespace, "kgateway-tls-test")

	testInstallation := e2e.CreateTestInstallation(
		t,
		&install.Context{
			InstallNamespace:          installNs,
			ProfileValuesManifestFile: e2e.CommonRecommendationManifest,
			ValuesManifestFile:        e2e.ControlPlaneTLSManifestPath,
			ExtraHelmArgs: []string{
				"--set", "controller.extraEnv.KGW_GLOBAL_POLICY_NAMESPACE=" + installNs,
			},
		},
	)
	if !nsEnvPredefined {
		os.Setenv(testutils.InstallNamespace, installNs)
	}

	// Create the installation namespace first if it doesn't exist, since we need to create
	// the TLS secret in it before kgateway starts.
	nsYAML := nsManifest(installNs)
	testutils.Cleanup(t, func() {
		if err := testInstallation.Actions.Kubectl().Delete(cleanupCtx, []byte(nsYAML)); err != nil {
			t.Fatalf("failed to delete namespace: %v", err)
		}
	})
	err := testInstallation.Actions.Kubectl().Apply(t.Context(), []byte(nsYAML))
	if err != nil {
		t.Fatalf("failed to create namespace: %v", err)
	}

	// Create the TLS secret before installing kgateway. The secret must exist in the
	// installation namespace before kgateway starts, as it's required for the control plane
	// to initialize the xDS TLS certificate watcher. No need to register the cleanup function
	// here, as the secret will be cleaned up automatically when the namespace is deleted.
	// Use the same certificate for both ca.crt and tls.crt (self-signed).
	secretYAML, err := tls.SecretManifest(installNs, tls.DefaultExpiration)
	if err != nil {
		t.Fatalf("failed to create TLS secret: %v", err)
	}
	if err := testInstallation.Actions.Kubectl().Apply(t.Context(), []byte(secretYAML)); err != nil {
		t.Fatalf("failed to create TLS secret: %v", err)
	}

	// Install kgateway with TLS enabled
	testutils.Cleanup(t, func() {
		if !nsEnvPredefined {
			os.Unsetenv(testutils.InstallNamespace)
		}
		// use a separate context than the one used for the test, as the test context
		// might be cancelled if we fail to install kgateway.
		testInstallation.Uninstall(cleanupCtx, t)
	})
	testInstallation.InstallFromLocalChart(t.Context(), t)

	TLSSuiteRunner().Run(t.Context(), t, testInstallation)
}

func nsManifest(ns string) string {
	return fmt.Sprintf(`apiVersion: v1
kind: Namespace
metadata:
  name: %s
`, ns)
}
