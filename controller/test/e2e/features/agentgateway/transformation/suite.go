//go:build e2e

package transformation

import (
	"bytes"
	"context"
	"crypto/tls"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"net/http"
	"time"

	"github.com/stretchr/testify/suite"
	"golang.org/x/net/http2"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

// testingSuite is a suite of basic routing / "happy path" tests
type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	// Define the setup TestCase for common resources
	setupTestCase := base.TestCase{
		Manifests: []string{},
	}

	testCases := map[string]*base.TestCase{
		"TestGatewayWithTransformedHTTPRoute": {
			Manifests: []string{
				transformForHeadersManifest,
				transformForBodyManifest,
				gatewayAttachedTransformManifest,
			},
		},
		"TestGatewayWithTransformedGRPCRoute": {
			Manifests: []string{grpcTransformationManifest},
		},
	}

	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setupTestCase, testCases),
	}
}

func (s *testingSuite) SetupSuite() {
	s.BaseTestingSuite.SetupSuite()
}

func (s *testingSuite) TestGatewayWithTransformedHTTPRoute() {
	// Wait for the agent gateway to be ready
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
		name      string
		routeName string
		opts      []curl.Option
		resp      *testmatchers.HttpResponse
	}{
		{
			name:      "basic-gateway-attached",
			routeName: "gateway-attached-transform",
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Headers: map[string]any{
					"response-gateway": "goodbye",
				},
				NotHeaders: []string{
					"x-foo-response",
				},
			},
		},
		{
			name:      "basic",
			routeName: "headers",
			opts: []curl.Option{
				curl.WithBody("hello"),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Headers: map[string]any{
					"x-foo-response": "notsuper",
				},
				NotHeaders: []string{
					"response-gateway",
				},
			},
		},
		{
			name:      "conditional set by request header", // inja and the request_header function in use
			routeName: "headers",
			opts: []curl.Option{
				curl.WithBody("hello"),
				curl.WithHeader("x-add-bar", "super"),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Headers: map[string]any{
					"x-foo-response": "supersupersuper",
				},
			},
		},
		{
			name:      "pull json info", // shows we parse the body as json
			routeName: "route-for-body",
			opts: []curl.Option{
				curl.WithBody(`{"mykey": {"myinnerkey": "myinnervalue"}}`),
				curl.WithHeader("X-Incoming-Stuff", "super"),
			},
			resp: &testmatchers.HttpResponse{
				StatusCode: http.StatusOK,
				Headers: map[string]any{
					"x-how-great":   "level_super",
					"from-incoming": "key_level_myinnervalue",
				},
			},
		},
	}
	for _, tc := range testCases {
		allOpts := append(tc.opts,
			curl.WithHostHeader(fmt.Sprintf("example-%s.com", tc.routeName)),
		)
		common.BaseGateway.Send(
			s.T(),
			tc.resp,
			allOpts...,
		)
	}
}

// TestGatewayWithTransformedGRPCRoute sends a native HTTP/2 (h2c) request with
// gRPC framing and verifies the transformed response metadata is present.
func (s *testingSuite) TestGatewayWithTransformedGRPCRoute() {
	// Wait for the agent gateway to be ready
	s.TestInstallation.Assertions.EventuallyGatewayCondition(
		s.Ctx,
		gateway.Name,
		gateway.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
		timeout,
	)
	s.TestInstallation.Assertions.EventuallyGatewayCondition(
		s.Ctx,
		gateway.Name,
		gateway.Namespace,
		gwv1.GatewayConditionAccepted,
		metav1.ConditionTrue,
		timeout,
	)

	// Ensure the GRPCRoute is admitted and ready.
	const grpcRouteName = "example-route"
	s.TestInstallation.Assertions.EventuallyGRPCRouteCondition(s.Ctx, grpcRouteName, namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue, timeout)
	s.TestInstallation.Assertions.EventuallyGRPCRouteCondition(s.Ctx, grpcRouteName, namespace, gwv1.RouteConditionResolvedRefs, metav1.ConditionTrue, timeout)

	// Ensure the HTTPRoute that shares the same hostname is also admitted and ready.
	// We'll use this to assert the HTTPRoute does *not* get gRPC metadata/header transformation.
	const httpRouteName = "example-route"
	s.TestInstallation.Assertions.EventuallyHTTPRouteCondition(s.Ctx, httpRouteName, namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue, timeout)
	s.TestInstallation.Assertions.EventuallyHTTPRouteCondition(s.Ctx, httpRouteName, namespace, gwv1.RouteConditionResolvedRefs, metav1.ConditionTrue, timeout)

	const (
		expectedHostname        = "example.com"
		grpcMethodPath          = "/proto.EchoTestService/Echo"
		expectedResponseMetaKey = "x-grpc-response"
		expectedResponseMetaVal = "from-grpc"
	)

	s.Require().Eventually(func() bool {
		resp, body, err := sendH2CGrpcRequest(
			common.BaseGateway.Address,
			expectedHostname,
			grpcMethodPath,
			[]byte{0x0a, 0x05, 'h', 'e', 'l', 'l', 'o'}, // EchoRequest{message:"hello"}
		)
		if err != nil {
			s.T().Logf("grpc request failed: %v", err)
			return false
		}

		// Ensure body is fully drained before checking trailers.
		_ = body

		grpcStatus := resp.Trailer.Get("grpc-status")
		if resp.StatusCode != http.StatusOK || grpcStatus != "0" {
			s.T().Logf("unexpected grpc response status=%d grpc-status=%q headers=%v trailers=%v",
				resp.StatusCode, grpcStatus, resp.Header, resp.Trailer)
			return false
		}

		if resp.Header.Get(expectedResponseMetaKey) != expectedResponseMetaVal {
			s.T().Logf("missing transformed grpc response header %s=%s, got headers=%v",
				expectedResponseMetaKey, expectedResponseMetaVal, resp.Header)
			return false
		}

		return true
	}, timeout, time.Second, "expected transformed response metadata on gRPC route")

	// Assert the HTTPRoute response does *not* include the `x-grpc-response` header, while the GRPCRoute does.
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			NotHeaders: []string{
				"x-grpc-response",
			},
		},
		curl.WithHostHeader(expectedHostname),
	)
}

func sendH2CGrpcRequest(address, authority, methodPath string, protobufPayload []byte) (*http.Response, []byte, error) {
	grpcFrame := make([]byte, 5+len(protobufPayload))
	grpcFrame[0] = 0 // uncompressed
	binary.BigEndian.PutUint32(grpcFrame[1:5], uint32(len(protobufPayload)))
	copy(grpcFrame[5:], protobufPayload)

	url := fmt.Sprintf("http://%s:80%s", address, methodPath)
	req, err := http.NewRequest(http.MethodPost, url, bytes.NewReader(grpcFrame))
	if err != nil {
		return nil, nil, err
	}
	req.Host = authority
	req.Header.Set("Content-Type", "application/grpc")
	req.Header.Set("TE", "trailers")

	client := &http.Client{
		Timeout: 10 * time.Second,
		Transport: &http2.Transport{
			AllowHTTP: true,
			DialTLSContext: func(ctx context.Context, network, addr string, _ *tls.Config) (net.Conn, error) {
				var d net.Dialer
				return d.DialContext(ctx, network, addr)
			},
		},
	}

	resp, err := client.Do(req)
	if err != nil {
		return nil, nil, err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, nil, err
	}

	// clone the response metadata we need after body close
	cloned := &http.Response{
		StatusCode: resp.StatusCode,
		Header:     resp.Header.Clone(),
		Trailer:    resp.Trailer.Clone(),
	}
	return cloned, body, nil
}
