//go:build e2e

package tests_test

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/onsi/gomega"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/envutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/install"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

const (
	discoveryLabel = "agentgateway.dev/discovery"
	nsSelected     = "discoveryns-selected"
	nsUnselected   = "discoveryns-unselected"
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

	agw := base.NewSuite(t.Context(), testInstallation)
	agw.SetT(t)
	agw.SetupSuite()
	t.Cleanup(agw.TearDownSuite)

	agw.Apply(discoveryNSManifest("setup.yaml"))

	t.Run("RouteInSelectedNamespaceIsReconciled", func(t *testing.T) {
		agw.SetT(t)
		agw.Apply(discoveryNSManifest("route-selected.yaml"))
		assertDiscoveryRouteReconciled(t, agw, "route-selected", nsSelected)
	})

	t.Run("DynamicLabelAddEnablesDiscovery", func(t *testing.T) {
		agw.SetT(t)
		agw.Apply(discoveryNSManifest("route-unselected.yaml"))
		assertDiscoveryRouteNotReconciled(t, agw, "route-unselected", nsUnselected, 10*time.Second)

		setDiscoveryNamespaceLabel(t.Context(), t, testInstallation, nsUnselected, "enabled")
		t.Cleanup(func() {
			unsetDiscoveryNamespaceLabel(context.Background(), t, testInstallation, nsUnselected)
		})

		assertDiscoveryRouteReconciled(t, agw, "route-unselected", nsUnselected)
	})
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
			Labels: map[string]string{discoveryLabel: "enabled"},
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
		if ns.Labels[discoveryLabel] != "enabled" {
			ns.Labels[discoveryLabel] = "enabled"
			if err := testInstallation.ClusterContext.Client.Update(ctx, ns); err != nil {
				t.Fatalf("failed to label namespace %s: %v", namespace, err)
			}
		}
	}
}

func setDiscoveryNamespaceLabel(
	ctx context.Context,
	t *testing.T,
	testInstallation *e2e.TestInstallation,
	namespace string,
	value string,
) {
	t.Helper()

	ns := &corev1.Namespace{}
	if err := testInstallation.ClusterContext.Client.Get(ctx, client.ObjectKey{Name: namespace}, ns); err != nil {
		t.Fatalf("failed to get namespace %s: %v", namespace, err)
	}
	if ns.Labels == nil {
		ns.Labels = make(map[string]string)
	}
	ns.Labels[discoveryLabel] = value
	if err := testInstallation.ClusterContext.Client.Update(ctx, ns); err != nil {
		t.Fatalf("failed to label namespace %s: %v", namespace, err)
	}
}

func unsetDiscoveryNamespaceLabel(
	ctx context.Context,
	t *testing.T,
	testInstallation *e2e.TestInstallation,
	namespace string,
) {
	t.Helper()

	ns := &corev1.Namespace{}
	if err := testInstallation.ClusterContext.Client.Get(ctx, client.ObjectKey{Name: namespace}, ns); err != nil {
		t.Logf("failed to get namespace %s while removing discovery label: %v", namespace, err)
		return
	}
	delete(ns.Labels, discoveryLabel)
	if err := testInstallation.ClusterContext.Client.Update(ctx, ns); err != nil {
		t.Logf("failed to remove discovery label from namespace %s: %v", namespace, err)
	}
}

func discoveryNSManifest(name string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", "discoverynsfilter", name)
}

func assertDiscoveryRouteReconciled(t *testing.T, agw *base.BaseTestingSuite, routeName, namespace string) {
	t.Helper()
	assertions := agw.TestInstallation.AssertionsT(t)
	assertions.EventuallyHTTPRouteCondition(
		agw.Ctx,
		routeName,
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	assertions.EventuallyHTTPRouteCondition(
		agw.Ctx,
		routeName,
		namespace,
		gwv1.RouteConditionResolvedRefs,
		metav1.ConditionTrue,
	)
}

func assertDiscoveryRouteNotReconciled(t *testing.T, agw *base.BaseTestingSuite, routeName, namespace string, duration time.Duration) {
	t.Helper()
	assertions := agw.TestInstallation.AssertionsT(t)
	assertions.Gomega.Consistently(func(g gomega.Gomega) {
		route := &gwv1.HTTPRoute{}
		err := agw.TestInstallation.ClusterContext.Client.Get(
			agw.Ctx,
			types.NamespacedName{Name: routeName, Namespace: namespace},
			route,
		)
		g.Expect(err).NotTo(gomega.HaveOccurred())
		g.Expect(route.Status.Parents).To(gomega.BeEmpty(),
			"route should not be reconciled before namespace discovery is enabled")
	}, duration, 2*time.Second).Should(gomega.Succeed())
}
