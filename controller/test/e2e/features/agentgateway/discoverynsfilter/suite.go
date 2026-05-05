//go:build e2e

package discoverynsfilter

import (
	"context"
	"time"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	routeSelected := &base.TestCase{
		Manifests: []string{routeSelectedManifest},
	}
	routeUnselected := &base.TestCase{
		Manifests: []string{routeUnselectedManifest},
	}

	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, map[string]*base.TestCase{
			"TestRouteInSelectedNamespaceIsReconciled": routeSelected,
			"TestDynamicLabelAddEnablesDiscovery":      routeUnselected,
		}),
	}
}

func (s *testingSuite) TestRouteInSelectedNamespaceIsReconciled() {
	s.assertRouteReconciled("route-selected", nsSelected)
}

func (s *testingSuite) TestDynamicLabelAddEnablesDiscovery() {
	kubectl := s.TestInstallation.Actions.Kubectl()
	s.assertRouteNotReconciled("route-unselected", nsUnselected, 10*time.Second)

	s.Require().NoError(
		kubectl.SetLabel(s.Ctx, "namespace", nsUnselected, "", DiscoveryLabel, "enabled"),
	)
	defer func() {
		_ = kubectl.UnsetLabel(s.Ctx, "namespace", nsUnselected, "", DiscoveryLabel)
	}()

	s.assertRouteReconciled("route-unselected", nsUnselected)
}

func (s *testingSuite) assertRouteReconciled(routeName, namespace string) {
	assertions := s.TestInstallation.AssertionsT(s.T())
	assertions.EventuallyHTTPRouteCondition(
		s.Ctx,
		routeName,
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	assertions.EventuallyHTTPRouteCondition(
		s.Ctx,
		routeName,
		namespace,
		gwv1.RouteConditionResolvedRefs,
		metav1.ConditionTrue,
	)
}

func (s *testingSuite) assertRouteNotReconciled(routeName, namespace string, duration time.Duration) {
	assertions := s.TestInstallation.AssertionsT(s.T())
	assertions.Gomega.Consistently(func(g gomega.Gomega) {
		route := &gwv1.HTTPRoute{}
		err := s.TestInstallation.ClusterContext.Client.Get(
			s.Ctx,
			types.NamespacedName{Name: routeName, Namespace: namespace},
			route,
		)
		g.Expect(err).NotTo(gomega.HaveOccurred())
		g.Expect(route.Status.Parents).To(gomega.BeEmpty(),
			"route should not be reconciled before namespace discovery is enabled")
	}, duration, 2*time.Second).Should(gomega.Succeed())
}
