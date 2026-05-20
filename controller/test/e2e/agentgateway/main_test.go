//go:build e2e

package agentgateway

import (
	"context"
	"os"
	"sync"
	"testing"

	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/envutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/install"
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

func New(t *testing.T) *base.BaseTestingSuite {
	t.Helper()
	agwSetupOnce.Do(func() {
		setup(t)
	})
	if agwInstallation == nil {
		t.Fatal("agentgateway e2e installation was not initialized")
	}
	t.Cleanup(func() {
		if t.Failed() && !testutils.ShouldSkipBugReport() {
			agwInstallation.PreFailHandler(agwCtx, t)
		}
	})

	suite := base.NewSuite(agwCtx, agwInstallation)
	suite.SetT(t)
	suite.SetupSuite()
	t.Cleanup(suite.TearDownSuite)
	return suite
}

func setup(t *testing.T) {
	t.Helper()
	agwSetupT = t
	agwCtx = context.Background()
	installNs, nsEnvPredefined := envutils.LookupOrDefault(testutils.InstallNamespace, "agentgateway-system")
	agwInstallNamespace = installNs
	agwNsEnvPredefined = nsEnvPredefined
	agwInstallation = e2e.CreateSharedTestInstallation(
		t,
		&install.Context{
			InstallNamespace:          installNs,
			ChartType:                 "agentgateway",
			ProfileValuesManifestFile: e2e.EmptyValuesManifestPath,
			ValuesManifestFile:        e2e.ManifestPath("agent-gateway-integration.yaml"),
		},
	)

	if !nsEnvPredefined {
		os.Setenv(testutils.InstallNamespace, installNs)
	}

	agwInstallation.InstallFromLocalChart(agwCtx, t)

	common.SetupBaseConfig(agwCtx, t, agwInstallation, e2e.ManifestPath("agent-gateway-base.yaml"))
	common.SetupBaseGateway(agwCtx, agwInstallation, types.NamespacedName{
		Namespace: base.Namespace,
		Name:      "gateway",
	})
}

func manifest(pathParts ...string) string {
	return base.Manifest(pathParts...)
}
