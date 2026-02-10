//go:build e2e

package configmap

import (
	"context"
	"path/filepath"
	"strings"
	"time"

	"github.com/stretchr/testify/suite"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/defaults"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

var (
	tracingSetupManifest     = filepath.Join(fsutils.MustGetThisDir(), "testdata", "tracing-setup.yaml")
	tracingConfigMapManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "tracing-configmap.yaml")

	sharedGatewayDeploymentMeta = metav1.ObjectMeta{
		Name:      "gateway",
		Namespace: "agentgateway-base",
	}

	agentgatewayClassObjectMeta = metav1.ObjectMeta{
		Name: "agentgateway",
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

	restoreGatewayClass := s.setGatewayClassParametersRef("tracing-config", "default")
	defer restoreGatewayClass()

	s.waitForAgentgatewayPodsRunning(sharedGatewayDeploymentMeta)

	s.verifyConfigMapMountedInDeployment("agentgateway-config", sharedGatewayDeploymentMeta)

	// Verify that the tracing configuration is actually loaded and active
	s.verifyTracingConfigurationActive(sharedGatewayDeploymentMeta)
}

// verifyConfigMapExists ensures the ConfigMap exists before proceeding
func (s *testingSuite) verifyConfigMapExists(name, namespace string) {
	s.T().Logf("Verifying ConfigMap %s exists in namespace %s", name, namespace)
	s.TestInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.T().Context(),
		&corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{
				Name:      name,
				Namespace: namespace,
			},
		},
	)
}

// waitForAgentgatewayPodsRunning waits for the agentgateway pods to be running
func (s *testingSuite) waitForAgentgatewayPodsRunning(deploymentMeta metav1.ObjectMeta) {
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(
		s.T().Context(),
		deploymentMeta.Namespace,
		metav1.ListOptions{LabelSelector: defaults.WellKnownAppLabel + "=" + deploymentMeta.Name},
		60*time.Second,
	)
}

// verifyConfigMapMountedInDeployment is a helper function that verifies a specific ConfigMap
// is mounted as config-volume in the agentgateway deployment
func (s *testingSuite) verifyConfigMapMountedInDeployment(expectedConfigMapName string, deploymentMeta metav1.ObjectMeta) {
	s.Require().Eventually(func() bool {
		deploymentObj := &appsv1.Deployment{}
		err := s.TestInstallation.ClusterContext.Client.Get(
			s.T().Context(),
			client.ObjectKey{
				Namespace: deploymentMeta.Namespace,
				Name:      deploymentMeta.Name,
			},
			deploymentObj,
		)
		if err != nil {
			return false
		}

		for _, volume := range deploymentObj.Spec.Template.Spec.Volumes {
			if volume.Name == "config-volume" && volume.ConfigMap != nil && volume.ConfigMap.Name == expectedConfigMapName {
				return true
			}
		}
		return false
	}, 60*time.Second, 5*time.Second, "ConfigMap %s should be mounted as config-volume", expectedConfigMapName)
}

// verifyTracingConfigurationActive checks that the tracing configuration from ConfigMap is accepted by agentgateway
func (s *testingSuite) verifyTracingConfigurationActive(deploymentMeta metav1.ObjectMeta) {
	expectedEndpoint := "endpoint: http://jaeger-collector.observability.svc.cluster.local:4317"
	s.Require().Eventually(func() bool {
		pods, err := s.TestInstallation.Actions.Kubectl().GetPodsInNsWithLabel(
			s.T().Context(),
			deploymentMeta.Namespace,
			defaults.WellKnownAppLabel+"="+deploymentMeta.Name,
		)
		if err != nil || len(pods) == 0 {
			return false
		}

		for _, pod := range pods {
			logs, err := s.TestInstallation.Actions.Kubectl().GetContainerLogs(
				s.T().Context(),
				deploymentMeta.Namespace,
				pod,
			)
			if err != nil {
				continue
			}
			if strings.Contains(logs, expectedEndpoint) {
				return true
			}
		}
		return false
	}, 60*time.Second, 5*time.Second, "Tracing endpoint %s from ConfigMap should be present in pod logs", expectedEndpoint)
}

func (s *testingSuite) setGatewayClassParametersRef(paramsName, paramsNamespace string) func() {
	key := client.ObjectKey{Name: agentgatewayClassObjectMeta.Name}
	gatewayClass := &gwv1.GatewayClass{}
	err := s.TestInstallation.ClusterContext.Client.Get(s.T().Context(), key, gatewayClass)
	s.Require().NoError(err)

	var originalRef *gwv1.ParametersReference
	if gatewayClass.Spec.ParametersRef != nil {
		ref := *gatewayClass.Spec.ParametersRef
		originalRef = &ref
	}

	paramsNamespaceRef := gwv1.Namespace(paramsNamespace)
	gatewayClass.Spec.ParametersRef = &gwv1.ParametersReference{
		Group:     gwv1.Group("agentgateway.dev"),
		Kind:      gwv1.Kind("AgentgatewayParameters"),
		Name:      paramsName,
		Namespace: &paramsNamespaceRef,
	}
	err = s.TestInstallation.ClusterContext.Client.Update(s.T().Context(), gatewayClass)
	s.Require().NoError(err)

	return func() {
		restore := &gwv1.GatewayClass{}
		err := s.TestInstallation.ClusterContext.Client.Get(s.T().Context(), key, restore)
		s.Require().NoError(err)
		restore.Spec.ParametersRef = originalRef
		err = s.TestInstallation.ClusterContext.Client.Update(s.T().Context(), restore)
		s.Require().NoError(err)
	}
}
