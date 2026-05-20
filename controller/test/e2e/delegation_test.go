//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestDelegation(t *testing.T) {
	agw := New(t)
	agw.Apply(delegationManifest("setup.yaml"))

	agw.Run("Basic", func() {
		testBasicDelegation(agw)
	})
	agw.Run("HeadersAndQueryParams", func() {
		testDelegationWithHeadersAndQueryParams(agw)
	})
	agw.Run("Cyclic", func() {
		testCyclicDelegation(agw)
	})
	agw.Run("Recursive", func() {
		testRecursiveDelegation(agw)
	})
	agw.Run("MultipleParents", func() {
		testMultipleParents(agw)
	})
	agw.Run("UnresolvedChild", func() {
		testUnresolvedChild(agw)
	})
}

func testBasicDelegation(agw *base.BaseTestingSuite) {
	agw.Apply(delegationManifest("basic-delegation.yaml"))

	assertHTTPRouteAccepted(agw, "root", "infra")
	agw.Send("example.com/anything/team1/foo", base.ExpectOK())
	agw.Send("example.com/anything/team2/foo", base.ExpectOK())
}

func testDelegationWithHeadersAndQueryParams(agw *base.BaseTestingSuite) {
	agw.Apply(delegationManifest("delegation-headers-query.yaml"))

	assertHTTPRouteAccepted(agw, "root", "infra")
	agw.Send(
		"example.com/anything/team1/foo?query1=val1&queryX=valX",
		base.ExpectOK(),
		curl.WithHeader("header1", "val1"),
		curl.WithHeader("headerX", "valX"),
	)
	agw.Send(
		"example.com/anything/team2/foo?queryX=valX",
		base.Expect(http.StatusNotFound),
		curl.WithHeader("headerX", "valX"),
	)
}

func testCyclicDelegation(agw *base.BaseTestingSuite) {
	agw.Apply(delegationManifest("cyclic-delegation.yaml"))

	assertHTTPRouteAccepted(agw, "root", "infra")
	agw.Send("example.com/anything/team1/foo", base.ExpectOK())
	agw.Send("example.com/anything/team2/foo", &testmatchers.HttpResponse{
		StatusCode: http.StatusInternalServerError,
		Body:       gomega.ContainSubstring("route delegation cycle detected"),
	})
}

func testRecursiveDelegation(agw *base.BaseTestingSuite) {
	agw.Apply(delegationManifest("recursive-delegation.yaml"))

	assertHTTPRouteAccepted(agw, "root", "infra")
	agw.Send("example.com/anything/team1/foo", base.ExpectOK())
	agw.Send("example.com/anything/team2/foo", base.ExpectOK())
}

func testMultipleParents(agw *base.BaseTestingSuite) {
	agw.Apply(delegationManifest("multiple-parents.yaml"))

	assertHTTPRouteAccepted(agw, "parent1", "infra")
	assertHTTPRouteAccepted(agw, "parent2", "infra")
	agw.Send("parent1.com/anything/team2/foo", base.ExpectOK())
	agw.Send("parent2.com/anything/team2/foo", base.Expect(http.StatusNotFound))
}

func testUnresolvedChild(agw *base.BaseTestingSuite) {
	agw.Apply(delegationManifest("unresolved-child.yaml"))

	assertHTTPRouteAccepted(agw, "root", "infra")
	agw.Send("example.com/anything/team1/foo", base.Expect(http.StatusNotFound))
}

func delegationManifest(name string) string {
	return manifest("delegation", name)
}

func assertHTTPRouteAccepted(t *base.BaseTestingSuite, name, namespace string) {
	t.T().Helper()
	t.TestInstallation.AssertionsT(t.T()).EventuallyHTTPRouteCondition(
		t.Ctx,
		name,
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
}
