//go:build e2e

package assertions

import (
	"context"
	"fmt"
	"time"

	"github.com/onsi/gomega/types"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
	"github.com/kgateway-dev/kgateway/v2/test/helpers"
)

// EventuallyReadyReplicas asserts that given a Deployment, eventually the number of pods matching the replicaMatcher
// are in the ready state and able to receive traffic.
func (p *Provider) EventuallyReadyReplicas(ctx context.Context, deploymentMeta metav1.ObjectMeta, replicaMatcher types.GomegaMatcher, timeout ...time.Duration) {
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	p.Gomega.Eventually(func(innerG Gomega) {
		// We intentionally rely only on Pods that have marked themselves as ready as a way of defining more explicit assertions
		pods, err := kubeutils.GetReadyPodsForDeployment(ctx, p.clusterContext.Clientset, deploymentMeta)
		innerG.Expect(err).NotTo(HaveOccurred(), "can get pods for deployment")
		innerG.Expect(len(pods)).To(replicaMatcher, "running pods matches expected count")
	}).
		WithContext(ctx).
		// It may take some time for pods to initialize and pull images from remote registries.
		// Therefore, we set a longer timeout, to account for latency that may exist.
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		Should(Succeed())
}

// EventuallyDeploymentNotExists asserts that eventually no deployments matching the given selector and namespace exist on the cluster.
func (p *Provider) EventuallyDeploymentNotExists(ctx context.Context,
	deploymentNamespace string,
	listOpt metav1.ListOptions,
	timeout ...time.Duration,
) {
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	p.Gomega.Eventually(func(g Gomega) {
		deployments, err := p.clusterContext.Clientset.AppsV1().Deployments(deploymentNamespace).List(ctx, listOpt)
		g.Expect(err).NotTo(HaveOccurred(), "Failed to list deployments")
		g.Expect(deployments.Items).To(BeEmpty(), "No deployments should be found")
	}).
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		Should(Succeed(), fmt.Sprintf("deployments matching %v in namespace %s should not be found in cluster",
			listOpt, deploymentNamespace))
}
