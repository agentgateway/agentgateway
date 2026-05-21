//go:build e2e

package e2e_test

import (
	"fmt"
	"io"
	"math/rand"
	"strings"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
)

const (
	collectorLogTimeout = 20 * time.Second
	collectorLogPoll    = 500 * time.Millisecond
)

func TestOTel(tt *testing.T) {
	t := New(tt)
	t.ApplyPersistent(otelManifest("setup.yaml"))

	t.Run("Tracing", func(t base.Test) {
		testOTelTracing(t)
	})
	t.Run("AccessLog", func(t base.Test) {
		testOTelAccessLog(t)
	})
}

func testOTelTracing(t base.Test) {
	t.Apply(otelManifest("tracing.yaml"))

	assertions.EventuallyAgwPolicyCondition(t, "agw", base.Namespace, "Accepted", metav1.ConditionTrue)

	headerValue := fmt.Sprintf("%v", rand.Intn(10000)) //nolint:gosec // G404: Using math/rand for test trace identification

	retry.UntilSuccessOrFail(t, func() error {
		t.Send("www.example.com/status/200", base.ExpectOK(), curl.WithHeader("x-header-tag", headerValue))

		logs, err := getCollectorLogs(t)
		if err != nil {
			return fmt.Errorf("failed to get collector pod logs: %w", err)
		}

		mustContain := []string{
			`-> http.method: Str(GET)`,
			`-> deployment.environment.name: Str(production)`,
			`-> service.version: Str(test)`,
			`-> custom: Str(literal)`,
			fmt.Sprintf("-> request: Str(%s)", headerValue),
		}

		var missing []string
		for _, line := range mustContain {
			if !strings.Contains(logs, line) {
				missing = append(missing, line)
			}
		}
		if len(missing) > 0 {
			return fmt.Errorf("missing required trace lines: %v", missing)
		}

		hasHTTPURL := strings.Contains(logs, `-> url.scheme: Str(http)`) &&
			strings.Contains(logs, `-> http.host: Str(www.example.com)`) &&
			strings.Contains(logs, `-> http.path: Str(/status/200)`)
		if !hasHTTPURL {
			return fmt.Errorf("missing expected URL/host/path attributes in traces")
		}

		if !strings.Contains(logs, `-> http.status: Int(200)`) {
			return fmt.Errorf("missing expected HTTP status attribute in traces")
		}
		return nil
	}, retry.Timeout(collectorLogTimeout), retry.Delay(collectorLogPoll), retry.Message("should find traces in collector pod logs"))
}

func testOTelAccessLog(t base.Test) {
	t.Apply(otelManifest("accesslog-otlp.yaml"))

	assertions.EventuallyAgwPolicyCondition(t, "agw-accesslog", base.Namespace, "Accepted", metav1.ConditionTrue)

	retry.UntilSuccessOrFail(t, func() error {
		t.Send("www.example.com/status/200", base.ExpectOK())

		logs, err := getCollectorLogs(t)
		if err != nil {
			return fmt.Errorf("failed to get collector pod logs: %w", err)
		}

		mustContain := []string{
			`ScopeLogs`,
			`LogRecord #0`,
			`-> http.method: Str(GET)`,
			`-> http.path: Str(/status/200)`,
			`-> http.status: Int(200)`,
		}

		var missing []string
		for _, line := range mustContain {
			if !strings.Contains(logs, line) {
				missing = append(missing, line)
			}
		}
		if len(missing) > 0 {
			return fmt.Errorf("missing required access log lines in collector output: %v", missing)
		}
		return nil
	}, retry.Timeout(collectorLogTimeout), retry.Delay(collectorLogPoll), retry.Message("should find access logs in collector pod logs"))
}

func otelManifest(name string) string {
	return manifest("otel", name)
}

func getCollectorPod(t base.Test) (string, error) {
	pods := &corev1.PodList{}
	err := t.TestInstallation.ClusterContext.ControllerClient.List(
		t.Ctx,
		pods,
		client.InNamespace(base.Namespace),
		client.MatchingLabels{"app.kubernetes.io/name": "opentelemetry-collector"},
	)
	if err != nil {
		return "", err
	}
	if len(pods.Items) == 0 {
		return "", fmt.Errorf("no collector pods found")
	}

	var newest *corev1.Pod
	for i := range pods.Items {
		pod := &pods.Items[i]
		if pod.DeletionTimestamp != nil || pod.Status.Phase != corev1.PodRunning || !podReady(pod) {
			continue
		}
		if newest == nil || pod.CreationTimestamp.After(newest.CreationTimestamp.Time) {
			newest = pod
		}
	}
	if newest == nil {
		return "", fmt.Errorf("no running collector pods found")
	}

	return newest.Name, nil
}

func getCollectorLogs(t base.Test) (string, error) {
	pod, err := getCollectorPod(t)
	if err != nil {
		return "", err
	}
	stream, err := t.TestInstallation.ClusterContext.Client.Kube().CoreV1().
		Pods(base.Namespace).
		GetLogs(pod, &corev1.PodLogOptions{}).
		Stream(t.Ctx)
	if err != nil {
		return "", err
	}
	defer stream.Close()

	logs, err := io.ReadAll(stream)
	if err != nil {
		return "", err
	}
	return string(logs), nil
}

func podReady(pod *corev1.Pod) bool {
	for _, condition := range pod.Status.Conditions {
		if condition.Type == corev1.PodReady {
			return condition.Status == corev1.ConditionTrue
		}
	}
	return false
}
