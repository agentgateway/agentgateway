//go:build e2e

package delegation

import (
	"context"
	"net/http"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

// TestBasicDelegation tests basic route delegation where a parent HTTPRoute delegates
// to child HTTPRoutes in different namespaces.
// - Child svc1 in team1 has no parentRefs (implicit delegation via wildcard selector)
// - Child svc2 in team2 has an explicit parentRef pointing to the parent
func (s *testingSuite) TestBasicDelegation() {
	// Assert parent route is accepted
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"root",
		"infra",
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)

	// Request to /anything/team1/foo should be delegated to svc1 in team1
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team1/foo"),
	)

	// Request to /anything/team2/foo should be delegated to svc2 in team2
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team2/foo"),
	)
}

// TestDelegationWithHeadersAndQueryParams tests that parent route match constraints
// (headers and query params) are enforced during delegation.
// - Child svc1 matches the parent's headers and query params -> request succeeds
// - Child svc2 does NOT match the parent's required headers/query params -> request fails
func (s *testingSuite) TestDelegationWithHeadersAndQueryParams() {
	// Assert parent route is accepted
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"root",
		"infra",
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)

	// Request to svc1 with correct headers and query params should succeed
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team1/foo?query1=val1&queryX=valX"),
		curl.WithHeader("header1", "val1"),
		curl.WithHeader("headerX", "valX"),
	)

	// Request to svc2 path with mismatched headers/query params should fail.
	// The child svc2 route specifies headerX/queryX but the parent requires header2/query2
	// to match the /anything/team2 rule. A request with only headerX/queryX won't match
	// the parent's rule, so the delegation never happens.
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusNotFound},
		curl.WithPath("/anything/team2/foo?queryX=valX"),
		curl.WithHeader("headerX", "valX"),
	)
}

// TestCyclicDelegation tests that cyclic route delegation is detected and returns an error.
// - team1 delegation works normally (non-cyclic)
// - team2 delegation creates a cycle: parent -> team2-root -> team2/svc2 -> team2 (self-referencing)
func (s *testingSuite) TestCyclicDelegation() {
	// Assert parent route is accepted
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"root",
		"infra",
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)

	// Request to team1 (non-cyclic) should succeed
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team1/foo"),
	)

	// Request to team2 (cyclic delegation) should return 500 with cycle error
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusInternalServerError,
			Body:       gomega.ContainSubstring("route delegation cycle detected"),
		},
		curl.WithPath("/anything/team2/foo"),
	)
}

// TestMultipleParents tests that a child HTTPRoute attaches to every parent
// that delegates into its namespace via a wildcard route-group reference,
// regardless of whether the child declares an explicit HTTPRoute parentRef.
//   - parent1.com and parent2.com both delegate /anything/team1 and /anything/team2
//     to the team1 / team2 namespaces via wildcard ("name: *") route-group refs.
//   - team1/svc1 has no parentRefs and is reached via both parent hostnames.
//   - team2/svc2 declares an explicit parentRef of infra/parent1; this scopes
//     which parent's status reflects the attachment, but at request time the
//     route is reachable through any parent that references the shared
//     team2/* route-group, since route-groups are shared across parents that
//     target the same gateway.
func (s *testingSuite) TestMultipleParents() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"parent1",
		"infra",
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"parent2",
		"infra",
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)

	// svc1 is reachable via both parent hostnames (implicit delegation).
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team1/foo"),
		curl.WithHostHeader("parent1.com"),
	)
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team1/foo"),
		curl.WithHostHeader("parent2.com"),
	)

	// svc2 (explicit parentRef=infra/parent1) is reachable via parent1.
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team2/foo"),
		curl.WithHostHeader("parent1.com"),
	)
}

// TestRecursiveDelegation tests multi-level route delegation.
//   - Parent infra/root delegates /anything/team2 to an intermediate route
//     team2-root/team2-root, which in turn delegates to team2/svc2.
//   - The shallow /anything/team1 delegation still works in parallel.
func (s *testingSuite) TestRecursiveDelegation() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"root",
		"infra",
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)

	// Single-level delegation via team1 still works
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team1/foo"),
	)

	// Two-level delegation: root -> team2-root -> svc2
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		curl.WithPath("/anything/team2/foo"),
	)
}

// TestUnresolvedChild tests that a parent HTTPRoute which delegates to a
// namespace with no matching children is still Accepted, and that requests
// to the delegated prefix return 404 rather than a hard error.
func (s *testingSuite) TestUnresolvedChild() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"root",
		"infra",
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)

	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: http.StatusNotFound},
		curl.WithPath("/anything/team1/foo"),
	)
}
