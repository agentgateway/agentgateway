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
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestDelegation(tt *testing.T) {
	t := New(tt)
	t.Apply(delegationManifest("setup.yaml"))

	t.Run("Basic", func(t base.Test) {
		testBasicDelegation(t)
	})
	t.Run("HeadersAndQueryParams", func(t base.Test) {
		testDelegationWithHeadersAndQueryParams(t)
	})
	t.Run("Cyclic", func(t base.Test) {
		testCyclicDelegation(t)
	})
	t.Run("Recursive", func(t base.Test) {
		testRecursiveDelegation(t)
	})
	t.Run("MultipleParents", func(t base.Test) {
		testMultipleParents(t)
	})
	t.Run("UnresolvedChild", func(t base.Test) {
		testUnresolvedChild(t)
	})
}

func testBasicDelegation(t base.Test) {
	t.Apply(delegationManifest("basic-delegation.yaml"))

	assertHTTPRouteAccepted(t, "root")
	t.Send("example.com/anything/team1/foo", base.ExpectOK())
	t.Send("example.com/anything/team2/foo", base.ExpectOK())
}

func testDelegationWithHeadersAndQueryParams(t base.Test) {
	t.Apply(delegationManifest("delegation-headers-query.yaml"))

	assertHTTPRouteAccepted(t, "root")
	t.Send(
		"example.com/anything/team1/foo?query1=val1&queryX=valX",
		base.ExpectOK(),
		curl.WithHeader("header1", "val1"),
		curl.WithHeader("headerX", "valX"),
	)
	t.Send(
		"example.com/anything/team2/foo?queryX=valX",
		base.Expect(http.StatusNotFound),
		curl.WithHeader("headerX", "valX"),
	)
}

func testCyclicDelegation(t base.Test) {
	t.Apply(delegationManifest("cyclic-delegation.yaml"))

	assertHTTPRouteAccepted(t, "root")
	t.Send("example.com/anything/team1/foo", base.ExpectOK())
	t.Send("example.com/anything/team2/foo", &testmatchers.HttpResponse{
		StatusCode: http.StatusInternalServerError,
		Body:       gomega.ContainSubstring("route delegation cycle detected"),
	})
}

func testRecursiveDelegation(t base.Test) {
	t.Apply(delegationManifest("recursive-delegation.yaml"))

	assertHTTPRouteAccepted(t, "root")
	t.Send("example.com/anything/team1/foo", base.ExpectOK())
	t.Send("example.com/anything/team2/foo", base.ExpectOK())
}

func testMultipleParents(t base.Test) {
	t.Apply(delegationManifest("multiple-parents.yaml"))

	assertHTTPRouteAccepted(t, "parent1")
	assertHTTPRouteAccepted(t, "parent2")
	t.Send("parent1.com/anything/team2/foo", base.ExpectOK())
	t.Send("parent2.com/anything/team2/foo", base.Expect(http.StatusNotFound))
}

func testUnresolvedChild(t base.Test) {
	t.Apply(delegationManifest("unresolved-child.yaml"))

	assertHTTPRouteAccepted(t, "root")
	t.Send("example.com/anything/team1/foo", base.Expect(http.StatusNotFound))
}

func delegationManifest(name string) string {
	return manifest("delegation", name)
}

func assertHTTPRouteAccepted(t base.Test, name string) {
	t.Helper()
	assertions.EventuallyHTTPRouteCondition(t,
		name,
		"infra",
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
}
