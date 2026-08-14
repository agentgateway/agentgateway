//go:build e2e

package e2e_test

import (
	"fmt"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/onsi/gomega"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

var (
	modelCatalogSetupManifest   = manifest("modelcatalog", "setup.yaml")
	modelCatalogAltManifest     = manifest("modelcatalog", "alt-catalog.yaml")
	modelCatalogUpdatedManifest = manifest("modelcatalog", "updated-catalog.yaml")
)

const (
	modelCatalogGatewayName    = "gw"
	modelCatalogAltGatewayName = "gw-alt"
	modelCatalogNamespace      = "default"

	// sentinel rate is 1000000/million tokens (1 cost unit/token); any real catalog rate yields << 1
	minSentinelCost = 1.0
)

// costTotalRe extracts agw.ai.usage.cost.total, tolerant of logfmt, quoted, and JSON renderings.
var costTotalRe = regexp.MustCompile(`agw\.ai\.usage\.cost\.total[^0-9-]*([0-9]+(?:\.[0-9]+)?)`)

// maxLoggedCost returns the highest agw.ai.usage.cost.total value found in logs, or -1 if none is present.
func maxLoggedCost(logs string) float64 {
	maxCost := -1.0
	for line := range strings.SplitSeq(logs, "\n") {
		if m := costTotalRe.FindStringSubmatch(line); m != nil {
			if cost, err := strconv.ParseFloat(m[1], 64); err == nil && cost > maxCost {
				maxCost = cost
			}
		}
	}
	return maxCost
}

// hasLoggedCostBelow reports whether logs contain an agw.ai.usage.cost.total value strictly below ceiling.
func hasLoggedCostBelow(logs string, ceiling float64) bool {
	for line := range strings.SplitSeq(logs, "\n") {
		if m := costTotalRe.FindStringSubmatch(line); m != nil {
			if cost, err := strconv.ParseFloat(m[1], 64); err == nil && cost < ceiling {
				return true
			}
		}
	}
	return false
}

func TestModelCatalogCost(tt *testing.T) {
	t := New(tt, base.WithMinGwApiVersion(base.GwApiRequireRouteNames))

	t.Apply(modelCatalogSetupManifest)
	t.GatewayReady(modelCatalogGatewayName, modelCatalogNamespace)

	t.Run("SentinelRate", func(t base.Test) {
		gwName := types.NamespacedName{Name: modelCatalogGatewayName, Namespace: modelCatalogNamespace}
		gw := base.Gateway{
			NamespacedName: gwName,
			// Resolve explicitly so the test works under both port-forward and LoadBalancer modes.
			Address: base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, gwName),
		}
		gw.Send(
			t,
			base.ExpectBody(gomega.ContainSubstring("The name of this project is agentgateway")),
			curl.WithPath("/v1/chat/completions"),
			curl.WithPostBody(`{"messages": [{"role": "user", "content": "What is the name of this project?"}]}`),
			curl.WithHeader("Content-Type", "application/json"),
		)
		gomega.NewWithT(t).Eventually(func() error {
			logs, err := gatewayAccessLogs(t, modelCatalogGatewayName)
			if err != nil {
				return err
			}
			maxCost := maxLoggedCost(logs)
			if maxCost < 0 {
				return fmt.Errorf("no agw.ai.usage.cost.total in gateway logs (catalog ConfigMap not loaded?)")
			}
			if maxCost < minSentinelCost {
				return fmt.Errorf("logged cost %v < expected floor %v (catalog rate not applied?)", maxCost, minSentinelCost)
			}
			return nil
		}).WithTimeout(30 * time.Second).WithPolling(2 * time.Second).Should(gomega.Succeed())
	})

	t.Run("ConfigMapUpdatePropagatesWithoutRestart", func(t base.Test) {
		gwName := types.NamespacedName{Name: modelCatalogGatewayName, Namespace: modelCatalogNamespace}
		gw := base.Gateway{
			NamespacedName: gwName,
			Address:        base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, gwName),
		}

		g := gomega.NewWithT(t)

		podsBefore, err := gatewayPodUIDs(t, modelCatalogGatewayName)
		g.Expect(err).NotTo(gomega.HaveOccurred())

		t.Apply(modelCatalogUpdatedManifest)

		podClient := t.TestInstallation.ClusterContext.Client.Kube().CoreV1().Pods(modelCatalogNamespace)
		pods, err := podClient.List(t.Ctx, metav1.ListOptions{
			LabelSelector: "gateway.networking.k8s.io/gateway-name=" + modelCatalogGatewayName,
		})
		g.Expect(err).NotTo(gomega.HaveOccurred())
		g.Expect(pods.Items).To(gomega.HaveLen(1), "expected a single gateway pod")
		pod := pods.Items[0]
		metav1.SetMetaDataAnnotation(&pod.ObjectMeta, "throwawaytoupdateconfigmapdata", "true")
		_, err = podClient.Update(t.Ctx, &pod, metav1.UpdateOptions{})
		g.Expect(err).NotTo(gomega.HaveOccurred())

		g.Eventually(func() error {
			gw.Send(
				t,
				base.ExpectBody(gomega.ContainSubstring("The name of this project is agentgateway")),
				curl.WithPath("/v1/chat/completions"),
				curl.WithPostBody(`{"messages": [{"role": "user", "content": "What is the name of this project?"}]}`),
				curl.WithHeader("Content-Type", "application/json"),
			)
			logs, err := gatewayAccessLogs(t, modelCatalogGatewayName)
			if err != nil {
				return err
			}
			if !hasLoggedCostBelow(logs, minSentinelCost) {
				return fmt.Errorf("no agw.ai.usage.cost.total below ceiling %v in gateway logs (ConfigMap update not yet reflected by running pod)", minSentinelCost)
			}
			return nil
		}).WithTimeout(10 * time.Second).WithPolling(1 * time.Second).Should(gomega.Succeed())

		podsAfter, err := gatewayPodUIDs(t, modelCatalogGatewayName)
		g.Expect(err).NotTo(gomega.HaveOccurred())
		g.Expect(podsAfter).To(gomega.Equal(podsBefore),
			"gateway pod must not have restarted for the ConfigMap update to take effect")
	})

	t.Run("AlternativeCatalog", func(t base.Test) {
		t.Apply(modelCatalogAltManifest)
		t.GatewayReady(modelCatalogAltGatewayName, modelCatalogNamespace)

		gwName := types.NamespacedName{Name: modelCatalogAltGatewayName, Namespace: modelCatalogNamespace}
		gw := base.Gateway{
			NamespacedName: gwName,
			Address:        base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, gwName),
		}
		gw.Send(
			t,
			base.ExpectBody(gomega.ContainSubstring("The name of this project is agentgateway")),
			curl.WithPath("/v1/chat/completions"),
			curl.WithPostBody(`{"messages": [{"role": "user", "content": "What is the name of this project?"}]}`),
			curl.WithHeader("Content-Type", "application/json"),
		)
		gomega.NewWithT(t).Eventually(func() error {
			logs, err := gatewayAccessLogs(t, modelCatalogAltGatewayName)
			if err != nil {
				return err
			}
			maxCost := maxLoggedCost(logs)
			if maxCost < 0 {
				return fmt.Errorf("no agw.ai.usage.cost.total in gateway logs (catalog ConfigMap not loaded?)")
			}
			if maxCost >= minSentinelCost {
				return fmt.Errorf("logged cost %v >= ceiling %v (sentinel rate applied instead of alt catalog?)", maxCost, minSentinelCost)
			}
			return nil
		}).WithTimeout(30 * time.Second).WithPolling(2 * time.Second).Should(gomega.Succeed())
	})
}

// gatewayPodUIDs returns the sorted UIDs of the running gateway pods, so callers
// can assert that a ConfigMap update took effect without a pod restart/rollout.
func gatewayPodUIDs(t base.Test, gatewayName string) ([]string, error) {
	cluster := t.TestInstallation.ClusterContext
	pods, err := cluster.Client.Kube().CoreV1().Pods(modelCatalogNamespace).List(t.Ctx, metav1.ListOptions{
		LabelSelector: "gateway.networking.k8s.io/gateway-name=" + gatewayName,
	})
	if err != nil {
		return nil, err
	}
	uids := make([]string, 0, len(pods.Items))
	for _, pod := range pods.Items {
		uids = append(uids, string(pod.UID))
	}
	slices.Sort(uids)
	return uids, nil
}

func gatewayAccessLogs(t base.Test, gatewayName string) (string, error) {
	cluster := t.TestInstallation.ClusterContext
	pods, err := cluster.Client.Kube().CoreV1().Pods(modelCatalogNamespace).List(t.Ctx, metav1.ListOptions{
		LabelSelector: "gateway.networking.k8s.io/gateway-name=" + gatewayName,
	})
	if err != nil {
		return "", err
	}
	if len(pods.Items) == 0 {
		return "", fmt.Errorf("no gateway pods found for %s/%s", modelCatalogNamespace, gatewayName)
	}
	var sb strings.Builder
	for _, pod := range pods.Items {
		logs, err := cluster.Client.PodLogs(t.Ctx, pod.Name, modelCatalogNamespace, "agentgateway", false)
		if err != nil {
			return "", fmt.Errorf("failed to read logs for pod %s: %w", pod.Name, err)
		}
		sb.WriteString(logs)
		sb.WriteString("\n")
	}
	return sb.String(), nil
}
