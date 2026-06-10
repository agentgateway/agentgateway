//go:build e2e

package e2e_test

import (
	"fmt"
	"strings"
	"testing"

	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
)

const (
	transparentEgressNamespace = "agw-odst"
	transparentEgressGateway   = "egress"
	transparentEgressRoute     = "original-dst-ok"
	transparentEgressPolicy    = "original-dst-frontend"
	transparentEgressTestbox   = "testbox"
)

func TestTransparentEgressOriginalDstNetworkAuthorization(tt *testing.T) {
	t := New(tt)

	t.Apply(manifest("transparent-egress", "original-dst.yaml"))
	t.GatewayReady(transparentEgressGateway, transparentEgressNamespace)
	t.HTTPRouteAccepted(transparentEgressRoute, transparentEgressNamespace)
	assertions.EventuallyAgwPolicyCondition(t, transparentEgressPolicy, transparentEgressNamespace, "Accepted", metav1.ConditionTrue)

	allowed := execTransparentEgressFetch(t, "http://93.184.216.34/")
	if !strings.Contains(allowed, "200 OK") {
		t.Fatalf("expected allowed original-dst request to return 200 OK, got:\n%s", allowed)
	}
	if !strings.Contains(allowed, "original-dst-ok") {
		t.Fatalf("expected allowed original-dst request body to identify the route, got:\n%s", allowed)
	}

	retry.UntilSuccessOrFail(t, func() error {
		podName, err := transparentEgressPodName(t)
		if err != nil {
			return err
		}
		stdout, stderr, err := t.TestInstallation.ClusterContext.Client.PodExecCommands(
			podName,
			transparentEgressNamespace,
			transparentEgressTestbox,
			[]string{"/usr/local/bin/testbox", "fetch", "http://1.1.1.1/"},
		)
		if err == nil {
			return fmt.Errorf("expected denied original-dst request to fail, got stdout=%q stderr=%q", stdout, stderr)
		}
		return nil
	})
}

func transparentEgressPodName(t base.Test) (string, error) {
	t.Helper()

	pods, err := t.TestInstallation.ClusterContext.Client.Kube().CoreV1().Pods(transparentEgressNamespace).List(
		t.Ctx,
		metav1.ListOptions{LabelSelector: "app.kubernetes.io/name=egress"},
	)
	if err != nil {
		return "", fmt.Errorf("failed to list transparent egress pods: %w", err)
	}
	for _, pod := range pods.Items {
		if pod.Status.Phase != corev1.PodRunning || !podReady(&pod) || pod.DeletionTimestamp != nil {
			continue
		}
		return pod.Name, nil
	}

	return "", fmt.Errorf("no ready transparent egress pod found")
}

func execTransparentEgressFetch(t base.Test, url string) string {
	t.Helper()

	var combined string
	retry.UntilSuccessOrFail(t, func() error {
		podName, err := transparentEgressPodName(t)
		if err != nil {
			return err
		}
		stdout, stderr, err := t.TestInstallation.ClusterContext.Client.PodExecCommands(
			podName,
			transparentEgressNamespace,
			transparentEgressTestbox,
			[]string{"/usr/local/bin/testbox", "fetch", url},
		)
		combined = stdout + stderr
		if err != nil {
			return fmt.Errorf("failed to fetch %q: %w\nstdout:\n%s\nstderr:\n%s", url, err, stdout, stderr)
		}
		return nil
	})

	return combined
}
