package transforms_test

import (
	"fmt"
	"net/http"

	"github.com/onsi/gomega/gstruct"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/transforms"
)

var _ = Describe("Curl", func() {

	var (
		validHttpResponseStringNoVersion = "*   Trying 10.96.92.168...\n* TCP_NODELAY set\n* Connected to gateway-proxy (10.96.92.168) port 80 (#0)\n> GET /test HTTP/1.1\n> Host: test-domain\n> User-Agent: curl/7.58.0\n> Accept: */*\n> header1: value1\n> header2: value2\n> instructions: invalid json value\n> \n< HTTP/%s 200 OK\n< x-powered-by: Express\n< content-type: application/json; charset=utf-8\n< content-length: 444\n< etag: W/\"1bc-u/C5Wu/6BvNtW0jEh2E+mCP4gUg\"\n< date: Thu, 28 Mar 2024 19:40:18 GMT\n< x-envoy-upstream-service-time: 374\n< server: envoy\n< \n{ [444 bytes data]\n* Connection #0 to host gateway-proxy left intact"
		validHttp1dot1StringResponse     = fmt.Sprintf(validHttpResponseStringNoVersion, "1.1")
		validHttp1dot1CurlResponse       = kubectl.CurlResponse{
			StdErr: validHttp1dot1StringResponse,
		}
		validHttp2CurlResponse = kubectl.CurlResponse{
			StdErr: fmt.Sprintf(validHttpResponseStringNoVersion, "2"),
		}
	)

	DescribeTable("WithCurlResponse", func(curlResponse *kubectl.CurlResponse) {
		Expect(curlResponse).To(
			WithTransform(transforms.WithCurlResponse,
				matchers.HaveHttpResponse(&matchers.HttpResponse{
					StatusCode: http.StatusOK,
					Body:       gstruct.Ignore(),
				})))
	},
		Entry("valid HTTP/1.1 response", &validHttp1dot1CurlResponse),
		Entry("valid HTTP/2 response", &validHttp2CurlResponse),
	)
})
