//go:build e2e

package e2e

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"testing"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/helmutils"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/labels"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/actions"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/assertions"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/cluster"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/install"
	testruntime "github.com/kgateway-dev/kgateway/v2/test/e2e/testutils/runtime"
	"github.com/kgateway-dev/kgateway/v2/test/helpers"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

// CreateTestInstallation is the simplest way to construct a TestInstallation in kgateway.
// It is syntactic sugar on top of CreateTestInstallationForCluster
func CreateTestInstallation(
	t *testing.T,
	installContext *install.Context,
) *TestInstallation {
	runtimeContext := testruntime.NewContext()
	clusterContext := cluster.MustKindContext(runtimeContext.ClusterName)

	if err := install.ValidateInstallContext(installContext); err != nil {
		// We error loudly if the context is misconfigured
		panic(err)
	}

	return CreateTestInstallationForCluster(t, runtimeContext, clusterContext, installContext)
}

// CreateTestInstallationForCluster is the standard way to construct a TestInstallation
// It accepts context objects from 3 relevant sources:
//
//	runtime - These are properties that are supplied at runtime and will impact how tests are executed
//	cluster - These are properties that are used to connect to the Kubernetes cluster
//	install - These are properties that are relevant to how the kgateway installation will be configured
func CreateTestInstallationForCluster(
	t *testing.T,
	runtimeContext testruntime.Context,
	clusterContext *cluster.Context,
	installContext *install.Context,
) *TestInstallation {
	installation := &TestInstallation{
		// RuntimeContext contains the set of properties that are defined at runtime by whoever is invoking tests
		RuntimeContext: runtimeContext,

		// ClusterContext contains the metadata about the Kubernetes Cluster that is used for this TestCluster
		ClusterContext: clusterContext,

		// Maintain a reference to the Metadata used for this installation
		Metadata: installContext,

		// Create an actions provider, and point it to the running installation
		Actions: actions.NewActionsProvider().
			WithClusterContext(clusterContext).
			WithInstallContext(installContext),

		// Create an assertions provider, and point it to the running installation
		Assertions: assertions.NewProvider(t).
			WithClusterContext(clusterContext).
			WithInstallContext(installContext),

		// Create an assertions provider function that returns a new provider for each test
		// This ensures each test gets its own properly scoped testing.T
		AssertionsT: func(t *testing.T) *assertions.Provider {
			return assertions.NewProvider(t).
				WithClusterContext(clusterContext).
				WithInstallContext(installContext)
		},

		// GeneratedFiles contains the unique location where files generated during the execution
		// of tests against this installation will be stored
		// By creating a unique location, per TestInstallation and per Cluster.Name we guarantee isolation
		// between TestInstallation outputs per CI run
		GeneratedFiles: MustGeneratedFiles(installContext.InstallNamespace, clusterContext.Name),
	}
	testutils.Cleanup(t, func() {
		installation.finalize()
	})
	return installation
}

// TestInstallation is the structure around a set of tests that validate behavior for an installation
// of kgateway.
type TestInstallation struct {
	fmt.Stringer

	// RuntimeContext contains the set of properties that are defined at runtime by whoever is invoking tests
	RuntimeContext testruntime.Context

	// ClusterContext contains the metadata about the Kubernetes Cluster that is used for this TestCluster
	ClusterContext *cluster.Context

	// Metadata contains the properties used to install kgateway
	Metadata *install.Context

	// Actions is the entity that creates actions that can be executed by the Operator
	Actions *actions.Provider

	// Assertions is the entity that creates assertions that can be executed by the Operator
	// DEPRECATED: Use AssertionsT instead (which is scoped to a specific test and not the root suite)
	Assertions *assertions.Provider

	// AssertionsT is a function that creates assertions for a specific test using the test-scoped testing.T
	// This ensures that assertion failures are properly attributed to the correct test
	AssertionsT func(*testing.T) *assertions.Provider

	// GeneratedFiles is the collection of directories and files that this test installation _may_ create
	GeneratedFiles GeneratedFiles

	// IstioctlBinary is the path to the istioctl binary that can be used to interact with Istio
	IstioctlBinary string
}

func (i *TestInstallation) String() string {
	return i.Metadata.InstallNamespace
}

func (i *TestInstallation) finalize() {
	if err := os.RemoveAll(i.GeneratedFiles.TempDir); err != nil {
		panic(fmt.Sprintf("Failed to remove temporary directory: %s", i.GeneratedFiles.TempDir))
	}
}

// InstallFromLocalChart installs the controller and CRD chart based on the `ChartType` of the underlying
// TestInstallation. By default `kgateway` will be installed but can be set to `agentgateway`
func (i *TestInstallation) InstallFromLocalChart(ctx context.Context, t *testing.T) {
		i.InstallAgentgatewayCRDsFromLocalChart(ctx, t)
		i.InstallAgentgatewayCoreFromLocalChart(ctx, t)
}

// InstallAgentgatewayCRDsFromLocalChart installs the agentgateway CRD chart from the local filesystem
func (i *TestInstallation) InstallAgentgatewayCRDsFromLocalChart(ctx context.Context, t *testing.T) {
	if testutils.ShouldSkipInstallAndTeardown() {
		return
	}

	// Check if we should skip installation if the release already exists (PERSIST_INSTALL or FAIL_FAST_AND_PERSIST mode)
	if testutils.ShouldPersistInstall() || testutils.ShouldFailFastAndPersist() {
		if i.Actions.Helm().ReleaseExists(ctx, helmutils.AgentgatewayCRDChartName, i.Metadata.InstallNamespace) {
			return
		}
	}

	// install the CRD chart first
	err := i.Actions.Helm().WithReceiver(os.Stdout).Upgrade(
		ctx,
		helmutils.InstallOpts{
			CreateNamespace: true,
			ReleaseName:     helmutils.AgentgatewayCRDChartName,
			Namespace:       i.Metadata.InstallNamespace,
			Chart:           "install/helm/agentgateway-crds",
		})
	i.AssertionsT(t).Require.NoError(err)
}

// InstallAgentgatewayCoreFromLocalChart installs the agentgateway main chart from the local filesystem
func (i *TestInstallation) InstallAgentgatewayCoreFromLocalChart(ctx context.Context, t *testing.T) {
	if testutils.ShouldSkipInstallAndTeardown() {
		return
	}

	// Check if we should skip installation if the release already exists (PERSIST_INSTALL or FAIL_FAST_AND_PERSIST mode)
	if testutils.ShouldPersistInstall() || testutils.ShouldFailFastAndPersist() {
		if i.Actions.Helm().ReleaseExists(ctx, helmutils.AgentgatewayChartName, i.Metadata.InstallNamespace) {
			return
		}
	}

	// and then install the main chart
	err := i.Actions.Helm().WithReceiver(os.Stdout).Upgrade(
		ctx,
		helmutils.InstallOpts{
			Namespace:       i.Metadata.InstallNamespace,
			CreateNamespace: true,
			ValuesFiles: []string{
				i.Metadata.ProfileValuesManifestFile,
				i.Metadata.ValuesManifestFile,
				ManifestPath("agent-gateway-integration.yaml"),
			},
			ReleaseName: helmutils.AgentgatewayChartName,
			Chart:           "install/helm/agentgateway",
			ExtraArgs:   i.Metadata.ExtraHelmArgs,
		})
	i.AssertionsT(t).Require.NoError(err)
	i.AssertionsT(t).EventuallyGatewayInstallSucceeded(ctx)
}

func (i *TestInstallation) Uninstall(ctx context.Context, t *testing.T) {

		i.UninstallAgentgatewayCore(ctx, t)
		i.UninstallAgentgatewayCRDs(ctx, t)
}

// UninstallAgentgatewayCore uninstalls the agentgateway main chart
func (i *TestInstallation) UninstallAgentgatewayCore(ctx context.Context, t *testing.T) {
	if testutils.ShouldSkipInstallAndTeardown() || testutils.ShouldPersistInstall() {
		return
	}

	// Check if the release exists before attempting to uninstall
	if !i.Actions.Helm().ReleaseExists(ctx, helmutils.AgentgatewayChartName, i.Metadata.InstallNamespace) {
		// Release doesn't exist, nothing to uninstall
		return
	}

	// uninstall the main chart first
	err := i.Actions.Helm().Uninstall(
		ctx,
		helmutils.UninstallOpts{
			Namespace:   i.Metadata.InstallNamespace,
			ReleaseName: helmutils.AgentgatewayChartName,
			ExtraArgs:   []string{"--wait"}, // Default timeout is 5m
		},
	)
	i.AssertionsT(t).Require.NoError(err, "failed to uninstall main chart")
	i.AssertionsT(t).EventuallyGatewayUninstallSucceeded(ctx)
}

// UninstallAgentgatewayCRDs uninstalls the agentgateway CRD chart
func (i *TestInstallation) UninstallAgentgatewayCRDs(ctx context.Context, t *testing.T) {
	if testutils.ShouldSkipInstallAndTeardown() || testutils.ShouldPersistInstall() {
		return
	}

	// Check if the release exists before attempting to uninstall
	if !i.Actions.Helm().ReleaseExists(ctx, helmutils.AgentgatewayCRDChartName, i.Metadata.InstallNamespace) {
		// Release doesn't exist, nothing to uninstall
		return
	}

	// uninstall the CRD chart
	err := i.Actions.Helm().Uninstall(
		ctx,
		helmutils.UninstallOpts{
			Namespace:   i.Metadata.InstallNamespace,
			ReleaseName: helmutils.AgentgatewayCRDChartName,
			ExtraArgs:   []string{"--wait"}, // Default timeout is 5m
		},
	)
	i.AssertionsT(t).Require.NoError(err, "failed to uninstall CRD chart")
}

// PreFailHandler is the function that is invoked if a test in the given TestInstallation fails
func (i *TestInstallation) PreFailHandler(ctx context.Context, t *testing.T) {
	i.preFailHandler(ctx, t, i.GeneratedFiles.FailureDir)
}

// PerTestPreFailHandler is the function that is invoked if a test in the given TestInstallation fails
func (i *TestInstallation) PerTestPreFailHandler(ctx context.Context, t *testing.T, testName string) {
	i.preFailHandler(ctx, t, filepath.Join(i.GeneratedFiles.FailureDir, testName))
}

// preFailHandler is the function that is invoked if a test in the given TestInstallation fails
func (i *TestInstallation) preFailHandler(ctx context.Context, t *testing.T, dir string) {
	// The idea here is we want to accumulate ALL information about this TestInstallation into a single directory
	// That way we can upload it in CI, or inspect it locally

	err := os.Mkdir(dir, os.ModePerm)
	// We don't want to fail on the output directory already existing. This could occur
	// if multiple tests running in the same cluster from the same installation namespace
	// fail.
	if err != nil && !errors.Is(err, fs.ErrExist) {
		i.AssertionsT(t).Require.NoError(err, "failed to create failure directory")
	}

	// The kubernetes/e2e tests may use multiple namespaces, so we need to dump all of them
	namespaces, err := i.Actions.Kubectl().Namespaces(ctx)
	i.AssertionsT(t).Require.NoError(err, "failed to get namespaces for failure dump")

	// Dump the logs and state of the cluster
	helpers.StandardKgatewayDumpOnFail(os.Stdout, i.Actions.Kubectl(), dir, namespaces)
}

// GeneratedFiles is a collection of files that are generated during the execution of a set of tests
type GeneratedFiles struct {
	// TempDir is the directory where any temporary files should be created
	// Tests may create files for any number of reasons:
	// - A: When a test renders objects in a file, and then uses this file to create and delete values
	// - B: When a test invokes a command that produces a file as a side effect
	// Files in this directory are an implementation detail of the test itself.
	// As a result, it is the callers responsibility to clean up the TempDir when the tests complete
	TempDir string

	// FailureDir is the directory where any assets that are produced on failure will be created
	FailureDir string
}

// MustGeneratedFiles returns GeneratedFiles, or panics if there was an error generating the directories
func MustGeneratedFiles(tmpDirId, clusterId string) GeneratedFiles {
	tmpDir, err := os.MkdirTemp("", tmpDirId)
	if err != nil {
		panic(err)
	}

	// output path is in the format of bug_report/cluster_name
	failureDir := filepath.Join(testruntime.PathToBugReport(), clusterId)
	err = os.MkdirAll(failureDir, os.ModePerm)
	if err != nil {
		panic(err)
	}

	return GeneratedFiles{
		TempDir:    tmpDir,
		FailureDir: failureDir,
	}
}

func (i *TestInstallation) releaseExists(ctx context.Context, releaseName, namespace string) bool {
	l := &corev1.SecretList{}
	if err := i.ClusterContext.Client.List(ctx, l, &client.ListOptions{
		Namespace: namespace,
		LabelSelector: labels.SelectorFromSet(map[string]string{
			"owner": "helm",
			"name":  releaseName,
		}),
	}); err != nil {
		return false
	}
	return len(l.Items) > 0
}
