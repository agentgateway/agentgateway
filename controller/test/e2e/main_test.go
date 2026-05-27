//go:build e2e

package e2e_test

import (
	"context"
	"os"
	"sync"
	"testing"

	"k8s.io/apimachinery/pkg/types"

	e2e "github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

var (
	agwCtx              context.Context
	agwSetupOnce        sync.Once
	agwSetupT           *testing.T
	agwNsEnvPredefined  bool
	agwInstallNamespace string
	agwInstallation     *e2e.TestInstallation
)

func TestMain(m *testing.M) {
	code := m.Run()

	if agwInstallation != nil {
		skipCleanup := testutils.ShouldSkipAllTeardown() || code != 0 && testutils.ShouldFailFastAndPersist()
		if !skipCleanup {
			agwInstallation.Uninstall(agwCtx, agwSetupT)
			agwInstallation.Finalize()
		}
		agwInstallation = nil
	}
	if !agwNsEnvPredefined && agwInstallNamespace != "" {
		os.Unsetenv(testutils.InstallNamespace)
	}

	os.Exit(code)
}

func New(t *testing.T, opts ...base.SuiteOption) base.Test {
	t.Helper()
	base.ConfigureTest(t)
	agwSetupOnce.Do(func() {
		done := base.TraceStep(t, "shared e2e setup")
		setup(t)
		done()
	})
	if agwInstallation == nil {
		t.Fatal("agentgateway e2e installation was not initialized")
	}
	testutils.Cleanup(t, func() {
		if t.Failed() && !testutils.ShouldSkipBugReport() {
			agwInstallation.PreFailHandler(agwCtx, t)
		}
	})

	test := base.NewTest(agwCtx, agwInstallation, t, opts...)
	test.Setup()
	return test
}

func setup(t *testing.T) {
	t.Helper()
	agwSetupT = t
	agwCtx = context.Background()
	done := base.TraceStep(t, "resolved install namespace")
	installNs, nsEnvPredefined := testutils.InstallNamespaceOrDefault("agentgateway-system")
	agwInstallNamespace = installNs
	agwNsEnvPredefined = nsEnvPredefined
	done()

	done = base.TraceStep(t, "created test installation")
	agwInstallation = e2e.CreateSharedTestInstallation(
		installNs,
		e2e.BaseValuesManifestPath,
	)
	done()

	if !nsEnvPredefined {
		os.Setenv(testutils.InstallNamespace, installNs)
	}

	done = base.TraceStep(t, "installed local chart")
	agwInstallation.InstallFromLocalChart(agwCtx, t)
	done()

	done = base.TraceStep(t, "applied base config")
	base.SetupBaseConfig(agwCtx, t, agwInstallation, e2e.ManifestPath("agent-gateway-base.yaml"))
	done()

	done = base.TraceStep(t, "resolved base gateway")
	base.SetupBaseGateway(agwCtx, t, agwInstallation, types.NamespacedName{
		Namespace: base.Namespace,
		Name:      "gateway",
	})
	done()
}

func manifest(pathParts ...string) string {
	return base.Manifest(pathParts...)
}
