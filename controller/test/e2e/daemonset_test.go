//go:build e2e

package e2e_test

import (
	"fmt"
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	discoveryv1 "k8s.io/api/discovery/v1"
	policyv1 "k8s.io/api/policy/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

const (
	daemonSetGatewayName = "daemonset-gateway"
	daemonSetGatewayPort = 8080
)

func TestDaemonSetWorkload(tt *testing.T) {
	t := New(tt)
	gatewayManifest := manifest("daemonset", "gateway.yaml")
	daemonSetManifest := manifest("daemonset", "daemonset.yaml")
	deploymentManifest := manifest("daemonset", "deployment.yaml")
	name := daemonSetGatewayName

	// Start with the default Deployment workload.
	t.Apply(gatewayManifest)
	assertDeploymentReady(t, base.Namespace, name)
	assertDaemonSetAbsent(t, base.Namespace, name)
	assertServiceReadyForTraffic(t)
	assertPDBConfigured(t, base.Namespace, name)
	assertGatewayRoutesTraffic(t)

	// Switch to DaemonSet and verify generated resources still serve traffic.
	applyStaticManifest(t, daemonSetManifest)
	assertDaemonSetReady(t, base.Namespace, name)
	assertDeploymentAbsent(t, base.Namespace, name)
	assertServiceReadyForTraffic(t)
	assertDaemonSetOverlayApplied(t, base.Namespace, name)
	assertPDBConfigured(t, base.Namespace, name)
	assertGatewayRoutesTraffic(t)

	// Delete the managed DaemonSet to verify child events reconcile the Gateway.
	deletedUID := deleteDaemonSet(t, base.Namespace, name)
	assertDaemonSetRecreatedReady(t, base.Namespace, name, deletedUID)
	assertServiceReadyForTraffic(t)
	assertGatewayRoutesTraffic(t)

	// Switch back to Deployment and verify the stale DaemonSet is pruned.
	applyStaticManifest(t, deploymentManifest)
	assertDeploymentReady(t, base.Namespace, name)
	assertDaemonSetAbsent(t, base.Namespace, name)
	assertServiceReadyForTraffic(t)
	assertPDBConfigured(t, base.Namespace, name)
	assertGatewayRoutesTraffic(t)
}

func assertDaemonSetReady(t base.Test, namespace, name string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		daemonSet, err := getDaemonSet(t, namespace, name)
		if err != nil {
			return err
		}

		return daemonSetReadyError(daemonSet, namespace, name)
	})
}

func assertDaemonSetRecreatedReady(t base.Test, namespace, name string, deletedUID types.UID) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		daemonSet, err := getDaemonSet(t, namespace, name)
		if err != nil {
			return err
		}
		if daemonSet.UID == deletedUID {
			return fmt.Errorf("DaemonSet %s/%s still has deleted UID %s", namespace, name, deletedUID)
		}
		if daemonSet.DeletionTimestamp != nil {
			return fmt.Errorf("DaemonSet %s/%s is still deleting", namespace, name)
		}

		return daemonSetReadyError(daemonSet, namespace, name)
	})
}

func daemonSetReadyError(daemonSet *appsv1.DaemonSet, namespace, name string) error {
	if daemonSet.Status.ObservedGeneration < daemonSet.Generation {
		return fmt.Errorf(
			"DaemonSet %s/%s observedGeneration=%d, want at least %d",
			namespace,
			name,
			daemonSet.Status.ObservedGeneration,
			daemonSet.Generation,
		)
	}
	if daemonSet.Status.DesiredNumberScheduled == 0 {
		return fmt.Errorf("DaemonSet %s/%s has no desired pods", namespace, name)
	}
	if daemonSet.Status.NumberReady != daemonSet.Status.DesiredNumberScheduled {
		return fmt.Errorf(
			"DaemonSet %s/%s ready=%d, want desired=%d",
			namespace,
			name,
			daemonSet.Status.NumberReady,
			daemonSet.Status.DesiredNumberScheduled,
		)
	}

	return nil
}

func assertDeploymentReady(t base.Test, namespace, name string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		deployment, err := getDeployment(t, namespace, name)
		if err != nil {
			return err
		}
		if deployment.Status.ObservedGeneration < deployment.Generation {
			return fmt.Errorf(
				"Deployment %s/%s observedGeneration=%d, want at least %d",
				namespace,
				name,
				deployment.Status.ObservedGeneration,
				deployment.Generation,
			)
		}
		desiredReplicas := int32(1)
		if deployment.Spec.Replicas != nil {
			desiredReplicas = *deployment.Spec.Replicas
		}
		if deployment.Status.ReadyReplicas != desiredReplicas {
			return fmt.Errorf(
				"Deployment %s/%s ready=%d, want desired=%d",
				namespace,
				name,
				deployment.Status.ReadyReplicas,
				desiredReplicas,
			)
		}

		return nil
	})
}

func assertDeploymentAbsent(t base.Test, namespace, name string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		_, err := getDeployment(t, namespace, name)
		if apierrors.IsNotFound(err) {
			return nil
		}
		if err != nil {
			return err
		}

		return fmt.Errorf("Deployment %s/%s still exists", namespace, name)
	})
}

func assertDaemonSetAbsent(t base.Test, namespace, name string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		_, err := getDaemonSet(t, namespace, name)
		if apierrors.IsNotFound(err) {
			return nil
		}
		if err != nil {
			return err
		}

		return fmt.Errorf("DaemonSet %s/%s still exists", namespace, name)
	})
}

func assertDaemonSetOverlayApplied(t base.Test, namespace, name string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		daemonSet, err := getDaemonSet(t, namespace, name)
		if err != nil {
			return err
		}
		if daemonSet.Labels["daemonset-e2e"] != "from-overlay" {
			return fmt.Errorf("DaemonSet %s/%s missing overlay label", namespace, name)
		}
		if daemonSet.Annotations["daemonset-e2e"] != "from-overlay" {
			return fmt.Errorf("DaemonSet %s/%s missing overlay annotation", namespace, name)
		}
		for _, toleration := range daemonSet.Spec.Template.Spec.Tolerations {
			if toleration.Operator == "Exists" {
				return nil
			}
		}

		return fmt.Errorf("DaemonSet %s/%s missing overlay toleration", namespace, name)
	})
}

func assertServiceSelectsReadyPods(t base.Test) {
	t.Helper()
	namespace := base.Namespace
	name := daemonSetGatewayName
	retry.UntilSuccessOrFail(t, func() error {
		service, err := getService(t, namespace, name)
		if err != nil {
			return err
		}
		if len(service.Spec.Selector) == 0 {
			return fmt.Errorf("Service %s/%s has no selector", namespace, name)
		}

		selector := labels.SelectorFromSet(service.Spec.Selector).String()
		pods, err := t.TestInstallation.ClusterContext.Client.Kube().CoreV1().Pods(namespace).List(
			t.Ctx,
			metav1.ListOptions{LabelSelector: selector},
		)
		if err != nil {
			return err
		}

		readyPods := 0
		for _, pod := range pods.Items {
			if pod.DeletionTimestamp != nil {
				continue
			}
			if isPodReady(&pod) {
				readyPods++
			}
		}
		if readyPods == 0 {
			return fmt.Errorf("Service %s/%s selects no ready pods", namespace, name)
		}

		return nil
	})
}

func assertServiceReadyForTraffic(t base.Test) {
	t.Helper()
	assertServiceSelectsReadyPods(t)
	assertServiceHasReadyEndpoints(t)
}

func assertServiceHasReadyEndpoints(t base.Test) {
	t.Helper()
	namespace := base.Namespace
	name := daemonSetGatewayName
	retry.UntilSuccessOrFail(t, func() error {
		selector := labels.Set{discoveryv1.LabelServiceName: name}.String()
		endpointSlices, err := t.TestInstallation.ClusterContext.Client.Kube().DiscoveryV1().EndpointSlices(namespace).List(
			t.Ctx,
			metav1.ListOptions{LabelSelector: selector},
		)
		if err != nil {
			return err
		}

		readyEndpoints := 0
		for _, endpointSlice := range endpointSlices.Items {
			if !endpointSliceHasPort(endpointSlice, daemonSetGatewayPort) {
				continue
			}
			for _, endpoint := range endpointSlice.Endpoints {
				if endpointReady(endpoint) {
					readyEndpoints++
				}
			}
		}
		if readyEndpoints == 0 {
			return fmt.Errorf(
				"Service %s/%s has no ready endpoints for port %d",
				namespace,
				name,
				daemonSetGatewayPort,
			)
		}

		return nil
	})
}

func assertPDBConfigured(t base.Test, namespace, name string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		pdb, err := getPDB(t, namespace, name)
		if err != nil {
			return err
		}
		if pdb.Labels["daemonset-pdb-e2e"] != "from-overlay" {
			return fmt.Errorf("PodDisruptionBudget %s/%s missing overlay label", namespace, name)
		}
		if pdb.Spec.MinAvailable == nil || pdb.Spec.MinAvailable.IntVal != 1 {
			return fmt.Errorf("PodDisruptionBudget %s/%s minAvailable is not 1", namespace, name)
		}
		if pdb.Spec.Selector == nil {
			return fmt.Errorf("PodDisruptionBudget %s/%s has no selector", namespace, name)
		}
		if pdb.Spec.Selector.MatchLabels["gateway.networking.k8s.io/gateway-name"] != name {
			return fmt.Errorf("PodDisruptionBudget %s/%s does not select Gateway pods", namespace, name)
		}

		return nil
	})
}

func assertGatewayRoutesTraffic(t base.Test) {
	t.Helper()
	name := daemonSetGatewayName
	t.GatewayReady(name, base.Namespace)
	t.HTTPRouteAccepted("daemonset-gateway-route", base.Namespace)

	gatewayName := types.NamespacedName{Name: name, Namespace: base.Namespace}
	gateway := base.Gateway{
		NamespacedName: gatewayName,
		Address:        base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, gatewayName),
	}
	gateway.Send(
		t,
		base.ExpectOK(),
		curl.WithPort(gateway.PortForRemote(daemonSetGatewayPort)),
		curl.WithHostHeader("daemonset.example.com"),
		curl.WithPath("/status/200"),
	)
}

func isPodReady(pod *corev1.Pod) bool {
	for _, condition := range pod.Status.Conditions {
		if condition.Type == corev1.PodReady && condition.Status == corev1.ConditionTrue {
			return true
		}
	}

	return false
}

func endpointReady(endpoint discoveryv1.Endpoint) bool {
	return endpoint.Conditions.Ready == nil || *endpoint.Conditions.Ready
}

func endpointSliceHasPort(endpointSlice discoveryv1.EndpointSlice, port int32) bool {
	for _, endpointPort := range endpointSlice.Ports {
		if endpointPort.Port != nil && *endpointPort.Port == port {
			return true
		}
	}

	return false
}

func applyStaticManifest(t base.Test, manifest string) {
	t.Helper()
	err := t.TestInstallation.ClusterContext.Client.ApplyYAMLFiles("", manifest)
	assert.NoError(t, err)
}

func deleteDaemonSet(t base.Test, namespace, name string) types.UID {
	t.Helper()
	daemonSet, err := getDaemonSet(t, namespace, name)
	assert.NoError(t, err)
	err = t.TestInstallation.ClusterContext.Client.Kube().AppsV1().DaemonSets(namespace).Delete(
		t.Ctx,
		name,
		metav1.DeleteOptions{},
	)
	assert.NoError(t, err)

	return daemonSet.UID
}

func getDaemonSet(t base.Test, namespace, name string) (*appsv1.DaemonSet, error) {
	t.Helper()

	return t.TestInstallation.ClusterContext.Client.Kube().AppsV1().DaemonSets(namespace).Get(
		t.Ctx,
		name,
		metav1.GetOptions{},
	)
}

func getDeployment(t base.Test, namespace, name string) (*appsv1.Deployment, error) {
	t.Helper()

	return t.TestInstallation.ClusterContext.Client.Kube().AppsV1().Deployments(namespace).Get(
		t.Ctx,
		name,
		metav1.GetOptions{},
	)
}

func getService(t base.Test, namespace, name string) (*corev1.Service, error) {
	t.Helper()

	return t.TestInstallation.ClusterContext.Client.Kube().CoreV1().Services(namespace).Get(
		t.Ctx,
		name,
		metav1.GetOptions{},
	)
}

func getPDB(t base.Test, namespace, name string) (*policyv1.PodDisruptionBudget, error) {
	t.Helper()
	kubeClient := t.TestInstallation.ClusterContext.Client.Kube()
	pdbClient := kubeClient.PolicyV1().PodDisruptionBudgets(namespace)

	return pdbClient.Get(
		t.Ctx,
		name,
		metav1.GetOptions{},
	)
}
