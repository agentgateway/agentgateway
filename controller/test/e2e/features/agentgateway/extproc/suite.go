//go:build e2e

package extproc

import (
	"context"
	"encoding/json"
	"net/http"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/transforms"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

// testingSuite is a suite of tests for ExtProc functionality with AgentgatewayPolicy
type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	// Define the setup TestCase for common resources only
	setupTestCase := base.TestCase{
		Manifests: []string{
			backendWithServiceManifest,
			extProcManifest,
		},
	}

	// Test-specific manifests are applied per test
	testCases := map[string]*base.TestCase{
		"TestExtProcWithHTTPRouteTargetRef": {
			Manifests: []string{
				routeWithTargetReferenceManifest,
			},
		},
		"TestExtProcWithGatewayTargetRef": {
			Manifests: []string{
				gatewayTargetReferenceManifest,
			},
		},
	}

	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setupTestCase, testCases),
	}
}

func (s *testingSuite) SetupSuite() {
	s.BaseTestingSuite.SetupSuite()
}

// TestExtProcWithGatewayTargetRef tests ExtProc with targetRef to Gateway using AgentgatewayPolicy
func (s *testingSuite) TestExtProcWithGatewayTargetRef() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		gateway.Name,
		gateway.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
		timeout,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		gateway.Name,
		gateway.Namespace,
		gwv1.GatewayConditionAccepted,
		metav1.ConditionTrue,
		timeout,
	)

	testCases := []struct {
		name string
		opts []curl.Option
		resp *testmatchers.HttpResponse
	}{
		{
			name: "first route should have ExtProc applied via Gateway policy",
			opts: []curl.Option{
				curl.WithHostHeader("www.example.com"),
				curl.WithPath("/"),
				curl.WithHeader("instructions", getInstructionsJson(instructions{
					AddHeaders: map[string]string{"extproctest": "true"},
				})),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithJsonBody(),
					gomega.And(
						gomega.HaveKeyWithValue("headers", gomega.HaveKey("Extproctest")),
					),
				),
			},
		},
		{
			name: "second route also has ExtProc applied via Gateway policy",
			opts: []curl.Option{
				curl.WithHostHeader("www.example.com"),
				curl.WithPath("/myapp"),
				curl.WithHeader("instructions", getInstructionsJson(instructions{
					AddHeaders: map[string]string{"extproctest": "true"},
				})),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithJsonBody(),
					gomega.And(
						gomega.HaveKeyWithValue("headers", gomega.HaveKey("Extproctest")),
					),
				),
			},
		},
	}
	for _, tc := range testCases {
		s.Run(tc.name, func() {
			common.BaseGateway.Send(s.T(), tc.resp, tc.opts...)
		})
	}
}

// TestExtProcWithHTTPRouteTargetRef tests ExtProc with targetRef to HTTPRoute using AgentgatewayPolicy
func (s *testingSuite) TestExtProcWithHTTPRouteTargetRef() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		gateway.Name,
		gateway.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
		timeout,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		gateway.Name,
		gateway.Namespace,
		gwv1.GatewayConditionAccepted,
		metav1.ConditionTrue,
		timeout,
	)

	testCases := []struct {
		name string
		opts []curl.Option
		resp *testmatchers.HttpResponse
	}{
		{
			name: "route with ExtProc applied should have header modified",
			opts: []curl.Option{
				curl.WithHostHeader("www.example.com"),
				curl.WithPath("/myapp"),
				curl.WithHeader("instructions", getInstructionsJson(instructions{
					AddHeaders: map[string]string{"extproctest": "true"},
				})),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithJsonBody(),
					gomega.And(
						gomega.HaveKeyWithValue("headers", gomega.HaveKey("Extproctest")),
					),
				),
			},
		},
		{
			name: "route without ExtProc should not have header modified",
			opts: []curl.Option{
				curl.WithHostHeader("www.example.com"),
				curl.WithPath("/"),
				curl.WithHeader("instructions", getInstructionsJson(instructions{
					AddHeaders: map[string]string{"extproctest": "true"},
				})),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithJsonBody(),
					gomega.And(
						gomega.HaveKeyWithValue("headers", gomega.Not(gomega.HaveKey("Extproctest"))),
					),
				),
			},
		},
	}
	for _, tc := range testCases {
		s.Run(tc.name, func() {
			common.BaseGateway.Send(s.T(), tc.resp, tc.opts...)
		})
	}
}

// The instructions format that the example extproc service understands.
// See test/e2e/defaults/extproc/README.md for more details.
type instructions struct {
	// Header key/value pairs to add to the request or response.
	AddHeaders map[string]string `json:"addHeaders"`
	// Header keys to remove from the request or response.
	RemoveHeaders []string `json:"removeHeaders"`
}

func getInstructionsJson(instr instructions) string {
	bytes, _ := json.Marshal(instr)
	return string(bytes)
}
