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
	istiolog "istio.io/istio/pkg/log"
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

func WithMinGwApiVersion(minVersions map[GwApiChannel]*GwApiVersion) SuiteOption {
	return func(s *Test) {
		s.MinGwApiVersion = minVersions
	}
}

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

func (s Test) E2EContext() context.Context {
	return s.Ctx
}

func (s Test) E2EClusterContext() *cluster.Context {
	return s.TestInstallation.ClusterContext
}

func (s *Test) Run(name string, f func(t Test)) bool {
	s.T.Helper()
	return s.T.Run(name, func(t *testing.T) {
		child := *s
		child.T = t
		f(child)
	})
}
func init() {
	for _, s := range istiolog.Scopes() {
		s.SetOutputLevel(istiolog.DebugLevel)
	}
	istiolog.EnableKlogWithVerbosity(6)
}

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

func (s *Test) GatewayReady(name, namespace string) {
	s.Helper()
	assertions.EventuallyGatewayCondition(s, name, namespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	assertions.EventuallyGatewayCondition(s, name, namespace, gwv1.GatewayConditionAccepted, metav1.ConditionTrue)
}

func (s *Test) HTTPRouteAccepted(name, namespace string) {
	s.Helper()
	assertions.EventuallyHTTPRouteCondition(s, name, namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)
}

func (s *Test) Send(target string, expect *testmatchers.HttpResponse, opts ...curl.Option) {
	s.Helper()
	BaseGateway.Send(s, expect, append(targetOptions(s, target), opts...)...)
}

func Expect(status int) *testmatchers.HttpResponse {
	return &testmatchers.HttpResponse{StatusCode: status}
}

func ExpectOK() *testmatchers.HttpResponse {
	return Expect(http.StatusOK)
}

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
		raw = "http://" + strings.TrimPrefix(raw, "/")
	}
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatalf("invalid request target %q: %v", target, err)
	}
	if u.Host == "" {
		t.Fatalf("invalid request target %q: missing host", target)
	}

	path := strings.TrimPrefix(u.EscapedPath(), "/")
	if u.RawQuery != "" {
		path += "?" + u.RawQuery
	}
	opts := []curl.Option{
		curl.WithHostHeader(u.Host),
		curl.WithPath(path),
	}
	if u.Scheme != "" {
		opts = append(opts, curl.WithScheme(u.Scheme))
	}
	return opts
}
