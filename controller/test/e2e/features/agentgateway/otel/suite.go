//go:build e2e

package otel

import (
	"context"
	"fmt"
	"math/rand"
	"path/filepath"
	"strings"
	"time"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

var (
	setupManifest         = filepath.Join(fsutils.MustGetThisDir(), "testdata", "setup.yaml")
	tracingManifest       = filepath.Join(fsutils.MustGetThisDir(), "testdata", "tracing.yaml")
	accessLogOtlpManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "accesslog-otlp.yaml")
	collectorLogTimeout   = 20 * time.Second
	collectorLogPoll      = 500 * time.Millisecond

	setup = base.TestCase{
		Manifests: []string{
			setupManifest,
		},
	}

	testCases = map[string]*base.TestCase{
		"TestOTelTracing": {
			Manifests: []string{
				tracingManifest,
			},
		},
		"TestOTelAccessLog": {
			Manifests: []string{
				accessLogOtlpManifest,
			},
		},
	}
)

type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

func (s *testingSuite) TestOTelTracing() {
	s.testOTelTracing()
}

func (s *testingSuite) TestOTelAccessLog() {
	s.testOTelAccessLog()
}

// testOTelTracing makes a request to the httpbin service
// and checks if the collector pod logs contain the expected trace lines.
func (s *testingSuite) testOTelTracing() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwPolicyCondition(s.Ctx, "agw", "agentgateway-base", "Accepted", metav1.ConditionTrue)

	headerValue := fmt.Sprintf("%v", rand.Intn(10000)) //nolint:gosec // G404: Using math/rand for test trace identification
	collectorPod, err := s.getCollectorPod()
	s.Require().NoError(err, "Failed to resolve collector pod")

	s.TestInstallation.AssertionsT(s.T()).Gomega.Eventually(func(g gomega.Gomega) {
		common.BaseGateway.Send(
			s.T(),
			&matchers.HttpResponse{
				StatusCode: 200,
			},
			curl.WithHostHeader("www.example.com"),
			curl.WithHeader("x-header-tag", headerValue),
			curl.WithPath("/status/200"),
		)

		logs, err := s.getCollectorLogs(collectorPod)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to get collector pod logs")

		mustContain := []string{
			`-> http.method: Str(GET)`,
			`-> deployment.environment.name: Str(production)`,
			`-> service.version: Str(test)`,
			`-> custom: Str(literal)`,
			fmt.Sprintf("-> request: Str(%s)", headerValue),
		}

		var missing []string
		for _, line := range mustContain {
			if !strings.Contains(logs, line) {
				missing = append(missing, line)
			}
		}
		g.Expect(missing).To(gomega.BeEmpty(), "missing required trace lines")

		hasHTTPURL := strings.Contains(logs, `-> url.scheme: Str(http)`) &&
			strings.Contains(logs, `-> http.host: Str(www.example.com)`) &&
			strings.Contains(logs, `-> http.path: Str(/status/200)`)
		g.Expect(hasHTTPURL).To(gomega.BeTrue(), "missing expected URL/host/path attributes in traces")

		g.Expect(strings.Contains(logs, `-> http.status: Int(200)`)).To(gomega.BeTrue(), "missing expected HTTP status attribute in traces")
	}, collectorLogTimeout, collectorLogPoll, "should find traces in collector pod logs").Should(gomega.Succeed())
}

// testOTelAccessLog makes a request and checks the collector pod logs
// for OTLP access log records.
func (s *testingSuite) testOTelAccessLog() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwPolicyCondition(s.Ctx, "agw-accesslog", "agentgateway-base", "Accepted", metav1.ConditionTrue)

	collectorPod, err := s.getCollectorPod()
	s.Require().NoError(err, "Failed to resolve collector pod")

	s.TestInstallation.AssertionsT(s.T()).Gomega.Eventually(func(g gomega.Gomega) {
		common.BaseGateway.Send(
			s.T(),
			&matchers.HttpResponse{
				StatusCode: 200,
			},
			curl.WithHostHeader("www.example.com"),
			curl.WithPath("/status/200"),
		)

		logs, err := s.getCollectorLogs(collectorPod)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to get collector pod logs")

		mustContain := []string{
			`ScopeLogs`,
			`LogRecord #0`,
			`-> http.method: Str(GET)`,
			`-> http.path: Str(/status/200)`,
			`-> http.status: Int(200)`,
		}

		var missing []string
		for _, line := range mustContain {
			if !strings.Contains(logs, line) {
				missing = append(missing, line)
			}
		}
		g.Expect(missing).To(gomega.BeEmpty(), "missing required access log lines in collector output")
	}, collectorLogTimeout, collectorLogPoll, "should find access logs in collector pod logs").Should(gomega.Succeed())
}

func (s *testingSuite) getCollectorPod() (string, error) {
	pods, err := s.TestInstallation.Actions.Kubectl().GetPodsInNsWithLabel(
		s.Ctx,
		"default",
		"app.kubernetes.io/name=opentelemetry-collector",
	)
	if err != nil {
		return "", err
	}
	if len(pods) == 0 {
		return "", fmt.Errorf("no collector pods found")
	}

	return pods[0], nil
}

func (s *testingSuite) getCollectorLogs(pod string) (string, error) {
	return s.TestInstallation.Actions.Kubectl().GetContainerLogs(s.Ctx, "default", pod)
}
