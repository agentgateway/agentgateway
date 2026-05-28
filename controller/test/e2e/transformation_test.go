//go:build e2e

package e2e_test

import (
	"bytes"
	"context"
	"crypto/tls"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"

	"golang.org/x/net/http2"
	"istio.io/istio/pkg/test/util/retry"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestTransformation(tt *testing.T) {
	t := New(tt)

	t.Run("HTTPRoute", func(t base.Test) {
		testGatewayWithTransformedHTTPRoute(t)
	})
	t.Run("GRPCRoute", func(t base.Test) {
		testGatewayWithTransformedGRPCRoute(t)
	})
}

func testGatewayWithTransformedHTTPRoute(t base.Test) {
	t.Apply(
		transformManifest("transform-for-headers.yaml"),
		transformManifest("transform-for-body.yaml"),
		transformManifest("gateway-attached-transform.yaml"),
	)

	assertTransformGatewayReady(t)

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
			name:      "conditional set by request header",
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
			name:      "pull json info",
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
		t.Run(tc.name, func(t base.Test) {
			t.Send(fmt.Sprintf("example-%s.com", tc.routeName), tc.resp, tc.opts...)
		})
	}
}

func testGatewayWithTransformedGRPCRoute(t base.Test) {
	t.Apply(transformManifest("grpc-transformation.yaml"))

	assertTransformGatewayReady(t)

	const grpcRouteName = "example-route"
	assertions.EventuallyGRPCRouteCondition(t, grpcRouteName, base.Namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)
	assertions.EventuallyGRPCRouteCondition(t, grpcRouteName, base.Namespace, gwv1.RouteConditionResolvedRefs, metav1.ConditionTrue)
	assertions.EventuallyHTTPRouteCondition(t, grpcRouteName, base.Namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)
	assertions.EventuallyHTTPRouteCondition(t, grpcRouteName, base.Namespace, gwv1.RouteConditionResolvedRefs, metav1.ConditionTrue)

	const (
		expectedHostname        = "example.com"
		grpcMethodPath          = "/proto.EchoTestService/Echo"
		expectedResponseMetaKey = "x-grpc-response"
		expectedResponseMetaVal = "from-grpc"
	)

	retry.UntilSuccessOrFail(t, func() error {
		// nolint: bodyclose // false positive
		resp, _, err := sendH2CGrpcRequest(
			base.BaseGateway.Address,
			expectedHostname,
			grpcMethodPath,
			[]byte{0x0a, 0x05, 'h', 'e', 'l', 'l', 'o'},
		)
		if err != nil {
			return fmt.Errorf("grpc request failed: %w", err)
		}

		grpcStatus := resp.Trailer.Get("grpc-status")
		if resp.StatusCode != http.StatusOK || grpcStatus != "0" {
			return fmt.Errorf("unexpected grpc response status=%d grpc-status=%q headers=%v trailers=%v",
				resp.StatusCode, grpcStatus, resp.Header, resp.Trailer)
		}

		if resp.Header.Get(expectedResponseMetaKey) != expectedResponseMetaVal {
			return fmt.Errorf("missing transformed grpc response header %s=%s, got headers=%v",
				expectedResponseMetaKey, expectedResponseMetaVal, resp.Header)
		}

		return nil
	})

	t.Send(expectedHostname, &testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		NotHeaders: []string{
			"x-grpc-response",
		},
	})
}

func transformManifest(name string) string {
	return manifest("transformation", name)
}

func assertTransformGatewayReady(t base.Test) {
	assertions.EventuallyGatewayCondition(t,
		"gateway",
		base.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
	)
	assertions.EventuallyGatewayCondition(t,
		"gateway",
		base.Namespace,
		gwv1.GatewayConditionAccepted,
		metav1.ConditionTrue,
	)
}

func sendH2CGrpcRequest(address, authority, methodPath string, protobufPayload []byte) (*http.Response, []byte, error) {
	grpcFrame := make([]byte, 5+len(protobufPayload))
	grpcFrame[0] = 0
	binary.BigEndian.PutUint32(grpcFrame[1:5], uint32(len(protobufPayload))) //nolint:gosec // test payload is tiny
	copy(grpcFrame[5:], protobufPayload)

	targetAddress := address
	if !strings.Contains(address, ":") {
		targetAddress = fmt.Sprintf("%s:80", address)
	}
	url := fmt.Sprintf("http://%s%s", targetAddress, methodPath)
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

	cloned := &http.Response{
		StatusCode: resp.StatusCode,
		Header:     resp.Header.Clone(),
		Trailer:    resp.Trailer.Clone(),
	}
	return cloned, body, nil
}
