//go:build e2e

package extauth

import (
	"context"
	"net/http"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

// testingSuite is a suite of tests for ExtAuth functionality
type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	// Define the setup TestCase for common resources
	setupTestCase := base.TestCase{
		Manifests: []string{
			simpleServiceManifest,
			extAuthManifest,
		},
	}

	// Define test-specific TestCases
	testCases := map[string]*base.TestCase{
		"TestExtAuthPolicy": {
			Manifests: []string{
				securedGatewayPolicyManifest,
				insecureRouteManifest,
			},
		},
		"TestRouteTargetedExtAuthPolicy": {
			Manifests: []string{
				securedRouteManifest,
				insecureRouteManifest,
			},
		},
		"TestExtAuthPolicyMissingBackendRef": {
			Manifests: []string{
				securedRouteMissingRefManifest,
			},
		},
	}

	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setupTestCase, testCases),
	}
}

// TestExtAuthPolicy tests the basic ExtAuth functionality with header-based allow/deny
// Checks for gateway level auth with route level opt out
func (s *testingSuite) TestExtAuthPolicy() {
	testCases := []struct {
		name                         string
		headers                      map[string]string
		hostname                     string
		expectedStatus               int
		expectedUpstreamBodyContents string
	}{
		{
			name: "request allowed with allow header",
			headers: map[string]string{
				"x-ext-authz": "allow",
			},
			hostname:                     "example.com",
			expectedStatus:               http.StatusOK,
			expectedUpstreamBodyContents: "X-Ext-Authz-Check-Result",
		},
		{
			name:           "request denied without allow header",
			headers:        map[string]string{},
			hostname:       "example.com",
			expectedStatus: http.StatusForbidden,
		},
		{
			name:     "request denied with deny header",
			hostname: "example.com",
			headers: map[string]string{
				"x-ext-authz": "deny",
			},
			expectedStatus: http.StatusForbidden,
		},
		// TODO(npolshak): re-enable once we can disable filters on agentgateway: https://github.com/agentgateway/agentgateway/issues/330
		//{
		//	name:           "request allowed on insecure route",
		//	hostname:       "insecureroute.com",
		//	expectedStatus: http.StatusOK,
		//},
	}

	for _, tc := range testCases {
		s.Run(tc.name, func() {
			// Build curl options
			opts := []curl.Option{
				curl.WithHostHeader(tc.hostname),
			}

			// Add test-specific headers
			for k, v := range tc.headers {
				opts = append(opts, curl.WithHeader(k, v))
			}

			// Test the request
			common.BaseGateway.Send(
				s.T(),
				&testmatchers.HttpResponse{
					StatusCode: tc.expectedStatus,
					Body:       gomega.ContainSubstring(tc.expectedUpstreamBodyContents),
				},
				opts...)
		})
	}
}

// TestRouteTargetedExtAuthPolicy tests route level only extauth
func (s *testingSuite) TestRouteTargetedExtAuthPolicy() {
	testCases := []struct {
		name                         string
		headers                      map[string]string
		hostname                     string
		expectedStatus               int
		expectedUpstreamBodyContents string
	}{
		{
			name:           "request allowed by default",
			headers:        map[string]string{},
			hostname:       "example.com",
			expectedStatus: http.StatusOK,
		},
		// TODO(npolshak): re-enable once we can disable filters on agentgateway: https://github.com/agentgateway/agentgateway/issues/330
		//{
		//	name:           "request allowed on insecure route",
		//	hostname:       "insecureroute.com",
		//	expectedStatus: http.StatusOK,
		//},
		{
			name: "request allowed with allow header on secured route",
			headers: map[string]string{
				"x-ext-authz": "allow",
			},
			hostname:                     "secureroute.com",
			expectedStatus:               http.StatusOK,
			expectedUpstreamBodyContents: "X-Ext-Authz-Check-Result",
		},
		{
			name:           "request denied without header on secured route",
			hostname:       "secureroute.com",
			headers:        map[string]string{},
			expectedStatus: http.StatusForbidden,
		},
	}

	for _, tc := range testCases {
		s.Run(tc.name, func() {
			// Build curl options
			opts := []curl.Option{
				curl.WithHostHeader(tc.hostname),
			}

			// Add test-specific headers
			for k, v := range tc.headers {
				opts = append(opts, curl.WithHeader(k, v))
			}

			// Test the request
			common.BaseGateway.Send(
				s.T(),
				&testmatchers.HttpResponse{
					StatusCode: tc.expectedStatus,
					Body:       gomega.ContainSubstring(tc.expectedUpstreamBodyContents),
				},
				opts...)
		})
	}
}

// TestExtAuthPolicyMissingBackendRef tests behavior when the ExtAuth policy is missing a backendRef
func (s *testingSuite) TestExtAuthPolicyMissingBackendRef() {
	testCases := []struct {
		name                         string
		headers                      map[string]string
		hostname                     string
		expectedStatus               int
		expectedUpstreamBodyContents string
	}{
		{
			name:           "request denied for invalid extauth policy due to missing backendRef",
			hostname:       "secureroute.com",
			headers:        map[string]string{},
			expectedStatus: http.StatusForbidden,
		},
	}

	for _, tc := range testCases {
		s.Run(tc.name, func() {
			// Build curl options
			opts := []curl.Option{
				curl.WithHostHeader(tc.hostname),
			}

			// Add test-specific headers
			for k, v := range tc.headers {
				opts = append(opts, curl.WithHeader(k, v))
			}

			// Test the request
			common.BaseGateway.Send(
				s.T(),
				&testmatchers.HttpResponse{
					StatusCode: tc.expectedStatus,
					Body:       gomega.ContainSubstring(tc.expectedUpstreamBodyContents),
				},
				opts...)
		})
	}
}
