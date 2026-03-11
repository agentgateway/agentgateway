//go:build e2e

package priorityrouting

import (
	"context"
	"net/http"
	"path/filepath"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

var (
	evictionManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "priority-routing-eviction.yaml")

	testCases = map[string]*base.TestCase{
		"TestEndpointGroupPriorityRouting": {
			Manifests: []string{evictionManifest},
		},
	}
)

const namespace = "agentgateway-base"

type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, base.TestCase{}, testCases),
	}
}

func (s *testingSuite) TestEndpointGroupPriorityRouting() {
	// Wait for the backend and eviction policy to be accepted.
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwBackendCondition(
		s.Ctx, "priority-eviction-backend", namespace, "Accepted", metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwPolicyCondition(
		s.Ctx, "priority-eviction-policy", namespace, "Accepted", metav1.ConditionTrue,
	)

	// Phase 1: Both endpoints are healthy. The primary (httpbin-primary) has the
	// highest priority and should receive all traffic. Verify by checking that
	// /hostname returns the primary pod's hostname.
	common.BaseGateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring("httpbin-primary-"),
		},
		curl.WithHostHeader("priority-eviction.example.com"),
		curl.WithPath("/hostname"),
	)

	// Phase 2: Scale down the primary to simulate failure.
	err := s.TestInstallation.Actions.Kubectl().Scale(s.Ctx, namespace, "deployment/httpbin-primary", 0)
	s.Require().NoError(err)

	// Phase 3: The primary now has no pods, so requests to it will fail.
	// After 3 consecutive failures the eviction policy removes the primary and
	// traffic fails over to the fallback (httpbin-fallback). Verify by checking
	// that /hostname now returns the fallback pod's hostname — different from
	// the primary's.
	common.BaseGateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring("httpbin-fallback-"),
		},
		curl.WithHostHeader("priority-eviction.example.com"),
		curl.WithPath("/hostname"),
	)
}
