//go:build e2e

package configmap

import (
	"context"
	"path/filepath"
	"time"

	"github.com/stretchr/testify/suite"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

var (
	tracingSetupManifest     = filepath.Join(fsutils.MustGetThisDir(), "testdata", "tracing-setup.yaml")
	tracingConfigMapManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "tracing-configmap.yaml")

	tracingAgentGatewayDeploymentMeta = metav1.ObjectMeta{
		Name:      "agentgateway",
		Namespace: "default",
	}

	// configmap manifests applied before the test
	tracingConfigMapSetup = base.TestCase{
		Manifests: []string{
			tracingSetupManifest,
		},
	}

	tracingConfigMapTest = base.TestCase{
		Manifests: []string{
			tracingConfigMapManifest,
		},
	}

	testCases = map[string]*base.TestCase{
		"TestTracingConfigMap": &tracingConfigMapTest,
	}
)

// testingSuite is a suite of agentgateway configmap tests
type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		base.NewBaseTestingSuite(ctx, testInst, tracingConfigMapSetup, testCases),
	}
}

// TestTracingConfigMap tests that agentgateway properly applies tracing configuration from ConfigMap
func (s *testingSuite) TestTracingConfigMap() {
	s.T().Log("Testing tracing ConfigMap configuration")

	// Ensure the ConfigMap exists before checking the gateway
	s.verifyConfigMapExists("agentgateway-config", "default")

	s.waitForAgentgatewayPodsRunning()

	s.verifyConfigMapMountedInDeployment("agentgateway-config", tracingAgentGatewayDeploymentMeta)

	// Verify that the tracing configuration is actually loaded and active
	s.verifyTracingConfigurationActive(tracingAgentGatewayDeploymentMeta)
}

// verifyConfigMapExists ensures the ConfigMap exists before proceeding
func (s *testingSuite) verifyConfigMapExists(name, namespace string) {
	s.T().Logf("Verifying ConfigMap %s exists in namespace %s", name, namespace)
	s.TestInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.T().Context(),
		&corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{
				Name:      name,
				Namespace: "default",
			},
		},
	)
}

// waitForAgentgatewayPodsRunning waits for the agentgateway pods to be running
func (s *testingSuite) waitForAgentgatewayPodsRunning() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(
		s.T().Context(),
		"default",
		metav1.ListOptions{LabelSelector: defaults.WellKnownAppLabel + "=agentgateway"},
		60*time.Second,
	)
}

// verifyConfigMapMountedInDeployment is a helper function that verifies a specific ConfigMap
// is mounted as config-volume in the agentgateway deployment
func (s *testingSuite) verifyConfigMapMountedInDeployment(expectedConfigMapName string, deploymentMeta metav1.ObjectMeta) {
	deploymentObj := &appsv1.Deployment{}
	err := s.TestInstallation.ClusterContext.Client.Get(
		s.T().Context(),
		client.ObjectKey{
			Namespace: deploymentMeta.Namespace,
			Name:      deploymentMeta.Name,
		},
		deploymentObj,
	)
	s.Require().NoError(err)

	found := false
	for _, volume := range deploymentObj.Spec.Template.Spec.Volumes {
		if volume.Name == "config-volume" && volume.ConfigMap != nil {
			if volume.ConfigMap.Name == expectedConfigMapName {
				found = true
				break
			}
		}
	}
	s.Require().True(found, "ConfigMap %s should be mounted as config-volume", expectedConfigMapName)
}

// verifyTracingConfigurationActive checks that the tracing configuration from ConfigMap is accepted by agentgateway
func (s *testingSuite) verifyTracingConfigurationActive(deploymentMeta metav1.ObjectMeta) {
	pods, err := s.TestInstallation.Actions.Kubectl().GetPodsInNsWithLabel(
		s.T().Context(),
		deploymentMeta.Namespace,
		defaults.WellKnownAppLabel+"=agentgateway",
	)
	s.Require().NoError(err)
	s.Require().NotEmpty(pods, "No agentgateway pods found")

	s.Require().Eventually(func() bool {
		logs, err := s.TestInstallation.Actions.Kubectl().GetContainerLogs(
			s.T().Context(),
			deploymentMeta.Namespace,
			pods[0],
		)
		s.Require().NoError(err, "Failed to get pod logs")

		expectedEndpoint := "endpoint: http://jaeger-collector.observability.svc.cluster.local:4317"
		s.Require().Contains(logs, expectedEndpoint,
			"Tracing endpoint %s from ConfigMap should be present in pod logs", expectedEndpoint)

		return true
	}, 60*time.Second, 5*time.Second)
}
