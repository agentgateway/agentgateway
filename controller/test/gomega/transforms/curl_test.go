package transforms_test

import (
	"fmt"
	"net/http"
	"testing"

	"istio.io/istio/pkg/test/util/assert"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/kubectl"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/gomega/transforms"
)

func TestWithCurlResponse(t *testing.T) {
	validHttpResponseStringNoVersion := "*   Trying 10.96.92.168...\n* TCP_NODELAY set\n* Connected to gateway-proxy (10.96.92.168) port 80 (#0)\n> GET /test HTTP/1.1\n> Host: test-domain\n> User-Agent: curl/7.58.0\n> Accept: */*\n> header1: value1\n> header2: value2\n> instructions: invalid json value\n> \n< HTTP/%s 200 OK\n< x-powered-by: Express\n< content-type: application/json; charset=utf-8\n< content-length: 444\n< etag: W/\"1bc-u/C5Wu/6BvNtW0jEh2E+mCP4gUg\"\n< date: Thu, 28 Mar 2024 19:40:18 GMT\n< x-envoy-upstream-service-time: 374\n< server: envoy\n< \n{ [444 bytes data]\n* Connection #0 to host gateway-proxy left intact"
	validHttp1dot1StringResponse := fmt.Sprintf(validHttpResponseStringNoVersion, "1.1")
	validHttp1dot1CurlResponse := kubectl.CurlResponse{
		StdErr: validHttp1dot1StringResponse,
	}
	validHttp2CurlResponse := kubectl.CurlResponse{
		StdErr: fmt.Sprintf(validHttpResponseStringNoVersion, "2"),
	}

	tests := []struct {
		name         string
		curlResponse *kubectl.CurlResponse
	}{
		{
			name:         "valid HTTP/1.1 response",
			curlResponse: &validHttp1dot1CurlResponse,
		},
		{
			name:         "valid HTTP/2 response",
			curlResponse: &validHttp2CurlResponse,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// nolint: bodyclose // false positive
			response := transforms.WithCurlResponse(tt.curlResponse)
			httpMatcher := matchers.HaveHttpResponse(&matchers.HttpResponse{
				StatusCode: http.StatusOK,
			})
			ok, err := httpMatcher.Match(response)
			assert.NoError(t, err)
			assert.Equal(t, true, ok)
		})
	}
}
