//go:build e2e

package zero_downtime_rollout

import (
	"context"
	"net/http"
	"path/filepath"
	"time"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var (
	serviceManifest      = filepath.Join(fsutils.MustGetThisDir(), "testdata", "service.yaml")
	agentgatewayManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "agentgateway.yaml")

	agentgatewayObjectMeta = metav1.ObjectMeta{
		Name:      "agentgw",
		Namespace: "default",
	}
)

type testingSuiteAgentgateway struct {
	*base.BaseTestingSuite
}

func NewTestingSuiteAgentgateway(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuiteAgentgateway{
		base.NewBaseTestingSuite(
			ctx,
			testInst,
			base.TestCase{
				Manifests: []string{serviceManifest},
			},
			map[string]*base.TestCase{
				"TestZeroDowntimeRolloutAgentgateway": {
					Manifests: []string{agentgatewayManifest, defaults.CurlPodManifest},
				},
			},
		),
	}
}

func (s *testingSuiteAgentgateway) TestZeroDowntimeRolloutAgentgateway() {
	// Ensure the agentgateway pod is up and running.
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(s.Ctx,
		agentgatewayObjectMeta.GetNamespace(), metav1.ListOptions{
			LabelSelector: defaults.WellKnownAppLabel + "=" + agentgatewayObjectMeta.GetName(),
		})

	s.TestInstallation.AssertionsT(s.T()).AssertEventualCurlResponse(
		s.Ctx,
		defaults.CurlPodExecOpt,
		[]curl.Option{
			curl.WithHost(kubeutils.ServiceFQDN(agentgatewayObjectMeta)),
			curl.WithHostHeader("example.com"),
		},
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
		})

	kCli := kubectl.NewCli()

	// Send traffic to the gateway pod while we restart the deployment.
	// Run this for 30s which is long enough to restart the deployment since there's no easy way
	// to stop this command once the test is over.
	// This executes 800 req @ 4 req/sec = 20s (3 * terminationGracePeriodSeconds (5) + buffer).
	// kubectl exec -n hey heyagw -- hey -disable-keepalive -c 4 -q 10 --cpus 1 -n 800 -m GET -t 1 -host example.com http://agentgw.default.svc.cluster.local:8080.
	args := []string{"exec", "-n", "hey", "heyagw", "--", "hey", "-disable-keepalive", "-c", "4", "-q", "10", "--cpus", "1", "-n", "800", "-m", "GET", "-t", "1", "-host", "example.com", "http://agentgw.default.svc.cluster.local:8080"}

	cmd := kCli.Command(s.Ctx, args...)

	if err := cmd.Start(); err != nil {
		s.T().Fatal("error starting command", err)
	}

	// Restart the deployment, twice.
	// There should be no downtime, since the gateway pod
	// should have readiness probes configured.
	err := kCli.RestartDeploymentAndWait(s.Ctx, "agentgw")
	s.Require().NoError(err)

	time.Sleep(time.Second)

	err = kCli.RestartDeploymentAndWait(s.Ctx, "agentgw")
	s.Require().NoError(err)

	if err := cmd.Wait(); err != nil {
		s.T().Fatal("error waiting for command to finish", err)
	}

	// Verify that there were no errors.
	s.Contains(string(cmd.Output()), "[200]	800 responses")
	s.NotContains(string(cmd.Output()), "Error distribution")
}
