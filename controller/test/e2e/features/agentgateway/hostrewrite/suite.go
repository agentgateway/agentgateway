//go:build e2e

package hostrewrite

import (
	"context"
	"net/http"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
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

func (s *testingSuite) TestServiceBackendHostRewrite() {
	s.assertHostRewrite(svcDefaultHost, svcDefaultHost)
	s.assertHostRewrite(svcNoneHost, svcNoneHost)
	s.assertHostRewrite(svcAutoHost, backendHost)
}

func (s *testingSuite) TestAgentgatewayBackendHostRewrite() {
	s.assertHostRewrite(agbeDefaultHost, backendHost)
	s.assertHostRewrite(agbeNoneHost, agbeNoneHost)
	s.assertHostRewrite(agbeAutoHost, backendHost)
}

func (s *testingSuite) assertHostRewrite(routeHost string, expectedUpstreamHost string) {
	common.BaseGateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring("Host=" + expectedUpstreamHost),
		},
		curl.WithHostHeader(routeHost),
		curl.WithPath("/"),
	)
}
