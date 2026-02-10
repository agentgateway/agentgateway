//go:build e2e

package assertions

import (
	"context"
	"fmt"
	"time"

	"github.com/onsi/gomega"
	"github.com/onsi/gomega/gstruct"
	"github.com/onsi/gomega/types"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
	"github.com/kgateway-dev/kgateway/v2/test/helpers"
)

// EventuallyPodReady asserts that all containers in the pod are reporting a ready status.
func (p *Provider) EventuallyPodReady(
	ctx context.Context,
	podNamespace string,
	podName string,
	timeout ...time.Duration,
) {
	p.t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	p.Gomega.Eventually(func(g gomega.Gomega) {
		pod, err := p.clusterContext.Clientset.CoreV1().Pods(podNamespace).Get(ctx, podName, metav1.GetOptions{})
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to get pod")
		for _, container := range pod.Status.ContainerStatuses {
			g.Expect(container.Ready).To(gomega.BeTrue(), "Container should be ready")
			return
		}
		g.Expect(fmt.Sprintf("Waiting for pod %s in namespace %s to be ready", podName, podNamespace)).To(gomega.BeTrue())
	}).
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		Should(gomega.Succeed(), fmt.Sprintf("Pod %s in namespace %s should be ready", podName, podNamespace))
}

// EventuallyPodsRunning asserts that eventually all pods matching the given ListOptions are running and ready.
func (p *Provider) EventuallyPodsRunning(
	ctx context.Context,
	podNamespace string,
	listOpt metav1.ListOptions,
	timeout ...time.Duration,
) {
	p.t.Helper()
	p.EventuallyPodsMatches(ctx, podNamespace, listOpt, matchers.PodMatches(matchers.ExpectedPod{Status: corev1.PodRunning, Ready: true}), timeout...)
}

// EventuallyPodsMatches asserts that the pod(s) in the given namespace matches the provided matcher
func (p *Provider) EventuallyPodsMatches(
	ctx context.Context,
	podNamespace string,
	listOpt metav1.ListOptions,
	matcher types.GomegaMatcher,
	timeout ...time.Duration,
) {
	p.t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	p.Gomega.Eventually(func(g gomega.Gomega) {
		pods, err := p.clusterContext.Clientset.CoreV1().Pods(podNamespace).List(ctx, listOpt)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to list pods")
		g.Expect(pods.Items).NotTo(gomega.BeEmpty(), "No pods found")
		for _, pod := range pods.Items {
			g.Expect(pod).To(matcher)
		}
	}).
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		Should(gomega.Succeed(), fmt.Sprintf("Failed to match pod in namespace %s, with conditions %v", podNamespace, listOpt))
}

// EventuallyPodsNotExist asserts that eventually no pods matching the given selector exist on the cluster.
func (p *Provider) EventuallyPodsNotExist(
	ctx context.Context,
	podNamespace string,
	listOpt metav1.ListOptions,
	timeout ...time.Duration,
) {
	p.t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	p.Gomega.Eventually(func(g gomega.Gomega) {
		pods, err := p.clusterContext.Clientset.CoreV1().Pods(podNamespace).List(ctx, listOpt)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to list pods")
		g.Expect(pods.Items).To(gomega.BeEmpty(), "No pods should be found")
	}).
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		Should(gomega.Succeed(), fmt.Sprintf("pods matching %v in namespace %s should not be found in cluster",
			listOpt, podNamespace))
}

// EventuallyPodContainerContainsEnvVar asserts that eventually all pods matching the given pod namespace and selector
// have a container with the given container name and the given env var.
func (p *Provider) EventuallyPodContainerContainsEnvVar(
	ctx context.Context,
	podNamespace string,
	podListOpt metav1.ListOptions,
	containerName string,
	envVar corev1.EnvVar,
	timeout ...time.Duration,
) {
	p.t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	p.Gomega.Eventually(func(g gomega.Gomega) {
		pods, err := p.clusterContext.Clientset.CoreV1().Pods(podNamespace).List(ctx, podListOpt)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to list pods")
		g.Expect(pods.Items).NotTo(gomega.BeEmpty(), "No pods found")
		for _, pod := range pods.Items {
			g.Expect(pod.Spec.Containers).To(gomega.ContainElement(
				gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
					"Name": gomega.Equal(containerName),
					"Env": gomega.ContainElement(
						gomega.Equal(envVar),
					),
				}),
			))
		}
	}).
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		Should(gomega.Succeed(), fmt.Sprintf("Failed to match pod in namespace %s", podNamespace))
}

// EventuallyPodContainerDoesNotContainEnvVar asserts that eventually no pods matching the given pod namespace and selector
// have a container with the given name and env var with the given name.
func (p *Provider) EventuallyPodContainerDoesNotContainEnvVar(
	ctx context.Context,
	podNamespace string,
	podListOpt metav1.ListOptions,
	containerName string,
	envVarName string,
	timeout ...time.Duration,
) {
	p.t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	p.Gomega.Eventually(func(g gomega.Gomega) {
		pods, err := p.clusterContext.Clientset.CoreV1().Pods(podNamespace).List(ctx, podListOpt)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "Failed to list pods")
		g.Expect(pods.Items).NotTo(gomega.BeEmpty(), "No pods found")
		for _, pod := range pods.Items {
			g.Expect(pod.Spec.Containers).NotTo(gomega.ContainElement(
				gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
					"Name": gomega.Equal(containerName),
					"Env": gomega.ContainElement(
						gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
							"Name": gomega.Equal(envVarName),
						}),
					),
				}),
			))
		}
	}).
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		Should(gomega.Succeed(), fmt.Sprintf("Failed to match pod in namespace %s", podNamespace))
}
