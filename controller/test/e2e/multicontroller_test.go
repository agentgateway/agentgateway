//go:build e2e

package e2e_test

import (
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/test/util/retry"
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

	assertGatewayClassController(t, "agentgateway", base.AgentgatewayControllerName)

	secondary := e2e.CreateSharedTestInstallation(
		secondaryControllerNamespace,
		e2e.ManifestPath("agent-gateway-secondary.yaml"),
	)
	secondary.ExtraHelmArgs = primaryImageHelmArgs(t)
	testutils.Cleanup(t, func() {
		secondary.UninstallAgentgatewayCore(t.Ctx, tt)
		secondary.Finalize()
	})
	secondary.InstallAgentgatewayCoreFromLocalChart(t.Ctx, tt)

	assertGatewayClassController(t, secondaryGatewayClass, secondaryControllerName)
	assertGatewayClassController(t, "agentgateway", base.AgentgatewayControllerName)

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

	assertGatewayClassController(t, "agentgateway", base.AgentgatewayControllerName)
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
	).Get(t.Ctx, "agentgateway", metav1.GetOptions{})
	if err != nil {
		t.Fatalf("get primary controller deployment: %v", err)
	}
	if len(deployment.Spec.Template.Spec.Containers) == 0 {
		t.Fatal("primary controller deployment has no containers")
	}

	image := deployment.Spec.Template.Spec.Containers[0].Image
	lastSlash := strings.LastIndex(image, "/")
	lastColon := strings.LastIndex(image, ":")
	if lastSlash < 0 || lastColon <= lastSlash || lastColon == len(image)-1 {
		t.Fatalf("primary controller image %q does not contain a registry, repository, and tag", image)
	}

	return []string{
		"--set-string", "image.registry=" + image[:lastSlash],
		"--set-string", "image.tag=" + image[lastColon+1:],
	}
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
