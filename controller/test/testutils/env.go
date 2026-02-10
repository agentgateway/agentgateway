package testutils

import (
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/envutils"
)

const (
	// SkipInstallAndTeardown can be used when you plan to re-run a test suite and want to skip the installation
	// and teardown of kgateway.
	SkipInstallAndTeardown = "SKIP_INSTALL"

	// PersistInstall is a convenience flag that skips installation if charts are already installed
	// and skips teardown. It will install if nothing is present, but skip installation if charts are already
	// installed, and then skip teardown. Useful for local development - "just handle it" mode.
	PersistInstall = "PERSIST_INSTALL"

	// FailFastAndPersist skips installation if charts already exist and skips
	// cleanup when tests fail. This provides the "install once" convenience of
	// PERSIST_INSTALL combined with "cleanup only on success (unless
	// SkipAllTeardown skips cleanup)" behavior leaving you with a local Kind
	// cluster on failure begging for forensic analysis.
	//
	// Setup/Install behavior:
	// - Installs kgateway if not present
	// - Skips installation if charts are already installed (same as PERSIST_INSTALL)
	//
	// Teardown/Cleanup behavior:
	// - If tests pass: Runs cleanup normally
	// - If tests fail: Skips cleanup to allow inspection of resources
	//
	// To abort further testing after first failure, combine with Go's -failfast flag:
	//   FAIL_FAST_AND_PERSIST=true go test -failfast ./...
	//
	// This is useful for debugging test failures while maintaining a fast development loop.
	FailFastAndPersist = "FAIL_FAST_AND_PERSIST"

	// SkipAllTeardown skips the teardown/cleanup that SkipInstallAndTeardown
	// and PersistInstall do, but then also skips the per-test-suite and
	// per-test-case teardown/cleanup. This is useful for debugging tests but
	// not suitable for reproducing flaky tests to the same extent as
	// FailFastAndPersist since you cannot, in general, run a single test in a
	// loop if it litters.
	SkipAllTeardown = "SKIP_ALL_TEARDOWN"

	// SkipDump, if true, disables test dumping
	SkipDump = "SKIP_DUMP"

	// InstallNamespace is the namespace in which kgateway is installed
	InstallNamespace = "INSTALL_NAMESPACE"

	// SkipIstioInstall is a flag that indicates whether to skip the install of Istio.
	// This is used to test against an existing installation of Istio so that the
	// test framework does not need to install/uninstall Istio.
	SkipIstioInstall = "SKIP_ISTIO_INSTALL"

	// SkipBugReport can be used to skip the automatic bug report generation on test failure.
	// This is useful when running tests in a local development environment to fail fast without generating bug reports.
	SkipBugReport = "SKIP_BUG_REPORT"

	// GithubAction is used by Github Actions and is the name of the currently running action or ID of a step
	// https://docs.github.com/en/actions/learn-github-actions/variables#default-environment-variables
	GithubAction = "GITHUB_ACTION"

	// ReleasedVersion can be used when running KubeE2E tests to have the test suite use a previously released version of kgateway
	// If set to 'LATEST', the most recently released version will be used
	// If set to another value, the test suite will use that version (ie '1.15.0-beta1')
	// This is an optional value, so if it is not set, the test suite will use the locally built version of kgateway
	ReleasedVersion = "RELEASED_VERSION"

	// ClusterName is the name of the cluster used for e2e tests
	ClusterName = "CLUSTER_NAME"

	// This can be used to override the default KubeCtx created.
	// The default KubeCtx used is "kind-<ClusterName>"
	KubeCtx = "KUBE_CTX"

	// DefaultNamespace is the default namespace to use for resources that don't specify one
	// Typically "default" for kind/k8s clusters, may differ for OpenShift/CRC
	DefaultNamespace = "DEFAULT_NAMESPACE"
)

// ShouldSkipInstallAndTeardown returns true if kgateway installation and teardown should be skipped.
func ShouldSkipInstallAndTeardown() bool {
	return envutils.IsEnvTruthy(SkipInstallAndTeardown)
}

// ShouldPersistInstall returns true if the install should be persisted across test runs.
// This skips installation when charts are already installed and skips teardown.
func ShouldPersistInstall() bool {
	return envutils.IsEnvTruthy(PersistInstall)
}

// ShouldSkipIstioInstall returns true if istio installation and teardown should be skipped.
func ShouldSkipIstioInstall() bool {
	return envutils.IsEnvTruthy(SkipIstioInstall)
}

// ShouldFailFastAndPersist returns true if tests should skip cleanup on failure.
// This allows resources to persist for debugging when tests fail.
// Combine with `go test -failfast` to stop running tests after first failure.
func ShouldFailFastAndPersist() bool {
	return envutils.IsEnvTruthy(FailFastAndPersist)
}

// SkipAllTeardown returns true if tests, regardless of success or failure,
// should skip cleanup and teardown to attempt to leave your local Kind cluster
// in the exact state of the heart of the test case. Typically you would run a
// single test case when skipping all teardown because a suite is likely to
// waste your time and confuse you if each test case starts in a chaotic state.
func ShouldSkipAllTeardown() bool {
	return envutils.IsEnvTruthy(SkipAllTeardown)
}

func ShouldSkipBugReport() bool {
	return envutils.IsEnvTruthy(SkipBugReport)
}

// TestingT is an interface that matches the subset of testing.T methods we need
type TestingT interface {
	Failed() bool
	Cleanup(func())
}

// ShouldSkipCleanup returns true if cleanup should be skipped.
// Cleanup is skipped if:
// - The test failed AND ShouldFailFastAndPersist() returns true (FAIL_FAST_AND_PERSIST env var)
func ShouldSkipCleanup(t TestingT) bool {
	if ShouldSkipAllTeardown() {
		return true
	}
	if t.Failed() && ShouldFailFastAndPersist() {
		return true
	}
	return false
}

// Cleanup registers a cleanup function that will only run if cleanup should not be skipped.
// Use this instead of t.Cleanup() to automatically handle cleanup based on environment variables.
//
// Cleanup will be skipped if:
// - SKIP_INSTALL is set (skip all cleanup)
// - PERSIST_INSTALL is set (persist resources across test runs)
// - FAIL_FAST_AND_PERSIST is set AND the test failed (skip cleanup on failure for debugging)
//
// By default, cleanup runs even if tests fail (to clean up resources).
// Set FAIL_FAST_AND_PERSIST=true to skip cleanup on failure for debugging.
func Cleanup(t TestingT, f func()) {
	t.Cleanup(func() {
		if ShouldSkipCleanup(t) {
			return
		}
		f()
	})
}

// GetDefaultNamespace returns the default namespace to use for resources that don't specify one.
// This can be overridden via the DEFAULT_NAMESPACE environment variable.
// Defaults to "default" if not set.
func GetDefaultNamespace() string {
	return envutils.GetOrDefault(DefaultNamespace, "default", false)
}
