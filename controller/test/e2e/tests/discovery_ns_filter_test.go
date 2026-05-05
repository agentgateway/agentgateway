//go:build e2e

package tests_test

import (
	"context"
	"os"
	"testing"

	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/envutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/features/agentgateway/discoverynsfilter"
	. "github.com/agentgateway/agentgateway/controller/test/e2e/tests"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/install"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

// TestDiscoveryNSFilter tests that the AGW_DISCOVERY_NAMESPACE_SELECTORS setting restricts
// the controller to only watch resources in namespaces matching the configured label selector,
// and that the filter responds dynamically to namespace label changes.
func TestDiscoveryNSFilter(t *testing.T) {
	cleanupCtx := context.Background()
	installNs, nsEnvPredefined := envutils.LookupOrDefault(testutils.InstallNamespace, "agentgateway-discoveryns")

	testInstallation := e2e.CreateTestInstallation(
		t,
		&install.Context{
			InstallNamespace:          installNs,
			ChartType:                 "agentgateway",
			ProfileValuesManifestFile: e2e.EmptyValuesManifestPath,
			ValuesManifestFile:        e2e.ManifestPath("discovery-ns-filter-helm.yaml"),
		},
	)

	if !nsEnvPredefined {
		os.Setenv(testutils.InstallNamespace, installNs)
	}

	testutils.Cleanup(t, func() {
		if !nsEnvPredefined {
			os.Unsetenv(testutils.InstallNamespace)
		}
		if t.Failed() {
			testInstallation.PreFailHandler(cleanupCtx, t)
		}
		testInstallation.Uninstall(cleanupCtx, t)
	})

	ensureDiscoveryNamespaceLabel(cleanupCtx, t, testInstallation, installNs)

	testInstallation.InstallFromLocalChart(t.Context(), t)

	DiscoveryNSFilterSuiteRunner().Run(t.Context(), t, testInstallation)
}

func ensureDiscoveryNamespaceLabel(
	ctx context.Context,
	t *testing.T,
	testInstallation *e2e.TestInstallation,
	namespace string,
) {
	t.Helper()

	key := client.ObjectKey{Name: namespace}
	ns := &corev1.Namespace{}
	err := testInstallation.ClusterContext.Client.Get(ctx, key, ns)
	switch {
	case apierrors.IsNotFound(err):
		ns.ObjectMeta = metav1.ObjectMeta{
			Name:   namespace,
			Labels: map[string]string{discoverynsfilter.DiscoveryLabel: "enabled"},
		}
		if err := testInstallation.ClusterContext.Client.Create(ctx, ns); err != nil {
			t.Fatalf("failed to create namespace %s: %v", namespace, err)
		}
	case err != nil:
		t.Fatalf("failed to get namespace %s: %v", namespace, err)
	default:
		if ns.Labels == nil {
			ns.Labels = make(map[string]string)
		}
		if ns.Labels[discoverynsfilter.DiscoveryLabel] != "enabled" {
			ns.Labels[discoverynsfilter.DiscoveryLabel] = "enabled"
			if err := testInstallation.ClusterContext.Client.Update(ctx, ns); err != nil {
				t.Fatalf("failed to label namespace %s: %v", namespace, err)
			}
		}
	}
}
