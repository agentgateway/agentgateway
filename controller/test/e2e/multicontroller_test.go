//go:build e2e

package e2e_test

import (
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	e2e "github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

const (
	secondaryControllerName      = "test.agentgateway.dev/secondary"
	secondaryGatewayClass        = "agentgateway-secondary"
	secondaryControllerNamespace = "agentgateway-secondary"
	secondaryWorkloadNamespace   = "agentgateway-secondary-workload"
)

func TestMultipleControllers(tt *testing.T) {
	t := New(tt)

	assertGatewayClassController(t, base.AgentgatewayClassName, base.AgentgatewayControllerName)

	secondary := e2e.CreateSharedTestInstallation(
		secondaryControllerNamespace,
		e2e.ManifestPath("agent-gateway-secondary.yaml"),
		e2e.WithManagedLifecycle(),
	)
	secondary.ExtraHelmArgs = primaryImageHelmArgs(t)
	testutils.Cleanup(t, func() {
		secondary.UninstallAgentgatewayCore(t.Ctx, tt)
		secondary.Finalize()
	})
	secondary.InstallAgentgatewayCoreFromLocalChart(t.Ctx, tt)

	assertGatewayClassController(t, secondaryGatewayClass, secondaryControllerName)
	assertGatewayClassController(t, base.AgentgatewayClassName, base.AgentgatewayControllerName)

	t.Apply(manifest("multicontroller", "setup.yaml"))
	t.GatewayReady("secondary-gateway", secondaryWorkloadNamespace)
	t.HTTPRouteAccepted("secondary-route", secondaryWorkloadNamespace)
	t.HTTPRouteAccepted("primary-route", base.Namespace)

	assertRouteController(t, "secondary-route", secondaryWorkloadNamespace, secondaryControllerName)
	assertRouteController(t, "primary-route", base.Namespace, base.AgentgatewayControllerName)
	assertGatewayUsesControlPlane(t, "secondary-gateway", secondaryWorkloadNamespace, secondaryControllerNamespace)
	applyWithoutResourceWait(t, manifest("multicontroller", "unselected.yaml"))
	assertGatewayNotProvisioned(t, "unselected-gateway", "agentgateway-unselected")

	secondaryName := types.NamespacedName{Name: "secondary-gateway", Namespace: secondaryWorkloadNamespace}
	secondaryGateway := base.Gateway{
		NamespacedName: secondaryName,
		Address:        base.ResolveGatewayAddress(t, t.Ctx, secondary, secondaryName),
	}
	secondaryGateway.Send(
		t,
		base.ExpectOK(),
		curl.WithHostHeader("secondary.multicontroller.example"),
		curl.WithPath("/status/200"),
	)

	base.BaseGateway.Send(
		t,
		base.ExpectOK(),
		curl.WithHostHeader("primary.multicontroller.example"),
		curl.WithPath("/status/200"),
	)

	assertGatewayClassController(t, base.AgentgatewayClassName, base.AgentgatewayControllerName)
}

func applyWithoutResourceWait(t base.Test, manifest string) {
	t.Helper()
	err := t.TestInstallation.ClusterContext.Client.ApplyYAMLFiles("", manifest)
	assert.NoError(t, err)
	testutils.Cleanup(t, func() {
		err := t.TestInstallation.ClusterContext.Client.DeleteYAMLFiles("", manifest)
		assert.NoError(t, err)
	})
}

func primaryImageHelmArgs(t base.Test) []string {
	t.Helper()
	deployment, err := t.TestInstallation.ClusterContext.Client.Kube().AppsV1().Deployments(
		t.TestInstallation.InstallNamespace,
	).Get(t.Ctx, base.AgentgatewayControllerDeploymentName, metav1.GetOptions{})
	if err != nil {
		t.Fatalf("get primary controller deployment: %v", err)
	}
	if len(deployment.Spec.Template.Spec.Containers) == 0 {
		t.Fatal("primary controller deployment has no containers")
	}

	controller := deployment.Spec.Template.Spec.Containers[0]
	helmArgs, err := installationImageHelmArgs(controller.Image, controller.Env)
	if err != nil {
		t.Fatal(err)
	}
	return helmArgs
}

func installationImageHelmArgs(image string, env []corev1.EnvVar) ([]string, error) {
	lastSlash := strings.LastIndex(image, "/")
	lastColon := strings.LastIndex(image, ":")
	if lastSlash < 0 || lastColon <= lastSlash || lastColon == len(image)-1 {
		return nil, fmt.Errorf("primary controller image %q does not contain a registry, repository, and tag", image)
	}

	registry := image[:lastSlash]
	tag := image[lastColon+1:]
	args := []string{
		"--set-string", "controller.image.registry=" + registry,
		"--set-string", "controller.image.repository=" + image[lastSlash+1:lastColon],
		"--set-string", "controller.image.tag=" + tag,
	}

	proxyImage := map[string]string{
		"registry":   registry,
		"repository": "",
		"tag":        tag,
	}
	for _, variable := range env {
		switch variable.Name {
		case "AGW_PROXY_IMAGE_REGISTRY":
			proxyImage["registry"] = variable.Value
		case "AGW_PROXY_IMAGE_REPOSITORY":
			proxyImage["repository"] = variable.Value
		case "AGW_PROXY_IMAGE_TAG":
			proxyImage["tag"] = variable.Value
		}
	}
	if proxyImage["repository"] == "" {
		return nil, fmt.Errorf("primary controller deployment does not configure AGW_PROXY_IMAGE_REPOSITORY")
	}

	args = append(args,
		"--set-string", "proxy.image.registry="+proxyImage["registry"],
		"--set-string", "proxy.image.repository="+proxyImage["repository"],
		"--set-string", "proxy.image.tag="+proxyImage["tag"],
	)
	return args, nil
}

func TestInstallationImageHelmArgs(t *testing.T) {
	tests := []struct {
		name  string
		image string
		env   []corev1.EnvVar
		want  []string
	}{
		{
			name:  "default registry",
			image: "cr.agentgateway.dev/agentgateway/controller:v1.2.3",
			env: []corev1.EnvVar{
				{Name: "AGW_PROXY_IMAGE_REGISTRY", Value: "cr.agentgateway.dev/agentgateway"},
				{Name: "AGW_PROXY_IMAGE_REPOSITORY", Value: "agentgateway"},
				{Name: "AGW_PROXY_IMAGE_TAG", Value: "v1.2.3"},
			},
			want: []string{
				"--set-string", "controller.image.registry=cr.agentgateway.dev/agentgateway",
				"--set-string", "controller.image.repository=controller",
				"--set-string", "controller.image.tag=v1.2.3",
				"--set-string", "proxy.image.registry=cr.agentgateway.dev/agentgateway",
				"--set-string", "proxy.image.repository=agentgateway",
				"--set-string", "proxy.image.tag=v1.2.3",
			},
		},
		{
			name:  "registry with port and alternate repository",
			image: "localhost:5000/team/enterprise-controller:test",
			env: []corev1.EnvVar{
				{Name: "AGW_PROXY_IMAGE_REGISTRY", Value: "localhost:5000/team"},
				{Name: "AGW_PROXY_IMAGE_REPOSITORY", Value: "enterprise-proxy"},
				{Name: "AGW_PROXY_IMAGE_TAG", Value: "test"},
			},
			want: []string{
				"--set-string", "controller.image.registry=localhost:5000/team",
				"--set-string", "controller.image.repository=enterprise-controller",
				"--set-string", "controller.image.tag=test",
				"--set-string", "proxy.image.registry=localhost:5000/team",
				"--set-string", "proxy.image.repository=enterprise-proxy",
				"--set-string", "proxy.image.tag=test",
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := installationImageHelmArgs(tt.image, tt.env)
			assert.NoError(t, err)
			assert.Equal(t, tt.want, got)
		})
	}

	_, err := installationImageHelmArgs("controller:latest", nil)
	assert.EqualError(t, err, `primary controller image "controller:latest" does not contain a registry, repository, and tag`)

	_, err = installationImageHelmArgs("registry.example/controller:latest", nil)
	assert.EqualError(t, err, "primary controller deployment does not configure AGW_PROXY_IMAGE_REPOSITORY")
}

func assertGatewayClassController(t base.Test, name, controllerName string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		gatewayClass := &gwv1.GatewayClass{}
		if err := t.TestInstallation.ClusterContext.ControllerClient.Get(
			t.Ctx,
			types.NamespacedName{Name: name},
			gatewayClass,
		); err != nil {
			return err
		}
		if string(gatewayClass.Spec.ControllerName) != controllerName {
			return fmt.Errorf(
				"GatewayClass %s controllerName=%q, want %q",
				name,
				gatewayClass.Spec.ControllerName,
				controllerName,
			)
		}
		return nil
	})
}

func assertRouteController(t base.Test, name, namespace, controllerName string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		route := &gwv1.HTTPRoute{}
		if err := t.TestInstallation.ClusterContext.ControllerClient.Get(
			t.Ctx,
			types.NamespacedName{Name: name, Namespace: namespace},
			route,
		); err != nil {
			return err
		}
		for _, parent := range route.Status.Parents {
			if string(parent.ControllerName) != controllerName {
				continue
			}
			for _, condition := range parent.Conditions {
				if condition.Type == string(gwv1.RouteConditionAccepted) && condition.Status == metav1.ConditionTrue {
					return nil
				}
			}
		}
		return fmt.Errorf("HTTPRoute %s/%s has no Accepted status from controller %q", namespace, name, controllerName)
	})
}

func assertGatewayUsesControlPlane(t base.Test, name, namespace, controllerNamespace string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		deployment, err := t.TestInstallation.ClusterContext.Client.Kube().AppsV1().Deployments(namespace).Get(
			t.Ctx,
			name,
			metav1.GetOptions{},
		)
		if err != nil {
			return err
		}
		if len(deployment.Spec.Template.Spec.Containers) == 0 {
			return fmt.Errorf("Gateway %s/%s deployment has no containers", namespace, name)
		}
		for _, env := range deployment.Spec.Template.Spec.Containers[0].Env {
			if env.Name != "XDS_ADDRESS" {
				continue
			}
			if strings.Contains(env.Value, "."+controllerNamespace+".svc.") {
				return nil
			}
			return fmt.Errorf("Gateway %s/%s XDS_ADDRESS=%q does not target namespace %q", namespace, name, env.Value, controllerNamespace)
		}
		return fmt.Errorf("Gateway %s/%s deployment has no XDS_ADDRESS", namespace, name)
	})
}

func assertGatewayNotProvisioned(t base.Test, name, namespace string) {
	t.Helper()
	assert.Never(t, func() bool {
		_, err := t.TestInstallation.ClusterContext.Client.Kube().AppsV1().Deployments(namespace).Get(
			t.Ctx,
			name,
			metav1.GetOptions{},
		)
		return !apierrors.IsNotFound(err)
	}, 5*time.Second, 100*time.Millisecond, "unselected Gateway %s/%s was provisioned", namespace, name)
}
