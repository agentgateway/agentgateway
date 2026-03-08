//go:build e2e

package common

import (
	"context"
	"fmt"
	"sort"
	"strings"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/portforward"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
)

func StartGatewayPodForwards(
	ctx context.Context,
	installation *e2e.TestInstallation,
	gateway types.NamespacedName,
	count int,
) ([]string, []string, func(), error) {
	service, err := installation.ClusterContext.Clientset.CoreV1().Services(gateway.Namespace).Get(ctx, gateway.Name, metav1.GetOptions{})
	if err != nil {
		return nil, nil, nil, fmt.Errorf("failed to get gateway service %s/%s: %w", gateway.Namespace, gateway.Name, err)
	}

	podRemotePort := gatewayPodPort(service)
	if podRemotePort <= 0 {
		return nil, nil, nil, fmt.Errorf("gateway service %s/%s must expose an HTTP pod target port", gateway.Namespace, gateway.Name)
	}

	podNames, err := kubeutils.GetReadyPodsForDeployment(
		ctx,
		installation.ClusterContext.Clientset,
		metav1.ObjectMeta{Name: gateway.Name, Namespace: gateway.Namespace},
	)
	if err != nil {
		return nil, nil, nil, fmt.Errorf("failed to find ready gateway pods for %s/%s: %w", gateway.Namespace, gateway.Name, err)
	}
	if len(podNames) < count {
		return nil, nil, nil, fmt.Errorf("expected at least %d ready gateway pods for %s/%s, got %d", count, gateway.Namespace, gateway.Name, len(podNames))
	}

	sort.Strings(podNames)
	podNames = append([]string(nil), podNames[:count]...)

	forwarders := make([]portforward.PortForwarder, 0, count)
	addresses := make([]string, 0, count)

	for _, podName := range podNames {
		forwarder, err := installation.Actions.Kubectl().StartPortForward(
			ctx,
			portforward.WithPod(podName, gateway.Namespace),
			portforward.WithRemotePort(podRemotePort),
		)
		if err != nil {
			for _, started := range forwarders {
				started.Close()
			}
			return nil, nil, nil, fmt.Errorf("failed to port-forward gateway pod %s/%s: %w", gateway.Namespace, podName, err)
		}
		forwarders = append(forwarders, forwarder)
		addresses = append(addresses, forwarder.Address())
	}

	cleanup := func() {
		for _, forwarder := range forwarders {
			forwarder.Close()
		}
	}

	return podNames, addresses, cleanup, nil
}

func gatewayPodPort(service *corev1.Service) int {
	for _, port := range service.Spec.Ports {
		if strings.EqualFold(port.Name, "http") || port.Port == 80 {
			if targetPort := port.TargetPort.IntValue(); targetPort > 0 {
				return targetPort
			}
			return int(port.Port)
		}
	}
	return 0
}
