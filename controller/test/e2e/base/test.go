//go:build e2e

package base

import (
	"context"
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/Masterminds/semver/v3"
	"github.com/onsi/gomega"
	"istio.io/istio/pkg/config/crd"
	"istio.io/istio/pkg/test"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/cluster"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const (
	Namespace         = "agentgateway-base"
	WellKnownAppLabel = "app.kubernetes.io/name"
)

type Test struct {
	*testing.T
	Ctx              context.Context
	TestInstallation *e2e.TestInstallation

	validator    *crd.Validator
	gwApiVersion *semver.Version
	gwApiChannel GwApiChannel

	MinGwApiVersion map[GwApiChannel]*GwApiVersion
}

type SuiteOption func(*Test)

// WithMinGwApiVersion skips the test when the cluster Gateway API CRDs are too old.
// Use it for tests that depend on newer Gateway API fields or behavior.
func WithMinGwApiVersion(minVersions map[GwApiChannel]*GwApiVersion) SuiteOption {
	return func(s *Test) {
		s.MinGwApiVersion = minVersions
	}
}

// NewTest builds the base test handle used by top-level package helpers.
// Test authors normally call the package-level New(tt) wrapper instead.
func NewTest(ctx context.Context, testInst *e2e.TestInstallation, t *testing.T, opts ...SuiteOption) Test {
	test := Test{
		T:                t,
		Ctx:              ctx,
		TestInstallation: testInst,
	}

	for _, opt := range opts {
		opt(&test)
	}

	return test
}

// E2EContext returns the shared e2e context for typed Kubernetes or helper calls.
func (s Test) E2EContext() context.Context {
	return s.Ctx
}

// E2EClusterContext returns the shared cluster handle for typed clients and low-level operations.
func (s Test) E2EClusterContext() *cluster.Context {
	return s.TestInstallation.ClusterContext
}

// Run creates a Go subtest that carries the same e2e installation and helper state.
// Prefer this over t.T.Run when nested tests need Apply, Send, or shared clients.
func (s *Test) Run(name string, f func(t Test)) bool {
	s.T.Helper()
	return s.T.Run(name, func(t *testing.T) {
		child := *s
		child.T = t
		f(child)
	})
}

// Setup initializes per-test helper state and skips early if SuiteOptions require it.
// Package-level New(tt) calls this automatically.
func (s *Test) Setup() {
	done := traceStep(s, "detected Gateway API version")
	s.detectAndCacheGwApiInfo()
	done()

	if s.ShouldSkip() {
		s.Skipf("Test requires Gateway API %s, but current is %s/%s", s.MinGwApiVersion, s.getCurrentGwApiChannel(), s.getCurrentGwApiVersion())
	}

	done = traceStep(s, "setup test helpers")
	s.setupHelpers()
	done()
}

// GatewayReady waits until a Gateway is accepted and programmed.
// Use this after applying a Gateway when a test needs to assert readiness explicitly.
func (s *Test) GatewayReady(name, namespace string) {
	s.Helper()
	assertions.EventuallyGatewayCondition(s, name, namespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	assertions.EventuallyGatewayCondition(s, name, namespace, gwv1.GatewayConditionAccepted, metav1.ConditionTrue)
}

// HTTPRouteAccepted waits until an HTTPRoute has Accepted=True.
// Use this when route status is part of the behavior under test.
func (s *Test) HTTPRouteAccepted(name, namespace string) {
	s.Helper()
	assertions.EventuallyHTTPRouteCondition(s, name, namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)
}

// Send sends a request through the shared base Gateway and retries until the expectation matches.
// Target may be a path ("/get"), host/path ("example.com/get"), or full URL; host is optional.
// Extra curl options can add headers, method, body, scheme, or other request settings.
func (s *Test) Send(target string, expect *testmatchers.HttpResponse, opts ...curl.Option) {
	s.Helper()
	BaseGateway.Send(s, expect, append(targetOptions(s, target), opts...)...)
}

// Expect builds a response matcher for a status code.
// Use with Send for simple assertions, for example t.Send("/status/404", base.Expect(404)).
func Expect(status int) *testmatchers.HttpResponse {
	return &testmatchers.HttpResponse{StatusCode: status}
}

// ExpectOK is shorthand for Expect(http.StatusOK).
func ExpectOK() *testmatchers.HttpResponse {
	return Expect(http.StatusOK)
}

func ExpectBody(body gomega.OmegaMatcher) *testmatchers.HttpResponse {
	return &testmatchers.HttpResponse{StatusCode: http.StatusOK, Body: body}
}

// ExpectForbidden expects HTTP 403 and a matching body.
// Use this for policy-denied requests where the response text is part of the assertion.
func ExpectForbidden(body gomega.OmegaMatcher) *testmatchers.HttpResponse {
	return &testmatchers.HttpResponse{
		StatusCode: http.StatusForbidden,
		Body:       body,
	}
}

func targetOptions(t test.Failer, target string) []curl.Option {
	t.Helper()
	if target == "" {
		t.Fatal("target must not be empty")
	}

	raw := target
	if !strings.Contains(raw, "://") {
		if after, ok := strings.CutPrefix(raw, "/"); ok {
			raw = "http:///" + after
		} else {
			raw = "http://" + strings.TrimPrefix(raw, "/")
		}
	}
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatalf("invalid request target %q: %v", target, err)
	}

	path := strings.TrimPrefix(u.EscapedPath(), "/")
	if u.RawQuery != "" {
		path += "?" + u.RawQuery
	}
	opts := []curl.Option{curl.WithPath(path)}
	if u.Host != "" {
		opts = append(opts, curl.WithHostHeader(u.Host))
	}
	if u.Scheme != "" {
		opts = append(opts, curl.WithScheme(u.Scheme))
	}
	return opts
}
