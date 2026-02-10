//go:build e2e

package tracing

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

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

var (
	setupManifest        = filepath.Join(fsutils.MustGetThisDir(), "testdata", "setup.yaml")
	tracingSetupManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "tracing.yaml")

	// setup manifests applied before the test
	setup = base.TestCase{
		Manifests: []string{
			setupManifest,
		},
	}

	testCases = map[string]*base.TestCase{
		"TestOTelTracing": {
			Manifests: []string{
				tracingSetupManifest,
			},
		},
	}
)

// testingSuite is a suite of agentgateway tracing tests
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

// testOTelTracing makes a request to the httpbin service
// and checks if the collector pod logs contain the expected lines.
func (s *testingSuite) testOTelTracing() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwPolicyCondition(s.Ctx, "agw", "agentgateway-base", "Accepted", metav1.ConditionTrue)

	// The headerValue passed is used to differentiate between multiple calls by identifying a unique trace per call
	headerValue := fmt.Sprintf("%v", rand.Intn(10000)) //nolint:gosec // G404: Using math/rand for test trace identification
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

		// fetch the collector pod logs
		pods, err := s.TestInstallation.Actions.Kubectl().GetPodsInNsWithLabel(
			s.Ctx,
			"default",
			"app.kubernetes.io/name=opentelemetry-collector",
		)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to get collector pods")
		g.Expect(pods).NotTo(gomega.BeEmpty(), "No collector pods found")

		logs, err := s.TestInstallation.Actions.Kubectl().GetContainerLogs(s.Ctx, "default", pods[0])
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to get pod logs")

		// Check if the logs match the patterns
		mustContain := []string{
			`-> http.method: Str(GET)`,
			// Custom resources configured in the policy
			`-> deployment.environment.name: Str(production)`,
			`-> service.version: Str(test)`,
			// Custom tag passed in the config
			`-> custom: Str(literal)`,
			// Custom tag fetched from the request header
			fmt.Sprintf("-> request: Str(%s)", headerValue),
		}

		var missing []string
		for _, line := range mustContain {
			if !strings.Contains(logs, line) {
				missing = append(missing, line)
			}
		}
		g.Expect(missing).To(gomega.BeEmpty(), "missing required trace lines")

		// Assert URL-related fields using the semantic convention emitted by the debug exporter.
		hasHTTPURL := strings.Contains(logs, `-> url.scheme: Str(http)`) &&
			strings.Contains(logs, `-> http.host: Str(www.example.com)`) &&
			strings.Contains(logs, `-> http.path: Str(/status/200)`)
		g.Expect(hasHTTPURL).To(gomega.BeTrue(), "missing expected URL/host/path attributes in traces")

		g.Expect(strings.Contains(logs, `-> http.status: Int(200)`)).To(gomega.BeTrue(), "missing expected HTTP status attribute in traces")
	}, time.Second*60, time.Second*15, "should find traces in collector pod logs").Should(gomega.Succeed())
}
