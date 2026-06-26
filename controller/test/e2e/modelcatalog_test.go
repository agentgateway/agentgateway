//go:build e2e

package e2e_test

import (
	"fmt"
	"regexp"
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
	modelCatalogSetupManifest = manifest("modelcatalog", "setup.yaml")
	modelCatalogAltManifest   = manifest("modelcatalog", "alt-catalog.yaml")
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
			logs, err := gatewayAccessLogs(t, modelCatalogNamespace, modelCatalogGatewayName)
			if err != nil {
				return err
			}
			maxCost := -1.0
			for line := range strings.SplitSeq(logs, "\n") {
				if m := costTotalRe.FindStringSubmatch(line); m != nil {
					if cost, err := strconv.ParseFloat(m[1], 64); err == nil && cost > maxCost {
						maxCost = cost
					}
				}
			}
			if maxCost < 0 {
				return fmt.Errorf("no agw.ai.usage.cost.total in gateway logs (catalog ConfigMap not loaded?)")
			}
			if maxCost < minSentinelCost {
				return fmt.Errorf("logged cost %v < expected floor %v (catalog rate not applied?)", maxCost, minSentinelCost)
			}
			return nil
		}).WithTimeout(30 * time.Second).WithPolling(2 * time.Second).Should(gomega.Succeed())
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
			logs, err := gatewayAccessLogs(t, modelCatalogNamespace, modelCatalogAltGatewayName)
			if err != nil {
				return err
			}
			maxCost := -1.0
			for line := range strings.SplitSeq(logs, "\n") {
				if m := costTotalRe.FindStringSubmatch(line); m != nil {
					if cost, err := strconv.ParseFloat(m[1], 64); err == nil && cost > maxCost {
						maxCost = cost
					}
				}
			}
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

func gatewayAccessLogs(t base.Test, ns, gatewayName string) (string, error) {
	cluster := t.TestInstallation.ClusterContext
	pods, err := cluster.Client.Kube().CoreV1().Pods(ns).List(t.Ctx, metav1.ListOptions{
		LabelSelector: "gateway.networking.k8s.io/gateway-name=" + gatewayName,
	})
	if err != nil {
		return "", err
	}
	if len(pods.Items) == 0 {
		return "", fmt.Errorf("no gateway pods found for %s/%s", ns, gatewayName)
	}
	var sb strings.Builder
	for _, pod := range pods.Items {
		logs, err := cluster.Client.PodLogs(t.Ctx, pod.Name, ns, "agentgateway", false)
		if err != nil {
			return "", fmt.Errorf("failed to read logs for pod %s: %w", pod.Name, err)
		}
		sb.WriteString(logs)
		sb.WriteString("\n")
	}
	return sb.String(), nil
}
