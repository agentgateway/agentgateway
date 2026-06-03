//go:build e2e

package e2e

import (
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/avast/retry-go/v4"
	"istio.io/istio/pkg/slices"
	istioassert "istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/helmutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/portforward"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/cluster"
	testruntime "github.com/agentgateway/agentgateway/controller/test/e2e/testutils/runtime"
	"github.com/agentgateway/agentgateway/controller/test/helpers"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

// CreateSharedTestInstallation constructs an installation for package-level
// fixtures. Call Finalize after the shared installation is no longer needed.
func CreateSharedTestInstallation(
	installNamespace string,
	valuesManifestFile string,
) *TestInstallation {
	runtimeContext := testruntime.NewContext()
	clusterContext := cluster.MustKindContext(runtimeContext.ClusterName)

	return createTestInstallationForCluster(runtimeContext, clusterContext, installNamespace, valuesManifestFile)
}

func createTestInstallationForCluster(
	runtimeContext testruntime.Context,
	clusterContext *cluster.Context,
	installNamespace string,
	valuesManifestFile string,
) *TestInstallation {
	if installNamespace == "" {
		panic("install namespace must not be empty")
	}
	if valuesManifestFile == "" {
		panic("values manifest file must not be empty")
	}
	return &TestInstallation{
		// RuntimeContext contains the set of properties that are defined at runtime by whoever is invoking tests
		RuntimeContext: runtimeContext,

		// ClusterContext contains the metadata about the Kubernetes Cluster that is used for this TestCluster
		ClusterContext: clusterContext,

		InstallNamespace:   installNamespace,
		ValuesManifestFile: valuesManifestFile,

		Helm: helmutils.NewClient(),

		// GeneratedFiles contains the unique location where files generated during the execution
		// of tests against this installation will be stored
		// By creating a unique location, per TestInstallation and per Cluster.Name we guarantee isolation
		// between TestInstallation outputs per CI run
		GeneratedFiles: MustGeneratedFiles(installNamespace, clusterContext.Name),
	}
}

// TestInstallation is the structure around a set of tests that validate behavior for an installation
// of agentgateway.
type TestInstallation struct {
	fmt.Stringer

	// RuntimeContext contains the set of properties that are defined at runtime by whoever is invoking tests
	RuntimeContext testruntime.Context

	// ClusterContext contains the metadata about the Kubernetes Cluster that is used for this TestCluster
	ClusterContext *cluster.Context

	InstallNamespace   string
	ValuesManifestFile string
	ExtraHelmArgs      []string

	Helm *helmutils.Client

	// GeneratedFiles is the collection of directories and files that this test installation _may_ create
	GeneratedFiles GeneratedFiles
}

func (i *TestInstallation) String() string {
	return i.InstallNamespace
}

func (i *TestInstallation) Finalize() {
	if err := os.RemoveAll(i.GeneratedFiles.TempDir); err != nil {
		panic(fmt.Sprintf("Failed to remove temporary directory: %s", i.GeneratedFiles.TempDir))
	}
}

func (i *TestInstallation) StartPortForward(ctx context.Context, options ...portforward.Option) (portforward.PortForwarder, error) {
	options = append([]portforward.Option{
		portforward.WithWriters(io.Discard, io.Discard),
		portforward.WithKubeContext(i.ClusterContext.KubeContext),
	}, options...)

	forwarder := portforward.NewCliPortForwarder(options...)
	err := forwarder.Start(
		ctx,
		retry.LastErrorOnly(true),
		retry.Delay(250*time.Millisecond),
		retry.DelayType(retry.FixedDelay),
		retry.Attempts(60),
	)
	return forwarder, err
}

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
		if i.releaseExists(ctx, helmutils.AgentgatewayCRDChartName, i.InstallNamespace) {
			return
		}
	}

	// Use absolute chart paths so tests work regardless of current working directory.
	crdChartPath := filepath.Join(fsutils.GetModuleRoot(), "controller", "install", "helm", "agentgateway-crds")
	// install the CRD chart first
	err := i.Helm.WithReceiver(os.Stdout).Upgrade(
		ctx,
		helmutils.InstallOpts{
			CreateNamespace: true,
			ReleaseName:     helmutils.AgentgatewayCRDChartName,
			Namespace:       i.InstallNamespace,
			Chart:           crdChartPath,
		})
	istioassert.NoError(t, err)
}

// InstallAgentgatewayCoreFromLocalChart installs the agentgateway main chart from the local filesystem
func (i *TestInstallation) InstallAgentgatewayCoreFromLocalChart(ctx context.Context, t *testing.T) {
	if testutils.ShouldSkipInstallAndTeardown() {
		return
	}

	// Check if we should skip installation if the release already exists (PERSIST_INSTALL or FAIL_FAST_AND_PERSIST mode)
	if testutils.ShouldPersistInstall() || testutils.ShouldFailFastAndPersist() {
		if i.releaseExists(ctx, helmutils.AgentgatewayChartName, i.InstallNamespace) {
			return
		}
	}

	// Use absolute chart paths so tests work regardless of current working directory.
	coreChartPath := filepath.Join(fsutils.GetModuleRoot(), "controller", "install", "helm", "agentgateway")

	extraArgs := i.ExtraHelmArgs
	// If VERSION is set, override the chart's AppVersion so locally-built images are used
	// instead of trying to pull the chart's default appVersion from the remote registry.
	if tag, ok := testutils.VersionValue(); ok {
		extraArgs = append(extraArgs, "--set-string", "image.tag="+tag)
	}

	// and then install the main chart
	err := i.Helm.WithReceiver(os.Stdout).Upgrade(
		ctx,
		helmutils.InstallOpts{
			Namespace:       i.InstallNamespace,
			CreateNamespace: true,
			ValuesFiles: []string{
				i.ValuesManifestFile,
				ManifestPath("agent-gateway-integration.yaml"),
			},
			ReleaseName: helmutils.AgentgatewayChartName,
			Chart:       coreChartPath,
			ExtraArgs:   extraArgs,
		})
	istioassert.NoError(t, err)
	assertions.EventuallyGatewayInstallSucceeded(t, ctx, i.ClusterContext, i.InstallNamespace)
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
	if !i.releaseExists(ctx, helmutils.AgentgatewayChartName, i.InstallNamespace) {
		// Release doesn't exist, nothing to uninstall
		return
	}

	// uninstall the main chart first
	err := i.Helm.Uninstall(
		ctx,
		helmutils.UninstallOpts{
			Namespace:   i.InstallNamespace,
			ReleaseName: helmutils.AgentgatewayChartName,
			ExtraArgs:   []string{"--wait"}, // Default timeout is 5m
		},
	)
	istioassert.NoError(t, err)
	assertions.EventuallyGatewayUninstallSucceeded(t, ctx, i.ClusterContext, i.InstallNamespace)
}

// UninstallAgentgatewayCRDs uninstalls the agentgateway CRD chart
func (i *TestInstallation) UninstallAgentgatewayCRDs(ctx context.Context, t *testing.T) {
	if testutils.ShouldSkipInstallAndTeardown() || testutils.ShouldPersistInstall() {
		return
	}

	// Check if the release exists before attempting to uninstall
	if !i.releaseExists(ctx, helmutils.AgentgatewayCRDChartName, i.InstallNamespace) {
		// Release doesn't exist, nothing to uninstall
		return
	}

	// uninstall the CRD chart
	err := i.Helm.Uninstall(
		ctx,
		helmutils.UninstallOpts{
			Namespace:   i.InstallNamespace,
			ReleaseName: helmutils.AgentgatewayCRDChartName,
			ExtraArgs:   []string{"--wait"}, // Default timeout is 5m
		},
	)
	istioassert.NoError(t, err)
}

// PreFailHandler is the function that is invoked if a test in the given TestInstallation fails
func (i *TestInstallation) PreFailHandler(ctx context.Context, t *testing.T) {
	i.preFailHandler(ctx, t, filepath.Join(i.GeneratedFiles.FailureDir, t.Name()))
}

// preFailHandler is the function that is invoked if a test in the given TestInstallation fails
func (i *TestInstallation) preFailHandler(ctx context.Context, t *testing.T, dir string) {
	// The idea here is we want to accumulate ALL information about this TestInstallation into a single directory
	// That way we can upload it in CI, or inspect it locally

	err := os.MkdirAll(dir, os.ModePerm)
	// We don't want to fail on the output directory already existing. This could occur
	// if multiple tests running in the same cluster from the same installation namespace
	// fail.
	if err != nil && !errors.Is(err, fs.ErrExist) {
		istioassert.NoError(t, err)
	}

	// The kubernetes/e2e tests may use multiple namespaces, so we need to dump all of them
	namespaceList, err := i.ClusterContext.Client.Kube().CoreV1().Namespaces().List(ctx, metav1.ListOptions{})
	istioassert.NoError(t, err)
	namespaces := slices.Map(namespaceList.Items, func(ns corev1.Namespace) string {
		return ns.Name
	})
	namespaces = slices.Filter(namespaces, func(s string) bool {
		return s != "kube-node-lease" &&
			s != "kube-public" &&
			s != "kube-system" &&
			s != "local-path-storage" &&
			s != "metallb-system"
	})

	// Dump the logs and state of the cluster
	helpers.StandardAgentgatewayDumpOnFail(os.Stdout, i.ClusterContext.ControllerClient, i.ClusterContext.Client.Kube(), dir, namespaces)
}

func (i *TestInstallation) releaseExists(ctx context.Context, releaseName, namespace string) bool {
	l := &corev1.SecretList{}
	if err := i.ClusterContext.ControllerClient.List(ctx, l, &client.ListOptions{
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
