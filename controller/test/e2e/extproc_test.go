//go:build e2e

package e2e_test

import (
	"encoding/json"
	"net/http"
	"testing"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/gomega/transforms"
)

func TestExtProc(t *testing.T) {
	agw := New(t)

	agw.Run("GatewayTargetRef", func() {
		testExtProcWithGatewayTargetRef(agw)
	})
	agw.Run("HTTPRouteTargetRef", func() {
		testExtProcWithHTTPRouteTargetRef(agw)
	})
}

// testExtProcWithGatewayTargetRef tests ExtProc with targetRef to Gateway using AgentgatewayPolicy
func testExtProcWithGatewayTargetRef(agw *base.BaseTestingSuite) {
	agw.Apply(manifest("extproc", "gateway-targetref.yaml"))

	testCases := []struct {
		name string
		url  string
		opts []curl.Option
		resp *testmatchers.HttpResponse
	}{
		{
			name: "first route should have ExtProc applied via Gateway policy",
			url:  "www.example.com",
			opts: []curl.Option{
				curl.WithHeader("instructions", getInstructionsJson(instructions{
					AddHeaders: map[string]string{"extproctest": "true"},
				})),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithEchoHeaders(),
					gomega.HaveKeyWithValue("Extproctest", "true"),
				),
			},
		},
		{
			name: "second route also has ExtProc applied via Gateway policy",
			url:  "www.example.com/myapp",
			opts: []curl.Option{
				curl.WithHeader("instructions", getInstructionsJson(instructions{
					AddHeaders: map[string]string{"extproctest": "true"},
				})),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithEchoHeaders(),
					gomega.HaveKeyWithValue("Extproctest", "true"),
				),
			},
		},
	}
	for _, tc := range testCases {
		tc := tc
		agw.Run(tc.name, func() {
			agw.Send(tc.url, tc.resp, tc.opts...)
		})
	}
}

// testExtProcWithHTTPRouteTargetRef tests ExtProc with targetRef to HTTPRoute using AgentgatewayPolicy
func testExtProcWithHTTPRouteTargetRef(agw *base.BaseTestingSuite) {
	agw.Apply(manifest("extproc", "httproute-targetref.yaml"))

	testCases := []struct {
		name string
		url  string
		opts []curl.Option
		resp *testmatchers.HttpResponse
	}{
		{
			name: "route with ExtProc applied should have header modified",
			url:  "www.example.com/myapp",
			opts: []curl.Option{
				curl.WithHeader("instructions", getInstructionsJson(instructions{
					AddHeaders: map[string]string{"extproctest": "true"},
				})),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithEchoHeaders(),
					gomega.HaveKeyWithValue("Extproctest", "true"),
				),
			},
		},
		{
			name: "route without ExtProc should not have header modified",
			url:  "www.example.com",
			opts: []curl.Option{
				curl.WithHeader("instructions", getInstructionsJson(instructions{
					AddHeaders: map[string]string{"extproctest": "true"},
				})),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Body: gomega.WithTransform(transforms.WithEchoHeaders(),
					gomega.Not(gomega.HaveKeyWithValue("Extproctest", "true")),
				),
			},
		},
	}
	for _, tc := range testCases {
		tc := tc
		agw.Run(tc.name, func() {
			agw.Send(tc.url, tc.resp, tc.opts...)
		})
	}
}

// The instructions format that the extproc service in testbox understands.
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
